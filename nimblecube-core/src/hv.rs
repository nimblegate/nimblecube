//! Hypervector type and core operations.

/// Number of bits in a hypervector.
pub const DIM_BITS: usize = 4096;
/// Number of u64 words backing a hypervector.
pub const WORDS: usize = DIM_BITS / 64; // 64

/// A fixed-size binary hypervector (4096 bits packed into 64 u64 words).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Hv(pub [u64; WORDS]);

/// Carry-save adder: returns (carry, sum) of the three input words.
#[inline]
fn csa(a: u64, b: u64, c: u64) -> (u64, u64) {
    let u = a ^ b;
    ((a & b) | (u & c), u ^ c)
}

impl Hv {
    /// All-zero hypervector.
    pub fn zero() -> Self {
        Hv([0u64; WORDS])
    }

    /// Bind = bitwise XOR. Self-inverse: `a.bind(b).bind(b) == a`.
    pub fn bind(&self, other: &Hv) -> Hv {
        let mut out = [0u64; WORDS];
        for (i, item) in out.iter_mut().enumerate().take(WORDS) {
            *item = self.0[i] ^ other.0[i];
        }
        Hv(out)
    }

    /// Number of differing bits between two hypervectors.
    pub fn hamming(&self, other: &Hv) -> u32 {
        // Harley-Seal carry-save popcount of the 64 XOR words: ~8 `count_ones`
        // instead of 64. Bit-identical to the per-word sum. WORDS=64 = 4 blocks of 16.
        let x = |i: usize| self.0[i] ^ other.0[i];
        let mut ones = 0u64;
        let mut twos = 0u64;
        let mut fours = 0u64;
        let mut eights = 0u64;
        let mut total: u32 = 0;
        let mut i = 0;
        while i + 16 <= WORDS {
            let (twos_a, o) = csa(ones, x(i), x(i + 1));
            ones = o;
            let (twos_b, o) = csa(ones, x(i + 2), x(i + 3));
            ones = o;
            let (four_a, t) = csa(twos, twos_a, twos_b);
            twos = t;
            let (twos_a, o) = csa(ones, x(i + 4), x(i + 5));
            ones = o;
            let (twos_b, o) = csa(ones, x(i + 6), x(i + 7));
            ones = o;
            let (four_b, t) = csa(twos, twos_a, twos_b);
            twos = t;
            let (eight_a, f) = csa(fours, four_a, four_b);
            fours = f;
            let (twos_a, o) = csa(ones, x(i + 8), x(i + 9));
            ones = o;
            let (twos_b, o) = csa(ones, x(i + 10), x(i + 11));
            ones = o;
            let (four_a, t) = csa(twos, twos_a, twos_b);
            twos = t;
            let (twos_a, o) = csa(ones, x(i + 12), x(i + 13));
            ones = o;
            let (twos_b, o) = csa(ones, x(i + 14), x(i + 15));
            ones = o;
            let (four_b, t) = csa(twos, twos_a, twos_b);
            twos = t;
            let (eight_b, f) = csa(fours, four_a, four_b);
            fours = f;
            let (sixteens, e) = csa(eights, eight_a, eight_b);
            eights = e;
            total += sixteens.count_ones();
            i += 16;
        }
        total = 16 * total
            + 8 * eights.count_ones()
            + 4 * fours.count_ones()
            + 2 * twos.count_ones()
            + ones.count_ones();
        total
    }

    /// Normalized similarity in [0.0, 1.0]: 1.0 = identical, 0.5 = orthogonal.
    pub fn similarity(&self, other: &Hv) -> f32 {
        (DIM_BITS as f32 - self.hamming(other) as f32) / DIM_BITS as f32
    }

    /// Total number of set bits.
    pub fn count_ones(&self) -> u32 {
        self.0.iter().map(|w| w.count_ones()).sum()
    }

