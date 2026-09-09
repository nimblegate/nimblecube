//! On-chip check of `RejectNet`: the one-compare rejection filter against the
//! linear scan it is meant to replace on the anomaly path.
//!
//! `FixedStore::nearest` terminates early only when a close match tightens the
//! bound, so an anomalous query (nothing close) pays the full scan. That is the
//! slowest case and it is the one a detector exists to catch. Both query kinds
//! are timed here, on a coherent store shaped like the MQ-2 baseline.
//!   cd nimblecube-esp32 && cargo run --release --bin reject_bench

#![no_std]
#![no_main]

use core::hint::black_box;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::main;
use esp_hal::time::Instant;
use esp_println::println;

use nimblecube_core::hv::{Hv, DIM_BITS, WORDS};
use nimblecube_core::reject::RejectNet;
use nimblecube_core::store::FixedStore;

esp_bootloader_esp_idf::esp_app_desc!();

const N: usize = 64; // matches bench.rs so the numbers line up
const M: u64 = 1000;
const SPREAD: usize = 100; // bits of clean-air variation between baselines
const MARGIN: u32 = 150; // declared noise tolerance, in bits

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

fn noisy(base: &Hv, nbits: usize, state: &mut u64) -> Hv {
    let mut h = base.clone();
    for _ in 0..nbits {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        let b = (*state as usize) % DIM_BITS;
        h.0[b / 64] ^= 1u64 << (b % 64);
    }
    h
}

#[main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let _p = esp_hal::init(config);

    let mut st: u64 = 0x1234_5678_9abc_def1;

    // A coherent store: N variants of one baseline, like enrolled clean air.
    let base = rand_hv(&mut st);
    let mut store: FixedStore<N> = FixedStore::new();
    for i in 0..N {
        store.insert(noisy(&base, SPREAD, &mut st), i as u32).ok();
    }
    let mut net = RejectNet::new(MARGIN);
    net.rebuild(store.slots());

    // Two query kinds: unrelated (an anomaly) and a fresh clean-air reading.
    let anomaly: [Hv; 8] = core::array::from_fn(|_| rand_hv(&mut st));
    let normal: [Hv; 8] = core::array::from_fn(|_| noisy(&base, SPREAD, &mut st));

    // Correctness on-chip, not just speed.
    let mut rej_anom = 0u32;
    let mut rej_norm = 0u32;
    for i in 0..8 {
        if net.rejects(&anomaly[i]) {
            rej_anom += 1;
        }
        if net.rejects(&normal[i]) {
            rej_norm += 1;
        }
    }
    let d_anom = net.distance(&anomaly[0]);
    let d_norm = net.distance(&normal[0]);

    let mut acc: u64 = 0;
    for i in 0..64usize {
        acc = acc.wrapping_add(store.nearest(black_box(&anomaly[i % 8])).unwrap().1 as u64);
    }

    // --- nearest on an anomaly: nothing close, so no early termination ---
    let t0 = Instant::now();
    for i in 0..M as usize {
        acc = acc.wrapping_add(store.nearest(black_box(&anomaly[i % 8])).unwrap().1 as u64);
    }
    let near_anom_us = t0.elapsed().as_micros();

    // --- RejectNet on the same anomaly: one compare ---
    let t1 = Instant::now();
    for i in 0..M as usize {
        acc = acc.wrapping_add(net.rejects(black_box(&anomaly[i % 8])) as u64);
    }
    let rej_anom_us = t1.elapsed().as_micros();

    // --- nearest on a normal reading: early termination already works ---
    let t2 = Instant::now();
    for i in 0..M as usize {
        acc = acc.wrapping_add(store.nearest(black_box(&normal[i % 8])).unwrap().1 as u64);
    }
    let near_norm_us = t2.elapsed().as_micros();

    // --- the real path: reject first, scan only on a miss ---
    let t3 = Instant::now();
    for i in 0..M as usize {
        let q = black_box(&anomaly[i % 8]);
        if !net.rejects(q) {
            acc = acc.wrapping_add(store.nearest(q).unwrap().1 as u64);
        }
    }
    let combo_anom_us = t3.elapsed().as_micros();

    let t4 = Instant::now();
    for i in 0..M as usize {
        let q = black_box(&normal[i % 8]);
        if !net.rejects(q) {
            acc = acc.wrapping_add(store.nearest(q).unwrap().1 as u64);
        }
    }
    let combo_norm_us = t4.elapsed().as_micros();

    black_box(acc);

    let delay = Delay::new();
    loop {
        println!("nimblecube reject_bench (ESP32-S3 @ 240 MHz)  N={} M={}", N, M);
        println!("store: {} coherent baselines, spread={} bits, margin={}", N, SPREAD, MARGIN);
        println!("net radius={}  d(anomaly)={}  d(normal)={}", net.radius(), d_anom, d_norm);
        println!("rejected: {}/8 anomaly (want 8)   {}/8 normal (want 0)", rej_anom, rej_norm);
        println!("--");
        println!("anomaly  nearest      : {} us/query", near_anom_us / M);
        println!("anomaly  rejects      : {} ns/query", rej_anom_us * 1000 / M);
        println!("anomaly  reject+scan  : {} us/query", combo_anom_us / M);
        println!("normal   nearest      : {} us/query", near_norm_us / M);
        println!("normal   reject+scan  : {} us/query", combo_norm_us / M);
        println!("checksum={}", acc);
        delay.delay_millis(2000);
    }
}
