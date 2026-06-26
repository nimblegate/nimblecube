#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::analog::adc::{Adc, AdcConfig, Attenuation};
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::main;
use esp_println::println;

use nimblecube_core::encode::FeatureEncoder;
use nimblecube_core::store::FixedStore;

esp_bootloader_esp_idf::esp_app_desc!();

const W: usize = 32; // readings per window (~32 * 10 ms)
const WARMUP: usize = 30; // windows skipped for MQ-2 heater settling
const BASELINE: usize = 8; // clean-air windows enrolled as "normal"
const STORE: usize = 8;
const K: usize = 3; // persistence: alarm if >= K of the last M windows are over threshold
const M: usize = 5;

#[main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let mut adc_config = AdcConfig::new();
    let mut adc_pin = adc_config.enable_pin(peripherals.GPIO3, Attenuation::_11dB);
    let mut adc = Adc::new(peripherals.ADC1, adc_config);
    let delay = Delay::new();

    // Features: mean level, peak, slope - over the 12-bit ADC span.
    let enc = FeatureEncoder::<3, 16>::new(7, [(0, 4095), (0, 4095), (-4095, 4095)]);
    let mut store: FixedStore<STORE> = FixedStore::new();

    println!("mq2 gas_anomaly: warming up...");

    let mut t: usize = 0;
    let mut threshold: u32 = 0;
    let mut max_baseline: u32 = 0;
    let mut recent_over = [false; M];
    let mut ri = 0usize;

    loop {
        // --- sample one window ---
        let mut window = [0i32; W];
        for w in window.iter_mut() {
            let raw: u16 = nb::block!(adc.read_oneshot(&mut adc_pin)).unwrap();
            *w = raw as i32;
            delay.delay_millis(10);
        }
        // --- features ---
        let mut sum = 0i64;
        let mut peak = 0i32;
        for &v in window.iter() {
            sum += v as i64;
            if v > peak {
                peak = v;
            }
        }
        let mean = (sum / W as i64) as i32;
        let slope = window[W - 1] - window[0];
        let hv = enc.encode(&[mean, peak, slope]);

        if t < WARMUP {
            // MQ-2 heater settling - ignore.
        } else if t < WARMUP + BASELINE {
            // Enroll clean-air baseline; track the worst clean-air spread.
            if !store.is_empty() {
                let d = store.nearest(&hv).unwrap().1;
                if d > max_baseline {
                    max_baseline = d;
                }
            }
            store.insert(hv, (t - WARMUP) as u32).ok();
            println!("enroll mean={}", mean);
        } else {
            if threshold == 0 {
                threshold = max_baseline + 70;
                println!("baseline ceiling d={} -> threshold={}", max_baseline, threshold);
            }
            let d = store.nearest(&hv).unwrap().1;
            recent_over[ri % M] = d > threshold;
            ri += 1;
            let count = recent_over.iter().filter(|&&b| b).count();
            if count >= K {
                println!("mean={} d={} ANOMALY: smoke/gas", mean, d);
            } else {
                println!("mean={} d={} ok", mean, d);
            }
        }
        t += 1;
    }
}
