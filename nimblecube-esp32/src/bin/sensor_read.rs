#![no_std]
#![no_main]

use esp_backtrace as _;
use esp_hal::analog::adc::{Adc, AdcConfig, Attenuation};
use esp_hal::clock::CpuClock;
use esp_hal::delay::Delay;
use esp_hal::main;
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // MQ-2 AOUT on ADC1 / GPIO3, 11 dB attenuation = full ~0..3.3 V range.
    let mut adc_config = AdcConfig::new();
    let mut adc_pin = adc_config.enable_pin(peripherals.GPIO3, Attenuation::_11dB);
    let mut adc = Adc::new(peripherals.ADC1, adc_config);
    let delay = Delay::new();

    println!("mq2 sensor_read: raw ADC on GPIO3 (wave smoke, watch it rise)");
    loop {
        let raw: u16 = nb::block!(adc.read_oneshot(&mut adc_pin)).unwrap();
        println!("raw={}", raw);
        delay.delay_millis(100);
    }
}
