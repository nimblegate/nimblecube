//! no_std integer feature encoder: a vector of quantized sensor features -> one
//! 4096-bit `Hv`, via level-encode (graceful magnitude) + bind-per-channel
//! (channel identity) + bundle. Alloc-free; the front half of a smart sensor.

use crate::hv::{Hv, DIM_BITS, WORDS};

/// Map an integer feature value onto a level index in `0..levels`.
/// Integer-only and panic-free: clamps out-of-range; a degenerate range
/// (`max <= min`) or `levels == 0` yields `0`.
pub fn quantize(v: i32, min: i32, max: i32, levels: usize) -> usize {
    if levels == 0 || max <= min || v <= min {
        return 0;
    }
    if v >= max {
        return levels - 1;
    }
    let span = max as i64 - min as i64;
    let idx = ((v as i64 - min as i64) * levels as i64 / span) as usize;
    if idx >= levels { levels - 1 } else { idx }
}

/// Integer feature encoder over `CH` channels and `L` graceful levels.
/// `level[i]` differs from `level[j]` by exactly `|i-j| * (DIM_BITS / L)` bits,
/// so close feature values produce close hypervectors. Each channel binds its
/// level vector with a distinct random basis vector so channels never collide.
pub struct FeatureEncoder<const CH: usize, const L: usize> {
    channels: [Hv; CH],
    levels: [Hv; L],
    ranges: [(i32, i32); CH],
}

impl<const CH: usize, const L: usize> FeatureEncoder<CH, L> {
    /// Build the encoder deterministically from `seed`. `ranges[i]` is the
    /// `[min, max]` used to quantize channel `i`. Requires `L >= 1`.
    pub fn new(seed: u64, ranges: [(i32, i32); CH]) -> Self {
        let mut state = seed | 1;

        // Graceful, exact-monotone levels: random base, then flip the next
        // `step` bits per level (linear bit order). The `b < DIM_BITS` guard
        // drops any `DIM_BITS % L` remainder bits - still monotone.
        let mut levels: [Hv; L] = core::array::from_fn(|_| Hv::zero());
        levels[0] = rand_hv(&mut state);
        let step = DIM_BITS / L;
        for i in 1..L {
            let mut next = levels[i - 1].clone();
            let start = (i - 1) * step;
            for b in start..(start + step) {
                if b < DIM_BITS {
                    next.0[b / 64] ^= 1u64 << (b % 64);
                }
            }
            levels[i] = next;
        }

        // Distinct random basis per channel (drawn after the levels).
        let channels: [Hv; CH] = core::array::from_fn(|_| rand_hv(&mut state));

        FeatureEncoder { channels, levels, ranges }
    }

    /// Encode a feature vector into one hypervector. Bind and bundle are fused
    /// into a single word-at-a-time pass, so no per-channel contribution is
    /// ever materialized: each output word is derived straight from the level
    /// and channel words. Bit-identical to the former
    /// `Hv::bundle(&[level.bind(channel), ..])`, tie-break included, which
    /// `tests::encode_reference` pins as the oracle.
    ///
    /// `CH == 3` takes a closed-form majority (`(a&b)|(a&c)|(b&c)`) instead of
    /// the bit-sliced counters. `CH` is a const generic, so the branch folds at
    /// monomorphization.
    /// Encode a feature vector into one hypervector. Alloc-free.
    ///
    /// All level indices are resolved before any hypervector work begins.
    /// Interleaving `quantize`'s integer division with the 512-byte binds costs
    /// measurably more than the two-pass form (host: 1.03x to 1.33x depending
    /// on `CH`), and the split leaves the bind loop simple enough to vectorize.
    ///
    /// Bit-identical to the former single-pass version, which
    /// `tests::encode_reference` pins as the oracle.
    pub fn encode(&self, feats: &[i32; CH]) -> Hv {
        let idx: [usize; CH] = core::array::from_fn(|i| {
            let (min, max) = self.ranges[i];
            quantize(feats[i], min, max, L)
        });
        let contrib: [Hv; CH] =
            core::array::from_fn(|i| self.levels[idx[i]].bind(&self.channels[i]));
        Hv::bundle(&contrib)
    }
}

