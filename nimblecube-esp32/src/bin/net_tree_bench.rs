//! Stage 2 of the net-tree evaluation: does the compare-count win survive as
//! wall-clock on real hardware?
//!
//! `examples/net_tree_eval.rs` measured 260 compares at 100% recall on the host,
//! against 12000 for a linear scan. Compare count is not time (the encode hoist
//! was 1.21x on x86 and 2% slower here), so this rebuilds the same index in
//! PSRAM and times it. Recall is re-checked on-chip rather than assumed.
//!   cd nimblecube-esp32 && cargo run --release --bin net_tree_bench

#![no_std]
#![no_main]

extern crate alloc;
use alloc::vec::Vec;
use core::hint::black_box;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::main;
use esp_hal::time::Instant;
use esp_println::println;
use nimblecube_core::hv::{Hv, DIM_BITS, WORDS};

esp_bootloader_esp_idf::esp_app_desc!();

const N: usize = 12000; // matches psram_bench and the host eval
const G: usize = 200; // clusters
const FLIP: usize = 200; // intra-cluster spread, in bits
const CAP: usize = 64; // members per cell, the SNR-bounded fan-out
const JOIN_RADIUS: u32 = (DIM_BITS / 4) as u32;
const Q: usize = 20; // queries (a linear scan is ~278 ms each)

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

/// Ground-truth nearest distance, full scan with early termination (the path
/// `psram_bench` measures at ~278 ms).
fn linear_nearest(items: &[Hv], q: &Hv) -> u32 {
    let mut best_d = items[0].hamming(q);
    for it in items.iter().skip(1) {
        let s = &it.0;
        let qq = &q.0;
        let mut d = 0u32;
        let mut w = 0;
        while w < WORDS {
            d += (s[w] ^ qq[w]).count_ones();
            if d >= best_d {
                break;
            }
            w += 1;
        }
        if w == WORDS {
            best_d = d;
        }
    }
    best_d
}

struct NetTree {
    nets: Vec<Hv>,
    members: Vec<Vec<u32>>,
}

impl NetTree {
    fn rebundle(&mut self, items: &[Hv], c: usize) {
        let mems: Vec<Hv> = self.members[c].iter().map(|&id| items[id as usize].clone()).collect();
        self.nets[c] = Hv::bundle(&mems);
    }

    fn insert(&mut self, items: &[Hv], id: u32) {
        let mut best = usize::MAX;
        let mut bd = u32::MAX;
        for (c, net) in self.nets.iter().enumerate() {
            if self.members[c].len() >= CAP {
                continue;
            }
            let d = net.hamming(&items[id as usize]);
            if d < bd {
                bd = d;
                best = c;
            }
        }
        if best == usize::MAX || bd > JOIN_RADIUS {
            self.nets.push(items[id as usize].clone());
            let mut m = Vec::new();
            m.push(id);
            self.members.push(m);
            return;
        }
        self.members[best].push(id);
        self.rebundle(items, best);
    }

    fn query(&self, items: &[Hv], q: &Hv, nprobe: usize) -> u32 {
        // rank cells by distance to the query; nprobe is small so a partial
        // selection beats sorting the whole list
        let mut chosen = [usize::MAX; 4];
        let mut chosen_d = [u32::MAX; 4];
        for (c, net) in self.nets.iter().enumerate() {
            let d = net.hamming(q);
            for p in 0..nprobe.min(4) {
                if d < chosen_d[p] {
                    for k in (p + 1..nprobe.min(4)).rev() {
                        chosen_d[k] = chosen_d[k - 1];
                        chosen[k] = chosen[k - 1];
                    }
                    chosen_d[p] = d;
                    chosen[p] = c;
                    break;
                }
            }
        }
        let mut bd = u32::MAX;
        for p in 0..nprobe.min(4) {
            if chosen[p] == usize::MAX {
                continue;
            }
            for &id in &self.members[chosen[p]] {
                let d = items[id as usize].hamming(q);
                if d < bd {
                    bd = d;
                }
            }
        }
        bd
    }
}

#[main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let psram_config = esp_hal::psram::PsramConfig {
        mode: esp_hal::psram::PsramMode::OctalSpi,
        size: esp_hal::psram::PsramSize::AutoDetect,
        ..Default::default()
    };
    esp_alloc::psram_allocator!(peripherals.PSRAM, esp_hal::psram, psram_config);

    let delay = Delay::new();
    println!("net_tree_bench: generating {} clustered items...", N);

    let mut st: u64 = 0x000A_11CE_5EED;
    let bases: Vec<Hv> = (0..G).map(|_| rand_hv(&mut st)).collect();
    let mut items: Vec<Hv> = Vec::with_capacity(N);
    for i in 0..N {
        items.push(noisy(&bases[i % G], FLIP, &mut st));
    }
    let queries: Vec<Hv> = (0..Q)
        .map(|_| {
            let b = (xs(&mut st) as usize) % G;
            noisy(&bases[b], FLIP, &mut st)
        })
        .collect();

    println!("building net tree (cap={}, this takes a while)...", CAP);
    let tb = Instant::now();
    let mut tree = NetTree { nets: Vec::new(), members: Vec::new() };
    for i in 0..N {
        tree.insert(&items, i as u32);
    }
    let build_ms = tb.elapsed().as_micros() / 1000;
    println!("built {} cells in {} ms", tree.nets.len(), build_ms);

    // ground truth + linear timing
    let mut truth: Vec<u32> = Vec::with_capacity(Q);
    let t0 = Instant::now();
    for q in queries.iter() {
        truth.push(linear_nearest(&items, black_box(q)));
    }
    let lin_us = t0.elapsed().as_micros() / Q as u64;

    let t1 = Instant::now();
    let mut hit1 = 0usize;
    for (i, q) in queries.iter().enumerate() {
        if tree.query(&items, black_box(q), 1) == truth[i] {
            hit1 += 1;
        }
    }
    let tree1_us = t1.elapsed().as_micros() / Q as u64;

    let t2 = Instant::now();
    let mut hit2 = 0usize;
    for (i, q) in queries.iter().enumerate() {
        if tree.query(&items, black_box(q), 2) == truth[i] {
            hit2 += 1;
        }
    }
    let tree2_us = t2.elapsed().as_micros() / Q as u64;

    loop {
        println!("net_tree_bench (ESP32-S3 @ 240 MHz)  N={} clusters={} cap={}", N, G, CAP);
        println!("cells={}  build={} ms  queries={}", tree.nets.len(), build_ms, Q);
        println!("--");
        println!("linear scan     : {} us/query   (recall 100% by definition)", lin_us);
        println!("net tree nprobe1: {} us/query   recall {}/{}", tree1_us, hit1, Q);
        println!("net tree nprobe2: {} us/query   recall {}/{}", tree2_us, hit2, Q);
        println!("speedup nprobe1 : {}x", lin_us / tree1_us.max(1));
        println!("speedup nprobe2 : {}x", lin_us / tree2_us.max(1));
        delay.delay_millis(3000);
    }
}
