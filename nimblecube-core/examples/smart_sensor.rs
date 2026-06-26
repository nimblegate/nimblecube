//! Sub-project D end-to-end demo: synthetic vibration -> integer features ->
//! FeatureEncoder -> FixedStore enroll "normal" -> detect a fault, with a
//! hiccup-vs-sustained persistence rule. Deterministic (fixed seed). Host/std.
//!   cargo run --release --example smart_sensor
//! Env: SENSOR_SEED (default 7).

use nimblecube_core::encode::FeatureEncoder;
use nimblecube_core::store::FixedStore;

const W: usize = 64; // samples per window
const CH: usize = 3; // features per window
const L: usize = 16; // levels
const STORE: usize = 16; // enrolled normals

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// Synthetic vibration window: base oscillation + small noise; a fault adds a
/// periodic spike train (a distinct, larger-amplitude signature).
fn gen_window(state: &mut u64, faulty: bool) -> [i32; W] {
    let mut w = [0i32; W];
    for (i, s) in w.iter_mut().enumerate() {
        let phase = (i % 16) as i32;
        let base = (phase - 8) * 12; // triangle-ish, ~[-96, 84]
        let noise = (xorshift(state) % 11) as i32 - 5; // [-5, 5]
        let mut v = base + noise;
        if faulty && i % 8 == 0 {
            v += 220; // fault spike
        }
        *s = v;
    }
    w
}

/// Toy integer features: mean-abs amplitude, peak, zero-crossings.
fn features(win: &[i32; W]) -> [i32; CH] {
    let mut sum_abs = 0i64;
    let mut peak = 0i32;
    let mut zc = 0i32;
    for i in 0..W {
        let a = win[i].unsigned_abs() as i32;
        sum_abs += a as i64;
        if a > peak {
            peak = a;
        }
        if i > 0 && (win[i - 1] < 0) != (win[i] < 0) {
            zc += 1;
        }
    }
    [(sum_abs / W as i64) as i32, peak, zc]
}

fn main() {
    let seed: u64 = std::env::var("SENSOR_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(7);

    // Ranges sized to the feature spans above (mean-abs, peak, zero-crossings).
    let enc = FeatureEncoder::<CH, L>::new(seed, [(0, 150), (0, 400), (0, 64)]);
    let mut gen = seed | 1;

    // Enroll "normal" windows.
    let mut store: FixedStore<STORE> = FixedStore::new();
    for id in 0..STORE as u32 {
        let win = gen_window(&mut gen, false);
        store.insert(enc.encode(&features(&win)), id).unwrap();
    }

    // Held-out normals vs faults: nearest Hamming to the enrolled set.
    let mut max_normal = 0u32;
    let mut sum = 0u64;
    println!("--- detection (nearest Hamming to enrolled normal) ---");
    for _ in 0..8 {
        let nd = store.nearest(&enc.encode(&features(&gen_window(&mut gen, false)))).unwrap().1;
        let fd = store.nearest(&enc.encode(&features(&gen_window(&mut gen, true)))).unwrap().1;
        if nd > max_normal {
            max_normal = nd;
        }
        sum += nd as u64 + fd as u64;
        println!("  normal d={nd:4}   fault d={fd:4}");
    }
    let threshold = max_normal + max_normal / 4 + 50; // margin above worst normal
    println!("threshold = {threshold}  (worst normal = {max_normal})");

    // Persistence: a lone hiccup must NOT alarm; a sustained run must.
    let mut stream: Vec<bool> = Vec::new();
    stream.extend(std::iter::repeat(false).take(6));
    stream.push(true); // lone hiccup
    stream.extend(std::iter::repeat(false).take(4));
    stream.extend(std::iter::repeat(true).take(6)); // sustained fault

    let (k, m) = (3usize, 5usize); // alarm if >= k of the last m windows are over threshold
    let mut flags: Vec<bool> = Vec::new();
    let mut alarmed_at: Option<usize> = None;
    println!("--- persistence stream (alarm if >= {k} of last {m} over threshold) ---");
    for (t, &faulty) in stream.iter().enumerate() {
        let d = store.nearest(&enc.encode(&features(&gen_window(&mut gen, faulty)))).unwrap().1;
        let over = d > threshold;
        flags.push(over);
        let recent = flags.len().saturating_sub(m);
        let count = flags[recent..].iter().filter(|&&b| b).count();
        let alarm = count >= k;
        if alarm && alarmed_at.is_none() {
            alarmed_at = Some(t);
        }
        println!("  t={t:2} faulty={faulty:5} d={d:4} over={over:5} alarm={alarm}");
    }
    println!(
        "alarm first fired at t={:?} (expected in the sustained run t>=11, NOT the lone hiccup t=6)",
        alarmed_at
    );
    println!("checksum sum_d={sum}");
}
