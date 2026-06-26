//! IVF (cluster-and-probe) index eval: recall@1 + speedup of an HDC-native inverted-file
//! index vs the linear scan, on clustered vs uniform-random data. Host/std, deterministic.
//!   cargo run --release --example ivf_eval
//! Centroid = Hv::bundle (majority); assign/probe = Hv::hamming.

use nimblecube_core::hv::{Hv, DIM_BITS, WORDS};

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
fn noisy(base: &Hv, nbits: usize, s: &mut u64) -> Hv {
    let mut h = base.clone();
    for _ in 0..nbits {
        let b = (xs(s) as usize) % DIM_BITS;
        h.0[b / 64] ^= 1u64 << (b % 64);
    }
    h
}

/// linear-scan ground-truth nearest distance.
fn linear_dist(items: &[Hv], q: &Hv) -> u32 {
    let mut bd = u32::MAX;
    for h in items {
        let d = h.hamming(q);
        if d < bd {
            bd = d;
        }
    }
    bd
}

struct Ivf {
    centroids: Vec<Hv>,
    members: Vec<Vec<u32>>,
}

fn build_ivf(items: &[Hv], k: usize, t: usize) -> Ivf {
    let n = items.len();
    let mut centroids: Vec<Hv> = (0..k).map(|i| items[(i * n / k).min(n - 1)].clone()).collect();
    let mut members: Vec<Vec<u32>> = vec![Vec::new(); k];
    for _ in 0..t {
        for m in members.iter_mut() {
            m.clear();
        }
        for (i, h) in items.iter().enumerate() {
            let mut bc = 0usize;
            let mut bd = u32::MAX;
            for (c, cen) in centroids.iter().enumerate() {
                let d = cen.hamming(h);
                if d < bd {
                    bd = d;
                    bc = c;
                }
            }
            members[bc].push(i as u32);
        }
        for c in 0..k {
            if !members[c].is_empty() {
                let mems: Vec<Hv> = members[c].iter().map(|&id| items[id as usize].clone()).collect();
                centroids[c] = Hv::bundle(&mems);
            }
        }
    }
    Ivf { centroids, members }
}

/// IVF nearest distance + number of Hamming compares performed.
fn ivf_dist(ivf: &Ivf, items: &[Hv], q: &Hv, nprobe: usize) -> (u32, u64) {
    let k = ivf.centroids.len();
    let mut cd: Vec<(u32, usize)> = (0..k).map(|c| (ivf.centroids[c].hamming(q), c)).collect();
    let mut compares = k as u64;
    cd.sort_by_key(|x| x.0);
    let mut bd = u32::MAX;
    for p in 0..nprobe.min(k) {
        let c = cd[p].1;
        for &id in &ivf.members[c] {
            let d = items[id as usize].hamming(q);
            compares += 1;
            if d < bd {
                bd = d;
            }
        }
    }
    (bd, compares)
}

fn run(items: &[Hv], queries: &[Hv], label: &str) {
    println!("{}:", label);
    println!("  {:>4} {:>7} {:>9} {:>8} {:>13}", "k", "nprobe", "recall@1", "speedup", "avg_compares");
    let n = items.len() as f64;
    let truth: Vec<u32> = queries.iter().map(|q| linear_dist(items, q)).collect();
    for &k in &[110usize, 256] {
        let ivf = build_ivf(items, k, 5);
        for &nprobe in &[1usize, 4, 16, 64] {
            let mut hit = 0usize;
            let mut total_cmp = 0u64;
            for (qi, q) in queries.iter().enumerate() {
                let (bd, cmp) = ivf_dist(&ivf, items, q, nprobe);
                if bd == truth[qi] {
                    hit += 1;
                }
                total_cmp += cmp;
            }
            let recall = 100.0 * hit as f64 / queries.len() as f64;
            let avg = total_cmp as f64 / queries.len() as f64;
            println!("  {:>4} {:>7} {:>8.1}% {:>7.1}x {:>13.0}", k, nprobe, recall, n / avg, avg);
        }
    }
}

fn main() {
    let mut s: u64 = 0x000A_11CE_5EED;
    let n = 12000usize;
    let g = 200usize;
    let flip = 200usize;
    let q = 500usize;

    let bases: Vec<Hv> = (0..g).map(|_| rand_hv(&mut s)).collect();
    let mut items: Vec<Hv> = Vec::with_capacity(n);
    for i in 0..n {
        items.push(noisy(&bases[i % g], flip, &mut s));
    }
    let queries: Vec<Hv> = (0..q)
        .map(|_| {
            let b = (xs(&mut s) as usize) % g;
            noisy(&bases[b], flip, &mut s)
        })
        .collect();

    let rand_items: Vec<Hv> = (0..n).map(|_| rand_hv(&mut s)).collect();
    let rand_queries: Vec<Hv> = (0..q).map(|_| rand_hv(&mut s)).collect();

    println!("IVF eval  N={} G={} flip={} queries={}\n", n, g, flip, q);
    run(&items, &queries, "clustered data");
    println!();
    run(&rand_items, &rand_queries, "uniform-random baseline (no structure)");
}
