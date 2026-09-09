//! One-compare rejection filter: "is anything in this store close at all?"
//!
//! [`FixedStore::nearest`](crate::store::FixedStore::nearest) terminates early
//! only when it has a close match to bail against. When nothing matches, the
//! bound never tightens and the full scan is paid, so the linear scan is at its
//! slowest exactly on the anomaly path a detector exists to catch.
//!
//! A `RejectNet` is the mirror image. All enrolled vectors are bundled into one
//! `Hv` (a superposition is similar to every one of its members), so a single
//! `hamming` against that bundle answers "is anything here like the query?" for
//! the whole store at the cost of one compare.
//!
//! # The margin is a guarantee, not a tuning knob
//!
//! `radius` is measured at [`rebuild`](RejectNet::rebuild) as the largest
//! member-to-bundle distance, and a query is rejected only past
//! `radius + margin`. Hamming is a metric, so for an enrolled member `m` and a
//! query `q` within `e` bits of it:
//!
//! ```text
//! d(q, net) <= d(m, net) + e <= radius + e
//! ```
//!
//! With margin `m`, **any query within `m` bits of an enrolled vector is never
//! rejected.** Structurally, not statistically. `margin` is the noise tolerance
//! being declared, in bits.
//!
//! What it does not promise: a query farther than `margin` from everything may
//! be rejected even when its true nearest sits just past that band. That is
//! threshold-detector semantics, and it is why this filter is separate from
//! `nearest`, which stays exact.
//!
//! # Operating envelope
//!
//! Usefulness is governed by how *coherent* the enrolled set is, not by how
//! large it is. `radius` is the separation budget: whatever is left between it
//! and the ~2048-bit noise floor is what `margin` can spend. Measured on the
//! host:
//!
//! ```text
//! coherent store (variants of one pattern, 100 bits of noise)
//!   n=4    radius=102   unrelated~2047   separation=1945
//!   n=64   radius=100   unrelated~2047   separation=1947
//!   n=256  radius=100   unrelated~2046   separation=1946
//!
//! diverse store (unrelated random vectors)
//!   n=8    radius=1527  unrelated~2050   separation=523
//!   n=32   radius=1835  unrelated~2049   separation=214
//!   n=256  radius=2039  unrelated~2046   separation=7
//! ```
//!
//! A coherent store does not degrade with size at all: enrolling 256 baseline
//! windows separates as well as enrolling 4. A diverse store collapses, because
//! bundling unrelated vectors converges on the noise floor and the bundle stops
//! resembling any single member.
//!
//! The degradation is fail-safe. As separation shrinks, `radius + margin`
//! swallows the whole range and the net simply stops rejecting: it becomes
//! useless, never wrong. The margin guarantee above holds throughout. So this
//! suits a baseline-of-normal detector (the intended use) and is inert on a
//! general-purpose store of unrelated patterns, which is the same distinction
//! the IVF index draws in `examples/ivf_eval.rs`.

use crate::hv::Hv;

/// One-compare "nothing close here" filter over a set of enrolled vectors.
pub struct RejectNet {
    net: Hv,
    radius: u32,
    margin: u32,
    len: usize,
}

impl RejectNet {
    /// Empty net with a noise tolerance of `margin` bits. Rejects nothing until
    /// [`rebuild`](Self::rebuild) is called.
    pub fn new(margin: u32) -> Self {
        RejectNet { net: Hv::zero(), radius: 0, margin, len: 0 }
    }

    /// Bundle `members` into the net and measure the rejection radius.
    /// Empty `members` resets to the empty state.
    pub fn rebuild(&mut self, members: &[Hv]) {
        self.len = members.len();
        if members.is_empty() {
            self.net = Hv::zero();
            self.radius = 0;
            return;
        }
        self.net = Hv::bundle(members);
        // Every member is within `radius` of the bundle by construction, which
        // is what makes the margin guarantee hold.
        let mut radius = 0u32;
        for m in members {
            let d = self.net.hamming(m);
            if d > radius {
                radius = d;
            }
        }
        self.radius = radius;
    }

    /// `true` when nothing enrolled is within `margin` bits of `query`, decided
    /// in a single `hamming`. Always `false` while empty.
    pub fn rejects(&self, query: &Hv) -> bool {
        // Saturating so an absurd margin means "never reject" rather than
        // wrapping into rejecting everything.
        self.len != 0 && self.net.hamming(query) > self.radius.saturating_add(self.margin)
    }

    /// Distance from `query` to the bundle. Exposed for threshold calibration.
    pub fn distance(&self, query: &Hv) -> u32 {
        self.net.hamming(query)
    }

    /// Largest member-to-bundle distance measured at the last `rebuild`.
    pub fn radius(&self) -> u32 {
        self.radius
    }

    /// Declared noise tolerance, in bits.
    pub fn margin(&self) -> u32 {
        self.margin
    }

    /// Vectors bundled at the last `rebuild`.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hv::{DIM_BITS, WORDS};

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

    /// `base` with `nbits` random bit positions flipped. Repeats can cancel, so
    /// the result is within `nbits` of `base`, not exactly at it.
    fn noisy(base: &Hv, nbits: usize, s: &mut u64) -> Hv {
        let mut h = base.clone();
        for _ in 0..nbits {
            let b = (xs(s) as usize) % DIM_BITS;
            h.0[b / 64] ^= 1u64 << (b % 64);
        }
        h
    }