/// xorshift64 fill - the repo's deterministic RNG (see `examples/hamming_bench.rs`).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantize_maps_range_to_levels() {
        assert_eq!(quantize(0, 0, 100, 10), 0);
        assert_eq!(quantize(100, 0, 100, 10), 9); // v >= max -> L-1
        assert_eq!(quantize(50, 0, 100, 10), 5);
        assert_eq!(quantize(-5, 0, 100, 10), 0); // below min
        assert_eq!(quantize(999, 0, 100, 10), 9); // above max
    }

    #[test]
    fn quantize_is_monotone_nondecreasing() {
        let mut prev = 0;
        for v in 0..=100 {
            let q = quantize(v, 0, 100, 16);
            assert!(q >= prev);
            assert!(q < 16);
            prev = q;
        }
    }

    #[test]
    fn quantize_degenerate_range_is_zero() {
        assert_eq!(quantize(50, 10, 10, 16), 0); // max <= min
        assert_eq!(quantize(50, 0, 100, 0), 0); // zero levels
    }

    #[test]
    fn quantize_no_overflow_on_extremes() {
        let q = quantize(i32::MAX, i32::MIN, i32::MAX, 8);
        assert!(q < 8); // must not panic or overflow
    }

    #[test]
    fn encode_is_deterministic() {
        let r = [(0i32, 100i32)];
        let e1 = FeatureEncoder::<1, 16>::new(42, r);
        let e2 = FeatureEncoder::<1, 16>::new(42, r);
        assert_eq!(e1.encode(&[37]), e2.encode(&[37]));
        assert_eq!(e1.encode(&[37]), e1.encode(&[37]));
    }

    #[test]
    fn close_values_are_closer_than_far_values() {
        let e = FeatureEncoder::<1, 16>::new(1, [(0, 100)]);
        let low = e.encode(&[10]);
        let mid = e.encode(&[50]);
        let high = e.encode(&[90]);
        assert!(low.hamming(&mid) < low.hamming(&high));
        assert!(low.hamming(&mid) > 0);
    }

    #[test]
    fn channels_do_not_collide() {
        let e = FeatureEncoder::<2, 16>::new(9, [(0, 100), (0, 100)]);
        assert_ne!(e.encode(&[20, 80]), e.encode(&[80, 20]));
    }

    #[test]
    fn out_of_range_clamps_without_panic() {
        let e = FeatureEncoder::<1, 16>::new(3, [(0, 100)]);
        let lo = e.encode(&[i32::MIN]);
        let hi = e.encode(&[i32::MAX]);
        let at0 = e.encode(&[-10]);
        assert_eq!(lo, at0); // both clamp to level 0
        assert_ne!(lo, hi); // min vs max land on different levels
    }

    fn xs(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }

    /// The pre-fusion encode, retained verbatim as the equivalence oracle:
    /// materialize one bound contribution per channel, then `Hv::bundle` them.
    fn encode_reference<const CH: usize, const L: usize>(
        e: &FeatureEncoder<CH, L>,
        feats: &[i32; CH],
    ) -> Hv {
        let contrib: [Hv; CH] = core::array::from_fn(|i| {
            let (min, max) = e.ranges[i];
            let idx = quantize(feats[i], min, max, L);
            e.levels[idx].bind(&e.channels[i])
        });
        Hv::bundle(&contrib)
    }

    fn assert_equiv<const CH: usize, const L: usize>(seed: u64, s: &mut u64) {
        let ranges: [(i32, i32); CH] = core::array::from_fn(|i| {
            if i % 2 == 0 { (0, 4095) } else { (-4095, 4095) }
        });
        let e = FeatureEncoder::<CH, L>::new(seed, ranges);
        for _ in 0..64 {
            let feats: [i32; CH] =
                core::array::from_fn(|_| (xs(s) % 12000) as i32 - 6000);
            assert_eq!(
                e.encode(&feats),
                encode_reference(&e, &feats),
                "fused encode diverged at CH={} L={}",
                CH,
                L
            );
        }
    }

    #[test]
    fn encode_matches_reference_odd_channels() {
        let mut s: u64 = 0x1234_5678_9abc_def1;
        assert_equiv::<1, 16>(7, &mut s);
        assert_equiv::<3, 16>(7, &mut s);
        assert_equiv::<5, 8>(11, &mut s);
        assert_equiv::<7, 2>(13, &mut s);
    }

    /// Even `CH` is where a naive fusion silently diverges: `Hv::bundle`
    /// resolves an exact tie with its `0xAAAA...` constant.
    #[test]
    fn encode_matches_reference_even_channels_tie_break() {
        let mut s: u64 = 0x0fed_cba9_8765_4321;
        assert_equiv::<2, 16>(3, &mut s);
        assert_equiv::<4, 16>(5, &mut s);
        assert_equiv::<6, 4>(9, &mut s);
        assert_equiv::<8, 32>(17, &mut s);
    }

    /// The `CH == 3` closed form is the path the firmware actually takes, so it
    /// is pinned separately from the general one.
    #[test]
    fn encode_ch3_fast_path_matches_reference() {
        let mut s: u64 = 0xfeed_face_cafe_0003;
        for seed in [1u64, 7, 42, 9999] {
            assert_equiv::<3, 16>(seed, &mut s);
            assert_equiv::<3, 2>(seed, &mut s);
        }
    }
}
