#![no_std]
#![no_main]

use core::panic::PanicInfo;
use nimblecube_core::hv::{Hv, WORDS};
use nimblecube_core::store::FixedStore;

/// Deterministic xorshift -> a 4096-bit hypervector.
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

/// Firmware entry: seed a small store, run a nearest query, and write the
/// `(id << 16) | distance` result to a fixed RAM word so the linker cannot strip
/// the core code. Then idle. (On real hardware you'd read this or drive a pin.)
#[no_mangle]
pub extern "C" fn Reset() -> ! {
    let mut s: u64 = 0xC0FF_EE00_1234_5678;
    let mut store: FixedStore<8> = FixedStore::new();
    let mut i = 0u32;
    while i < 8 {
        let _ = store.insert(rand_hv(&mut s), i);
        i += 1;
    }
    let query = rand_hv(&mut s);
    let result = match store.nearest(&query) {
        Some((id, dist)) => (id << 16) | dist,
        None => 0xFFFF_FFFF,
    };
    unsafe {
        core::ptr::write_volatile(0x2000_0000 as *mut u32, result);
    }
    loop {
        core::hint::spin_loop();
    }
}

/// Minimal vector table: initial SP (from link.x) + the reset vector. Enough to
/// link and measure (not a full boot table).
#[link_section = ".vector_table.reset_vector"]
#[no_mangle]
pub static RESET_VECTOR: extern "C" fn() -> ! = Reset;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
