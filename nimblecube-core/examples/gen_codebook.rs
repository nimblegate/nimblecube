//! Generate the FeatureEncoder codebook: every hypervector the encoder can ever
//! emit, indexed by its quantized level tuple. The encoder is frozen once seed
//! and ranges are chosen, so all L^CH outputs are known at build time.
//!   cargo run --release --example gen_codebook -- <out.bin>

use nimblecube_core::encode::{quantize, FeatureEncoder};
use nimblecube_core::hv::WORDS;
use std::io::Write;

const CH: usize = 3;
const L: usize = 16;
const SEED: u64 = 7;
const RANGES: [(i32, i32); CH] = [(0, 4095), (0, 4095), (-4095, 4095)];

/// One feature value per level, found by scanning. Exact rather than derived,
/// so it cannot drift from `quantize`'s rounding.
fn reps(min: i32, max: i32) -> [i32; L] {
    let mut out = [i32::MIN; L];
    let mut found = 0;
    let mut v = min;
    while v <= max && found < L {
        let idx = quantize(v, min, max, L);
        if out[idx] == i32::MIN {
            out[idx] = v;
            found += 1;
        }
        v += 1;
    }
    assert_eq!(found, L, "range [{}, {}] does not cover all {} levels", min, max, L);
    out
}

fn main() {
    let path = std::env::args().nth(1).expect("usage: gen_codebook <out.bin>");
    let enc = FeatureEncoder::<CH, L>::new(SEED, RANGES);
    let r: Vec<[i32; L]> = RANGES.iter().map(|&(lo, hi)| reps(lo, hi)).collect();

    let mut out = Vec::with_capacity(L * L * L * WORDS * 8);
    for i in 0..L {
        for j in 0..L {
            for k in 0..L {
                let hv = enc.encode(&[r[0][i], r[1][j], r[2][k]]);
                for w in hv.0.iter() {
                    out.extend_from_slice(&w.to_le_bytes());
                }
            }
        }
    }
    assert_eq!(out.len(), L * L * L * WORDS * 8);
    std::fs::File::create(&path).unwrap().write_all(&out).unwrap();
    println!("wrote {} entries, {} bytes to {}", L * L * L, out.len(), path);
    println!("flat index = i*{} + j*{} + k", L * L, L);
}
