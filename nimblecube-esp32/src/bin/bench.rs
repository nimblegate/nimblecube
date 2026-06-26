#![no_std]
#![no_main]

use core::hint::black_box;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::main;
use esp_hal::time::Instant;
use esp_println::println;

use nimblecube_core::encode::FeatureEncoder;
use nimblecube_core::hv::{Hv, WORDS};
use nimblecube_core::store::FixedStore;

esp_bootloader_esp_idf::esp_app_desc!();

const N: usize = 64;
const M: u64 = 1000;

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

#[main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let _p = esp_hal::init(config);

    let mut st: u64 = 0x1234_5678_9abc_def1;
    let mut match_q = Hv::zero(); // a stored vector, for the near-match nearest timing
    let mut store: FixedStore<N> = FixedStore::new();
    for i in 0..N {
        let hv = rand_hv(&mut st);
        if i == 0 {
            match_q = hv.clone();
        }
        store.insert(hv, i as u32).ok();
    }
    let q: [Hv; 8] = core::array::from_fn(|_| rand_hv(&mut st));
    let enc = FeatureEncoder::<3, 16>::new(7, [(0, 4095), (0, 4095), (-4095, 4095)]);

    let mut acc: u64 = 0;
    // warmup
    for i in 0..64usize {
        acc = acc.wrapping_add(store.nearest(black_box(&q[i % 8])).unwrap().1 as u64);
    }

    // --- encode ---
    let t0 = Instant::now();
    for i in 0..M {
        let f = [(i as i32) % 4096, (i as i32 * 7) % 4096, (i as i32) % 200 - 100];
        acc = acc.wrapping_add(enc.encode(black_box(&f)).0[0]);
    }
    let enc_us = t0.elapsed().as_micros();

    // --- raw hamming ---
    let t1 = Instant::now();
    for i in 0..M as usize {
        acc = acc.wrapping_add(q[i % 8].hamming(black_box(&q[(i + 1) % 8])) as u64);
    }
    let ham_us = t1.elapsed().as_micros();

    // --- nearest scan ---
    let t2 = Instant::now();
    for i in 0..M as usize {
        acc = acc.wrapping_add(store.nearest(black_box(&q[i % 8])).unwrap().1 as u64);
    }
    let near_us = t2.elapsed().as_micros();

    // --- nearest where the query IS a stored vector (best_d hits 0 early -> early-term shines) ---
    let t3 = Instant::now();
    for _ in 0..M as usize {
        acc = acc.wrapping_add(store.nearest(black_box(&match_q)).unwrap().1 as u64);
    }
    let nearmatch_us = t3.elapsed().as_micros();

    black_box(acc);
    let enc_ns = enc_us * 1000 / M;
    let ham_ns = ham_us * 1000 / M;
    let near_cmp = near_us * 1000 / (M * N as u64);
    let near_q = near_us / M;
    let nearmatch_q = nearmatch_us / M;

    // Reprint on a delay: a one-shot print + tight `loop {}` doesn't reliably flush
    // the USB-Serial-JTAG FIFO and risks a watchdog reset. The delay yields and lets
    // a full, clean table reach the host every cycle.
    let delay = Delay::new();
    loop {
        println!("nimblecube bench (ESP32-S3 @ 240 MHz)  N={} M={}", N, M);
        println!("encode       : {} ns/op", enc_ns);
        println!("hamming      : {} ns/op", ham_ns);
        println!("nearest scan : {} ns/comparison   scan-{}: {} us/query (random)", near_cmp, N, near_q);
        println!("nearest(match): {} us/query (query in store)", nearmatch_q);
        println!("checksum={}", acc);
        delay.delay_millis(2000);
    }
}