    /// Bundle (superpose) a set of hypervectors by bitwise majority vote.
    /// The result is similar to every input. Ties (only possible with an even
    /// count) are broken deterministically by bit position parity so the
    /// operation is reproducible.
    pub fn bundle(vectors: &[Hv]) -> Hv {
        let n = vectors.len();
        if n == 0 {
            return Hv::zero();
        }
        if n == 1 {
            return vectors[0].clone();
        }
        // Word-parallel bit-sliced majority - bit-identical to the former scalar loop,
        // incl. the even-n tie-break. See docs/superpowers/specs/2026-06-18-bundle-wordparallel-design.md.
        let p_bits = (usize::BITS - n.leading_zeros()) as usize;
        let half = n / 2;
        let tie_const: u64 = if n % 2 == 0 { 0xAAAA_AAAA_AAAA_AAAA } else { 0 };
        let mut out = [0u64; WORDS];
        let mut c = [0u64; 64];
        for w in 0..WORDS {
            for p in 0..p_bits {
                c[p] = 0;
            }
            // bit-sliced count of set bits across the n vectors' word w
            for v in vectors {
                let mut carry = v.0[w];
                let mut p = 0;
                while carry != 0 && p < p_bits {
                    let new = c[p] ^ carry;
                    carry = c[p] & carry;
                    c[p] = new;
                    p += 1;
                }
            }
            // per-lane compare of count vs half: gt = count > half, eq = count == half
            let mut gt = 0u64;
            let mut eq = u64::MAX;
            for p in (0..p_bits).rev() {
                let tb = if (half >> p) & 1 == 1 { u64::MAX } else { 0u64 };
                let vb = c[p];
                gt |= eq & vb & !tb;
                eq &= !(vb ^ tb);
            }
            out[w] = gt | (eq & tie_const);
        }
        Hv(out)
    }

