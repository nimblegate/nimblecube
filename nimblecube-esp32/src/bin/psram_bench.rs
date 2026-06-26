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
use nimblecube_core::hv::{Hv, WORDS};

esp_bootloader_esp_idf::esp_app_desc!();

const N: usize = 12000;
const M: u64 = 10;

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

// Early-termination nearest over a slice (id = index).
fn nearest(items: &[Hv], query: &Hv) -> (u32, u32) {
    let mut best_id = 0u32;
    let mut best_d = items[0].hamming(query);
    for i in 1..items.len() {
        let s = &items[i].0;
        let q = &query.0;
        let mut d = 0u32;
        let mut w = 0;
        while w < WORDS {
            d += (s[w] ^ q[w]).count_ones();
            if d >= best_d {
                break;
            }
            w += 1;
        }
        if w == WORDS {
            best_d = d;
            best_id = i as u32;
        }
    }
    (best_id, best_d)
}

#[main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // 8 MB octal PSRAM as the global heap (N16R8 is octal - explicit mode).
    let psram_config = esp_hal::psram::PsramConfig {
        mode: esp_hal::psram::PsramMode::OctalSpi,
        size: esp_hal::psram::PsramSize::AutoDetect,
        ..Default::default()
    };
    esp_alloc::psram_allocator!(peripherals.PSRAM, esp_hal::psram, psram_config);

    let delay = Delay::new();

    // N random hypervectors, allocated in PSRAM.
    let mut st: u64 = 0x1234_5678_9abc_def1;
    let mut items: Vec<Hv> = Vec::with_capacity(N);
    for _ in 0..N {
        items.push(rand_hv(&mut st));
    }
    let match_q = items[0].clone();
    let rand_q = rand_hv(&mut st);

    let mut acc: u64 = 0;
    let t0 = Instant::now();
    for _ in 0..M {
        acc = acc.wrapping_add(nearest(&items, black_box(&rand_q)).1 as u64);
    }
    let rand_us = t0.elapsed().as_micros();

    let t1 = Instant::now();
    for _ in 0..M {
        acc = acc.wrapping_add(nearest(&items, black_box(&match_q)).1 as u64);
    }
    let match_us = t1.elapsed().as_micros();

    black_box(acc);
    loop {
        println!(
            "psram capacity (ESP32-S3-N16R8)  N={}  bytes={} (~{} MB in PSRAM)",
            N,
            N * 512,
            N * 512 / 1024 / 1024
        );
        println!("nearest(random): {} ms/query", rand_us / M / 1000);
        println!("nearest(match) : {} us/query", match_us / M);
        println!("checksum={}", acc);
        delay.delay_millis(3000);
    }
}