    fn members(n: usize, s: &mut u64) -> impl Iterator<Item = Hv> + '_ {
        (0..n).map(move |_| rand_hv(s))
    }

    #[test]
    fn empty_net_rejects_nothing() {
        let mut s: u64 = 0x1234_5678_9abc_def1;
        let net = RejectNet::new(64);
        assert!(net.is_empty());
        assert_eq!(net.len(), 0);
        for _ in 0..32 {
            assert!(!net.rejects(&rand_hv(&mut s)));
        }
    }

    #[test]
    fn rebuild_with_no_members_stays_empty() {
        let mut s: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut net = RejectNet::new(64);
        net.rebuild(&[]);
        assert!(net.is_empty());
        assert!(!net.rejects(&rand_hv(&mut s)));
    }

    #[test]
    fn single_member_has_zero_radius() {
        let mut s: u64 = 0xdead_beef_0bad_f00d;
        let only = rand_hv(&mut s);
        let mut net = RejectNet::new(0);
        net.rebuild(core::slice::from_ref(&only));
        // bundle of one is that vector, so it sits exactly on the net.
        assert_eq!(net.radius(), 0);
        assert_eq!(net.distance(&only), 0);
        assert!(!net.rejects(&only));
    }

    #[test]
    fn enrolled_members_are_never_rejected() {
        let mut s: u64 = 0x0123_4567_89ab_cdef;
        for n in [1usize, 2, 3, 8, 33, 64] {
            let items: Vec<Hv> = members(n, &mut s).collect();
            let mut net = RejectNet::new(0);
            net.rebuild(&items);
            assert_eq!(net.len(), n);
            for m in &items {
                assert!(!net.rejects(m), "member rejected at n={}", n);
            }
        }
    }

    /// The triangle-inequality guarantee: within `margin` bits of any enrolled
    /// vector, rejection is structurally impossible.
    #[test]
    fn queries_within_margin_are_never_rejected() {
        let mut s: u64 = 0xfeed_face_cafe_0001;
        for margin in [0u32, 1, 16, 128, 400] {
            for n in [1usize, 4, 16, 64] {
                let items: Vec<Hv> = members(n, &mut s).collect();
                let mut net = RejectNet::new(margin);
                net.rebuild(&items);
                for _ in 0..24 {
                    let m = &items[(xs(&mut s) as usize) % n];
                    let e = (xs(&mut s) as usize) % (margin as usize + 1);
                    let q = noisy(m, e, &mut s);
                    assert!(
                        !net.rejects(&q),
                        "rejected a query {} bits from a member (margin={}, n={})",
                        e,
                        margin,
                        n
                    );
                }
            }
        }
    }

    #[test]
    fn unrelated_query_is_rejected_on_a_clustered_store() {
        let mut s: u64 = 0xabad_1dea_0000_0007;
        let base = rand_hv(&mut s);
        let items: Vec<Hv> = (0..8).map(|_| noisy(&base, 100, &mut s)).collect();
        let mut net = RejectNet::new(150);
        net.rebuild(&items);
        // unrelated vectors sit ~2048 bits out, far past radius + margin.
        for _ in 0..64 {
            assert!(net.rejects(&rand_hv(&mut s)));
        }
    }

    #[test]
    fn rebuild_is_deterministic() {
        let mut s: u64 = 0x5555_aaaa_5555_aaaa;
        let items: Vec<Hv> = members(16, &mut s).collect();
        let mut a = RejectNet::new(32);
        let mut b = RejectNet::new(32);
        a.rebuild(&items);
        b.rebuild(&items);
        let mut q = 0x1111_2222_3333_4444u64;
        for _ in 0..16 {
            let v = rand_hv(&mut q);
            assert_eq!(a.distance(&v), b.distance(&v));
        }
        assert_eq!(a.radius(), b.radius());
    }

    #[test]
    fn rebuild_replaces_previous_members() {
        let mut s: u64 = 0x7777_8888_9999_aaaa;
        let first: Vec<Hv> = members(4, &mut s).collect();
        let second: Vec<Hv> = members(4, &mut s).collect();
        let mut net = RejectNet::new(0);
        net.rebuild(&first);
        net.rebuild(&second);
        assert_eq!(net.len(), 4);
        for m in &second {
            assert!(!net.rejects(m));
        }
    }

    /// Coherence, not size, decides whether the net is useful. A diverse store
    /// degrades to rejecting nothing rather than to rejecting wrongly.
    #[test]
    fn diverse_store_fails_safe_while_coherent_store_still_rejects() {
        let mut s: u64 = 0x00c0_ffee_0000_002a;
        const MARGIN: u32 = 256;

        // 256 unrelated vectors: radius sits on the noise floor, so nothing is
        // ever rejected. Useless, but never wrong.
        let diverse: Vec<Hv> = members(256, &mut s).collect();
        let mut net = RejectNet::new(MARGIN);
        net.rebuild(&diverse);
        assert!(net.radius() > 1900, "expected radius near the noise floor");
        for _ in 0..200 {
            assert!(!net.rejects(&rand_hv(&mut s)), "diverse store must fail safe");
        }

        // 256 variants of one pattern: separation is unchanged by the size.
        let base = rand_hv(&mut s);
        let coherent: Vec<Hv> = (0..256).map(|_| noisy(&base, 100, &mut s)).collect();
        let mut net = RejectNet::new(MARGIN);
        net.rebuild(&coherent);
        assert!(net.radius() < 200, "coherent store should stay far from the floor");
        for _ in 0..200 {
            assert!(net.rejects(&rand_hv(&mut s)), "coherent store must still reject");
        }
        for m in &coherent {
            assert!(!net.rejects(m));
        }
    }
}
