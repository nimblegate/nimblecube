//! DNA eval: nimblecube's own three operations as a DNA read matcher, at small scale.
//! Host/std, deterministic, synthetic genome (no data files).
//!   cargo run --release --example dna_eval
//!
//! Encoding: each base gets a random 4096-bit vector, position i inside a k-mer is
//! `permute(i)`, a k-mer is the XOR (bind) of its permuted bases, and a read is the
//! majority bundle of all its k-mers ("bag of k-mers", so it has no fixed alignment).
//! Base vectors are drawn so each partner base is the same vector XOR one mask M
//! (A=a, T=a^M, C=c, G=c^M), which makes the complement strand a single XOR.
//! "Canonical" stores each k-mer as the smaller of itself and its reverse complement,
//! so a read from either strand encodes the same.
//!
//! Measured 2026-09-13, 20,000-base random genome, 200 windows of 100 bases, k=8,
//! 200 reads per row (forward and canonical rows agree within a few bits except strand):
//!
//! ```text
//! complement strand, one XOR with the mask : distance 0 on all 200 (raw ~2022)
//! exact read                               : 100%   true 0
//! 10% substitutions                        : 100%   true 1476
//! 20% substitutions                        : 82.5%  true 1836  <- margin gone
//! shifted 40 bases                         : 100%   true 1255, neighbour window 1569
//! 10 insertions/deletions                  : 100%   true 1354
//! other strand, forward k-mers             : 1.5%   (chance)
//! other strand, canonical k-mers           : 100%   true 0
//! nearest wrong window                     : ~1950 = min of 199 draws of 2048 +- 32
//! ```
//!
//! Random genome, no repeats: real genomes repeat, which is what breaks a k-mer bag.

use nimblecube_core::hv::{Hv, WORDS};
use nimblecube_core::store::FixedStore;

const K: usize = 8; // k-mer length
const L: usize = 100; // read and window length, in bases (93 k-mers, odd: no bundle ties)
const WINDOWS: usize = 200; // a 20,000-base genome cut every L bases
const QUERIES: usize = 200;

fn xs(s: &mut u64) -> u64 {
    *s ^= *s << 13;
    *s ^= *s >> 7;
    *s ^= *s << 17;
    *s
}
fn rand_hv(s: &mut u64) -> Hv {
    let mut w = [0u64; WORDS];
    for x in w.iter_mut() {
        *x = xs(s);
    }
    Hv(w)
}
fn chance(s: &mut u64, per_mille: u64) -> bool {
    xs(s) % 1000 < per_mille
}

/// Bases are 0..4 as A=00, C=01, G=10, T=11, so the partner base is `b ^ 3`.
struct Enc<'a> {
    pb: &'a [[Hv; K]; 4], // pb[base][i] = permute(base vector, i)
    canonical: bool,
}

impl Enc<'_> {
    fn kmer(&self, b: &[u8]) -> Hv {
        let rc: [u8; K] = core::array::from_fn(|i| b[K - 1 - i] ^ 3);
        // lexicographic order on 2-bit codes = numeric order of the k-mer
        let src: &[u8] = if self.canonical && rc[..] < b[..K] { &rc } else { &b[..K] };
        src.iter()
            .enumerate()
            .fold(Hv::zero(), |acc, (i, &base)| acc.bind(&self.pb[base as usize][i]))
    }
    fn encode(&self, seq: &[u8]) -> Hv {
        let kmers: Vec<Hv> = seq.windows(K).map(|b| self.kmer(b)).collect();
        Hv::bundle(&kmers)
    }
}

#[derive(Clone, Copy)]
enum Cond {
    Sub(u64), // substitutions, per mille of bases
    Shift(usize),
    Indel(usize),
    RevComp,
}

fn make_read(genome: &[u8], w: usize, cond: Cond, s: &mut u64) -> Vec<u8> {
    let window = &genome[w * L..(w + 1) * L];
    match cond {
        Cond::Sub(pm) => window
            .iter()
            .map(|&b| if chance(s, pm) { b ^ (1 + (xs(s) % 3) as u8) } else { b })
            .collect(),
        Cond::Shift(n) => genome[w * L + n..w * L + n + L].to_vec(),
        Cond::Indel(n) => {
            let mut r = window.to_vec();
            for _ in 0..n {
                let p = (xs(s) as usize) % r.len();
                if xs(s) & 1 == 0 {
                    r.insert(p, (xs(s) & 3) as u8);
                } else {
                    r.remove(p);
                }
            }
            r
        }
        Cond::RevComp => window.iter().rev().map(|b| b ^ 3).collect(),
    }
}

