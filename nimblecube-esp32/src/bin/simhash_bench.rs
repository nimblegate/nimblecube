//! Is on-device embedding encoding viable at all?
//!
//! `simhash_*` costs 4096 bits x D input elements, which is orders of magnitude
//! more work than `FeatureEncoder`. This measures it on the chip, alongside the
//! bind/bundle split of `FeatureEncoder::encode` so the two are comparable.
//!
//! Second question: the integer path accumulates into `i64`, and the LX7 is a
//! 32-bit core with a single-precision FPU, so `simhash_f32` may beat
//! `simhash_i32` here despite "integer-only" being the usual preference.
//!   cd nimblecube-esp32 && cargo run --release --bin simhash_bench

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
use nimblecube_core::simhash::{simhash_f32, simhash_i32};

esp_bootloader_esp_idf::esp_app_desc!();

const D: usize = 768; // a typical embedding width
const MS: u64 = 3; // simhash reps (expected to be slow)
const MF: u64 = 200; // FeatureEncoder reps

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

#[main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let _p = esp_hal::init(config);

    let mut st: u64 = 0x1234_5678_9abc_def1;
    let mut vi = [0i32; D];
    let mut vf = [0f32; D];
    for i in 0..D {
        let x = (xs(&mut st) % 2001) as i32 - 1000;
        vi[i] = x;
        vf[i] = x as f32;
    }

    let a = rand_hv(&mut st);
    let b = rand_hv(&mut st);
    let c = rand_hv(&mut st);
    let enc = FeatureEncoder::<3, 16>::new(7, [(0, 4095), (0, 4095), (-4095, 4095)]);

    let mut acc = 0u64;

    // --- FeatureEncoder, whole ---
    let t = Instant::now();
    for i in 0..MF {
        let f = [(i as i32) % 4096, (i as i32 * 7) % 4096, (i as i32) % 200 - 100];
        acc = acc.wrapping_add(enc.encode(black_box(&f)).0[0]);
    }
    let enc_ns = t.elapsed().as_micros() * 1000 / MF;

    // --- the two halves of encode, via public API ---
    let t = Instant::now();
    for _ in 0..MF {
        acc = acc.wrapping_add(black_box(&a).bind(black_box(&b)).0[0]);
        acc = acc.wrapping_add(black_box(&b).bind(black_box(&c)).0[0]);
        acc = acc.wrapping_add(black_box(&c).bind(black_box(&a)).0[0]);
    }
    let bind3_ns = t.elapsed().as_micros() * 1000 / MF;

    let trio = [a.clone(), b.clone(), c.clone()];
    let t = Instant::now();
    for _ in 0..MF {
        acc = acc.wrapping_add(Hv::bundle(black_box(&trio)).0[0]);
    }
    let bundle3_ns = t.elapsed().as_micros() * 1000 / MF;

    let t = Instant::now();
    for _ in 0..MF {
        acc = acc.wrapping_add(black_box(&a).hamming(black_box(&b)) as u64);
    }
    let ham_ns = t.elapsed().as_micros() * 1000 / MF;

    // --- simhash: integer vs float, and scaling in D ---
    let t = Instant::now();
    for _ in 0..MS {
        acc = acc.wrapping_add(simhash_i32(black_box(&vi[..128]), 7).0[0]);
    }
    let si128_us = t.elapsed().as_micros() / MS;

    let t = Instant::now();
    for _ in 0..MS {
        acc = acc.wrapping_add(simhash_i32(black_box(&vi[..384]), 7).0[0]);
    }
    let si384_us = t.elapsed().as_micros() / MS;

    let t = Instant::now();
    for _ in 0..MS {
        acc = acc.wrapping_add(simhash_i32(black_box(&vi), 7).0[0]);
    }
    let si768_us = t.elapsed().as_micros() / MS;

    let t = Instant::now();
    for _ in 0..MS {
        acc = acc.wrapping_add(simhash_f32(black_box(&vf), 7).0[0]);
    }
    let sf768_us = t.elapsed().as_micros() / MS;

    // sanity: both paths must agree on the same data
    let hi = simhash_i32(&vi, 7);
    let hf = simhash_f32(&vf, 7);
    let agree = hi.hamming(&hf);

    black_box(acc);
    let delay = Delay::new();
    // The USB-Serial-JTAG FIFO is 64 bytes and drops what it cannot hold, so a
    // burst of println! loses every line after the first. Pace them.
    macro_rules! line {
        ($($a:tt)*) => {{ println!($($a)*); delay.delay_millis(25); }};
    }
    loop {
        line!("simhash_bench (ESP32-S3 @ 240 MHz)  D={}", D);
        line!("-- FeatureEncoder path (ns/op) --");
        line!("hamming            : {} ns", ham_ns);
        line!("3x bind            : {} ns", bind3_ns);
        line!("bundle of 3        : {} ns", bundle3_ns);
        line!("encode (whole)     : {} ns", enc_ns);
        line!("-- SimHash path (us/vector) --");
        line!("simhash_i32 D=128  : {} us", si128_us);
        line!("simhash_i32 D=384  : {} us", si384_us);
        line!("simhash_i32 D=768  : {} us", si768_us);
        line!("simhash_f32 D=768  : {} us", sf768_us);
        line!("i32 vs f32 hamming : {} (0 = identical codes)", agree);
        line!("checksum={}", acc);
        delay.delay_millis(3000);
    }
}
