//! B1 micro-benchmark: scalar-vs-auto-vectorized throughput of the no_std core's
//! Hamming path (`Hv::hamming`, `FixedStore::nearest`). Deterministic, zero-dep.
//! Compare builds:
//!   cargo run --release --example hamming_bench
//!   RUSTFLAGS="-C target-feature=+popcnt" cargo run --release --example hamming_bench
//!   RUSTFLAGS="-C target-cpu=native"      cargo run --release --example hamming_bench
//! Env: BENCH_M (queries, 1000), BENCH_R (timed runs, 5), BENCH_SEED (1).
//! N (store capacity) is the compile-time const below.

use nimblecube_core::hv::{Hv, WORDS};
use nimblecube_core::store::FixedStore;
use std::hint::black_box;
use std::time::{Duration, Instant};

const N: usize = 1024;

fn envu(name: &str, d: usize) -> usize {
    std::env::var(name).ok().and_then(|s| s.parse().ok()).unwrap_or(d)
}

fn rand_hv(state: &mut u64) -> Hv {
    let mut w = [0u64; WORDS];
    for x in w.iter_mut() {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *x = *state;
    }
    Hv(w)
}

fn median(mut xs: Vec<Duration>) -> Duration {
    xs.sort();
    xs[xs.len() / 2]
}

fn features() -> &'static str {
    if cfg!(target_feature = "avx512f") {
        "avx512f"
    } else if cfg!(target_feature = "avx2") {
        "avx2(+popcnt)"
    } else if cfg!(target_feature = "popcnt") {
        "popcnt"
    } else if cfg!(target_feature = "neon") {
        "neon"
    } else {
        "baseline(no-popcnt)"
    }
}

fn main() {
    let m = envu("BENCH_M", 1000);
    let r = envu("BENCH_R", 5);
    let seed = envu("BENCH_SEED", 1) as u64;
    let mut st = seed | 1;

    // correctness guard: an exact-match query returns its id at distance 0.
    let marker = rand_hv(&mut st);
    let mut guard: FixedStore<4> = FixedStore::new();
    guard.insert(marker.clone(), 42).unwrap();
    guard.insert(rand_hv(&mut st), 7).unwrap();
    assert_eq!(guard.nearest(&marker), Some((42, 0)), "nearest must find the exact match");

    // deterministic store of N items + M queries + N pair-vectors for the micro.
    let mut store: Box<FixedStore<N>> = Box::new(FixedStore::new());
    for i in 0..N {
        store.insert(rand_hv(&mut st), i as u32).unwrap();
    }
    let queries: Vec<Hv> = (0..m).map(|_| rand_hv(&mut st)).collect();
    let pairs: Vec<Hv> = (0..N).map(|_| rand_hv(&mut st)).collect();

    // warm-up
    let mut sink = 0u64;
    for q in &queries {
        if let Some((id, d)) = store.nearest(black_box(q)) {
            sink ^= id as u64 ^ d as u64;
        }
    }

    // timed: nearest scan over the store
    let mut t_near = Vec::with_capacity(r);
    for _ in 0..r {
        let t = Instant::now();
        for q in &queries {
            if let Some((id, d)) = store.nearest(black_box(q)) {
                sink ^= id as u64 ^ d as u64;
            }
        }
        t_near.push(t.elapsed());
    }

    // timed: raw hamming
    let mut t_ham = Vec::with_capacity(r);
    let mut acc = 0u64;
    for _ in 0..r {
        let t = Instant::now();
        for (i, p) in pairs.iter().enumerate() {
            acc += black_box(p.hamming(black_box(&queries[i % m]))) as u64;
        }
        t_ham.push(t.elapsed());
    }
    black_box(sink);
    black_box(acc);

    let near = median(t_near);
    let ns_cmp = near.as_nanos() as f64 / (m * N) as f64;
    let scan_us = near.as_nanos() as f64 / m as f64 / 1000.0;
    let ham = median(t_ham);
    let ns_ham = ham.as_nanos() as f64 / pairs.len() as f64;

    println!("features: {}", features());
    println!("N={N} M={m} R={r} seed={seed}");
    println!("hamming      : {ns_ham:.3} ns/op          ({:.1} M ops/s)", 1000.0 / ns_ham);
    println!(
        "nearest scan : {ns_cmp:.3} ns/comparison   ({:.1} M cmp/s)   scan-N={N}: {scan_us:.2} us/query",
        1000.0 / ns_cmp
    );
    println!("checksum: sink={sink:#018x} acc={acc}");
}