/// (recall@1 in %, mean distance to the true window, mean distance to the nearest wrong one)
fn eval(
    store: &FixedStore<WINDOWS>,
    enc: &Enc,
    genome: &[u8],
    cond: Cond,
    seed: u64,
) -> (f64, f64, f64) {
    let mut s = seed;
    let (mut hits, mut dt, mut dw) = (0usize, 0u64, 0u64);
    for _ in 0..QUERIES {
        let w = (xs(&mut s) as usize) % WINDOWS;
        let hv = enc.encode(&make_read(genome, w, cond, &mut s));
        let (id, _) = store.nearest(&hv).unwrap();
        hits += (id as usize == w) as usize;
        dt += store.slots()[w].hamming(&hv) as u64;
        let slots = store.slots().iter().enumerate();
        dw += slots.filter(|(i, _)| *i != w).map(|(_, x)| x.hamming(&hv)).min().unwrap() as u64;
    }
    let q = QUERIES as f64;
    (100.0 * hits as f64 / q, dt as f64 / q, dw as f64 / q)
}

fn main() {
    let mut s = 0x9E37_79B9_7F4A_7C15u64;
    let (a, c, m) = (rand_hv(&mut s), rand_hv(&mut s), rand_hv(&mut s));
    let base = [a.clone(), c.clone(), c.bind(&m), a.bind(&m)]; // A, C, G, T
    let pb: [[Hv; K]; 4] = core::array::from_fn(|b| core::array::from_fn(|i| base[b].permute(i)));
    let fwd = Enc { pb: &pb, canonical: false };
    let can = Enc { pb: &pb, canonical: true };

    let genome: Vec<u8> = (0..(WINDOWS + 1) * L).map(|_| (xs(&mut s) & 3) as u8).collect();
    let mut store_f = FixedStore::<WINDOWS>::new();
    let mut store_c = FixedStore::<WINDOWS>::new();
    for w in 0..WINDOWS {
        let seq = &genome[w * L..(w + 1) * L];
        store_f.insert(fwd.encode(seq), w as u32).unwrap();
        store_c.insert(can.encode(seq), w as u32).unwrap();
    }
    let bases = WINDOWS * L;
    println!("genome {bases} bases, {WINDOWS} windows of {L}, k={K}, {QUERIES} reads per row\n");

    // Complement strand = one XOR: each k-mer moves by the same mask mk, and a majority
    // of an odd count commutes with XOR by a fixed mask.
    let mk = (0..K).fold(Hv::zero(), |acc, i| acc.bind(&m.permute(i)));
    let (mut worst, mut apart) = (0u32, 0u64);
    for w in 0..WINDOWS {
        let comp: Vec<u8> = genome[w * L..(w + 1) * L].iter().map(|b| b ^ 3).collect();
        let hv = fwd.encode(&comp);
        worst = worst.max(hv.hamming(&store_f.slots()[w].bind(&mk)));
        apart += hv.hamming(&store_f.slots()[w]) as u64;
    }
    let raw = apart as f64 / WINDOWS as f64;
    println!("complement strand: raw distance to its window {raw:.0} (looks unrelated);");
    println!("                   after one XOR with the mask: worst of {WINDOWS} = {worst}\n");

    let rows: [(&str, Cond); 11] = [
        ("exact read", Cond::Sub(0)),
        ("1% substitutions", Cond::Sub(10)),
        ("5% substitutions", Cond::Sub(50)),
        ("10% substitutions", Cond::Sub(100)),
        ("20% substitutions", Cond::Sub(200)),
        ("shifted 10 bases", Cond::Shift(10)),
        ("shifted 25 bases", Cond::Shift(25)),
        ("shifted 40 bases", Cond::Shift(40)),
        ("3 insertions/deletions", Cond::Indel(3)),
        ("10 insertions/deletions", Cond::Indel(10)),
        ("other strand (reverse complement)", Cond::RevComp),
    ];
    println!("{:34}  {:^22}  {:^22}", "", "forward k-mers", "canonical k-mers");
    let (r, t, w) = ("recall", "true", "wrong");
    println!("{:34}  {r:>7} {t:>6} {w:>7}  {r:>7} {t:>6} {w:>7}", "read");
    for (i, (name, cond)) in rows.iter().enumerate() {
        let seed = 0xD1CE_0000 + i as u64 * 7919;
        let (rf, tf, wf) = eval(&store_f, &fwd, &genome, *cond, seed);
        let (rc, tc, wc) = eval(&store_c, &can, &genome, *cond, seed);
        println!("{name:34}  {rf:6.1}% {tf:6.0} {wf:7.0}  {rc:6.1}% {tc:6.0} {wc:7.0}");
    }

    let other: Vec<u8> = (0..(WINDOWS + 1) * L).map(|_| (xs(&mut s) & 3) as u8).collect();
    let (mut sum, mut low) = (0u64, u32::MAX);
    for _ in 0..QUERIES {
        let p = (xs(&mut s) as usize) % (WINDOWS * L);
        let (_, d) = store_f.nearest(&fwd.encode(&other[p..p + L])).unwrap();
        sum += d as u64;
        low = low.min(d);
    }
    let mean = sum as f64 / QUERIES as f64;
    println!("\nunrelated reads (other genome): nearest window {mean:.0} on average, lowest {low}");
}
