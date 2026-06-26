#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::main;
use esp_println::println;

use nimblecube_core::encode::FeatureEncoder;
use nimblecube_core::store::FixedStore;

// Required by esp-hal 1.x: the ESP-IDF bootloader needs an app descriptor in the image.
esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // LED pin varies by devkit (plain GPIO2 here; some S3 boards use an addressable
    // RGB on GPIO48 - if it doesn't light, the serial output below still proves life).
    let mut led = Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default());
    let delay = Delay::new();

    println!("nimblecube-esp32 bring-up");

    // Run the HDC core once, on the chip, on synthetic features.
    let enc = FeatureEncoder::<3, 16>::new(7, [(0, 150), (0, 400), (0, 64)]);
    let mut store: FixedStore<4> = FixedStore::new();
    store.insert(enc.encode(&[50, 90, 12]), 0).unwrap(); // enroll one "normal"
    let dn = store.nearest(&enc.encode(&[52, 95, 11])).unwrap().1; // near-normal
    let df = store.nearest(&enc.encode(&[140, 360, 40])).unwrap().1; // fault-ish
    println!("normal d={} fault d={}", dn, df);

    loop {
        led.toggle();
        delay.delay_millis(500);
        println!("alive");
    }
}