    /// Cyclic right-rotation of the whole 4096-bit vector by `shift` bits
    /// (direction is symmetric for HDC; only invertibility matters).
    /// Invertible: `v.permute(s).permute(DIM_BITS - s) == v`. Used to encode
    /// roles/positions (reserved for future structured encoders).
    pub fn permute(&self, shift: usize) -> Hv {
        let shift = shift % DIM_BITS;
        if shift == 0 {
            return self.clone();
        }
        let word_shift = shift / 64;
        let bit_shift = shift % 64;
        let mut out = [0u64; WORDS];
        for (ow, item) in out.iter_mut().enumerate().take(WORDS) {
            let sw0 = (ow + word_shift) % WORDS;
            if bit_shift == 0 {
                *item = self.0[sw0];
            } else {
                let sw1 = (sw0 + 1) % WORDS;
                *item = (self.0[sw0] >> bit_shift) | (self.0[sw1] << (64 - bit_shift));
            }
        }
        Hv(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Exact pre-optimization scalar bundle, retained as the equivalence oracle.
    fn bundle_scalar(vectors: &[Hv]) -> Hv {
        let n = vectors.len();
        if n == 0 {
            return Hv::zero();
        }
        if n == 1 {
            return vectors[0].clone();
        }
        let mut out = [0u64; WORDS];
        for (w, item) in out.iter_mut().enumerate().take(WORDS) {
            for bit in 0..64 {
                let mut count = 0usize;
                for v in vectors {
                    count += ((v.0[w] >> bit) & 1) as usize;
                }
                let set = if count * 2 > n {
                    true
                } else if count * 2 == n {
                    ((w * 64 + bit) & 1) == 1
                } else {
                    false
                };
                if set {
                    *item |= 1u64 << bit;
                }
            }
        }
        Hv(out)
    }

    fn xs(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }
    fn rand_hv_t(s: &mut u64) -> Hv {
        let mut w = [0u64; WORDS];
        for x in w.iter_mut() {
            *x = xs(s);
        }
        Hv(w)
    }

    #[test]
    fn bundle_matches_scalar() {
        let mut s: u64 = 0x1234_5678_9abc_def1;
        for n in 0..=65usize {
            for _ in 0..20 {
                let vs: Vec<Hv> = (0..n).map(|_| rand_hv_t(&mut s)).collect();
                assert_eq!(Hv::bundle(&vs), bundle_scalar(&vs), "random mismatch n={}", n);
            }
        }
        // all-tie even-n: half all-ones, half all-zeros -> every bit count = n/2
        for n in (2..=64usize).step_by(2) {
            let vs: Vec<Hv> = (0..n)
                .map(|i| if i < n / 2 { Hv([u64::MAX; WORDS]) } else { Hv([0u64; WORDS]) })
                .collect();
            assert_eq!(Hv::bundle(&vs), bundle_scalar(&vs), "tie mismatch n={}", n);
        }
    }

    fn pattern(word: u64) -> Hv {
        Hv([word; WORDS])
    }

    #[test]
    fn bind_is_self_inverse() {
        let a = pattern(0xAAAA_AAAA_AAAA_AAAA);
        let b = pattern(0x0123_4567_89AB_CDEF);
        // a XOR b XOR b == a
        assert_eq!(a.bind(&b).bind(&b), a);
    }

    #[test]
    fn similarity_of_identical_is_one() {
        let a = pattern(0xAAAA_AAAA_AAAA_AAAA);
        assert_eq!(a.similarity(&a), 1.0);
    }

    #[test]
    fn hamming_all_bits_differ() {
        let zeros = pattern(0x0);
        let ones = pattern(u64::MAX);
        assert_eq!(zeros.hamming(&ones), DIM_BITS as u32);
        assert_eq!(zeros.similarity(&ones), 0.0);
    }

    #[test]
    fn count_ones_counts_all_words() {
        let ones = pattern(u64::MAX);
        assert_eq!(ones.count_ones(), DIM_BITS as u32);
    }

    #[test]
    fn bind_computes_actual_xor() {
        let a = pattern(0xF0F0_F0F0_F0F0_F0F0);
        let b = pattern(0x0F0F_0F0F_0F0F_0F0F);
        assert_eq!(a.bind(&b), pattern(0xFFFF_FFFF_FFFF_FFFF));
        // binding a value with itself yields the all-zero vector
        assert_eq!(a.bind(&a), Hv::zero());
    }

    #[test]
    fn ops_cover_all_words_not_just_the_first() {
        // Put distinct data only in the LAST word; a bug that touches only
        // word 0 would pass the uniform-pattern tests but fail here.
        let mut wa = [0u64; WORDS];
        let mut wb = [0u64; WORDS];
        wa[WORDS - 1] = 0xFFFF_FFFF_FFFF_FFFF; // 64 ones in the last word
        wb[WORDS - 1] = 0x0000_0000_FFFF_FFFF; // low 32 ones in the last word
        let a = Hv(wa);
        let b = Hv(wb);
        assert_eq!(a.count_ones(), 64);
        assert_eq!(a.hamming(&b), 32); // differ in the high 32 bits of the last word
    }

    #[test]
    fn similarity_is_half_when_half_the_bits_differ() {
        // Exactly 2048 of 4096 bits differ -> similarity 0.5.
        let mut w = [0u64; WORDS];
        for slot in w.iter_mut().take(WORDS / 2) {
            *slot = u64::MAX; // 32 words * 64 = 2048 set bits
        }
        let a = Hv(w);
        let b = Hv::zero();
        assert_eq!(a.hamming(&b), 2048);
        assert!((a.similarity(&b) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn permute_changes_the_vector() {
        let v = pattern(0x0123_4567_89AB_CDEF);
        assert_ne!(v.permute(7), v);
    }

    #[test]
    fn permute_is_invertible() {
        // Rotating left by s, then left by (DIM_BITS - s), returns the original.
        let v = pattern(0x0123_4567_89AB_CDEF);
        let s = 7;
        assert_eq!(v.permute(s).permute(DIM_BITS - s), v);
    }

    #[test]
    fn permute_by_whole_dim_is_identity() {
        let v = pattern(0xDEAD_BEEF_DEAD_BEEF);
        assert_eq!(v.permute(DIM_BITS), v);
        assert_eq!(v.permute(0), v);
    }

    #[test]
    fn permute_invertible_on_nonuniform_vector() {
        // Distinct data across words so a wrong word/bit shift is detectable.
        let mut w = [0u64; WORDS];
        w[0] = 0x0123_4567_89AB_CDEF;
        w[1] = 0xFFFF_0000_FFFF_0000;
        w[WORDS - 1] = 0x00000000_DEADBEEF;
        let v = Hv(w);
        for s in [1usize, 63, 64, 65, 1000, 4095] {
            assert_eq!(v.permute(s).permute(DIM_BITS - s), v, "failed at shift {}", s);
            assert_ne!(v.permute(s), v, "shift {} should change a non-uniform vector", s);
        }
    }

    #[test]
    fn bundle_of_empty_is_zero() {
        assert_eq!(Hv::bundle(&[]), Hv::zero());
    }

    #[test]
    fn bundle_of_one_is_identity() {
        let a = pattern(0xAAAA_AAAA_AAAA_AAAA);
        assert_eq!(Hv::bundle(core::slice::from_ref(&a)), a);
    }

    #[test]
    fn bundle_takes_bitwise_majority() {
        // Bit 0 set in 2 of 3 -> stays set. Bit 1 set in 1 of 3 -> cleared.
        let a = Hv({ let mut w = [0u64; WORDS]; w[0] = 0b01; w }); // bit0
        let b = Hv({ let mut w = [0u64; WORDS]; w[0] = 0b01; w }); // bit0
        let c = Hv({ let mut w = [0u64; WORDS]; w[0] = 0b10; w }); // bit1
        let r = Hv::bundle(&[a, b, c]);
        assert_eq!(r.0[0], 0b01, "only the majority bit0 should be set in word 0");
        assert!(r.0[1..].iter().all(|&x| x == 0), "no spurious bits in other words");
    }

    #[test]
    fn bundle_tie_break_is_deterministic_by_bit_parity() {
        // n=2, a bit set in exactly one input -> tie (count*2 == n).
        // Tie resolves to SET iff the global bit index (w*64 + bit) is odd.
        let mut wa = [0u64; WORDS];
        wa[0] = 0b11; // global indices 0 (even) and 1 (odd)
        wa[1] = 0b11; // global indices 64 (even) and 65 (odd)
        let a = Hv(wa);
        let b = Hv::zero();
        let r = Hv::bundle(&[a, b]);
        assert_eq!(r.0[0], 0b10, "word 0: even index cleared, odd index set");
        assert_eq!(r.0[1], 0b10, "word 1: even index cleared, odd index set");
    }

    fn hamming_scalar_t(a: &Hv, b: &Hv) -> u32 {
        let mut d = 0u32;
        for i in 0..WORDS {
            d += (a.0[i] ^ b.0[i]).count_ones();
        }
        d
    }

    #[test]
    fn hamming_matches_scalar() {
        let mut s: u64 = 0x0000_DEAD_BEEF_1234;
        for _ in 0..2000 {
            let a = rand_hv_t(&mut s);
            let b = rand_hv_t(&mut s);
            assert_eq!(a.hamming(&b), hamming_scalar_t(&a, &b));
        }
        let z = Hv([0u64; WORDS]);
        let f = Hv([u64::MAX; WORDS]);
        assert_eq!(z.hamming(&z), 0);
        assert_eq!(z.hamming(&f), 4096);
        let mut one = Hv([0u64; WORDS]);
        one.0[40] = 1u64 << 33;
        assert_eq!(z.hamming(&one), 1);
        assert_eq!(one.hamming(&one), 0);
    }
}
