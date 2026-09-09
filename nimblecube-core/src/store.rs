//! Fixed-capacity, heap-free associative memory over `Hv` (nearest by Hamming).

use crate::hv::{Hv, WORDS};

/// Returned by `insert` when the store is at capacity.
#[derive(Debug, PartialEq, Eq)]
pub struct StoreFull;

/// Up to `N` hypervectors with `u32` id payloads; nearest-match by Hamming distance.
/// Inline fixed array - no heap, no allocation.
pub struct FixedStore<const N: usize> {
    slots: [Hv; N],
    ids: [u32; N],
    len: usize,
}

impl<const N: usize> Default for FixedStore<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> FixedStore<N> {
    /// Empty store; all slots zero-initialized (no heap).
    pub fn new() -> Self {
        FixedStore { slots: core::array::from_fn(|_| Hv::zero()), ids: [0u32; N], len: 0 }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn capacity(&self) -> usize {
        N
    }

    /// The enrolled vectors, in insertion order. Empty slots are not included,
    /// so this feeds `RejectNet::rebuild` directly.
    pub fn slots(&self) -> &[Hv] {
        &self.slots[..self.len]
    }

    /// Append `(hv, id)`. `Err(StoreFull)` once `len == N`.
    pub fn insert(&mut self, hv: Hv, id: u32) -> Result<(), StoreFull> {
        if self.len >= N {
            return Err(StoreFull);
        }
        self.slots[self.len] = hv;
        self.ids[self.len] = id;
        self.len += 1;
        Ok(())
    }

    /// Nearest stored vector by Hamming distance: `(id, distance)`. Ties resolve to the
    /// first-inserted. `None` if empty.
    pub fn nearest(&self, query: &Hv) -> Option<(u32, u32)> {
        if self.len == 0 {
            return None;
        }
        // Entry 0 full; later entries accumulate per word and bail once they reach
        // the best-so-far (can't become a strict new minimum). Bit-identical to the
        // full-scan version incl. the first-inserted tie-break.
        let mut best_id = self.ids[0];
        let mut best_d = self.slots[0].hamming(query);
        for i in 1..self.len {
            let s = &self.slots[i].0;
            let q = &query.0;
            let mut d = 0u32;
            let mut w = 0;
            while w < WORDS {
                d += (s[w] ^ q[w]).count_ones();
                if d >= best_d {
                    break;
                }
                w += 1;
            }
            if w == WORDS {
                best_d = d;
                best_id = self.ids[i];
            }
        }
        Some((best_id, best_d))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hv::{Hv, WORDS};

    fn ones(n: usize) -> Hv {
        let mut w = [0u64; WORDS];
        for i in 0..n {
            w[i / 64] |= 1u64 << (i % 64);
        }
        Hv(w)
    }

    #[test]
    fn new_is_empty() {
        let s = FixedStore::<4>::new();
        assert_eq!(s.len(), 0);
        assert!(s.is_empty());
        assert_eq!(s.capacity(), 4);
    }

    #[test]
    fn insert_fills_then_reports_full() {
        let mut s = FixedStore::<2>::new();
        assert_eq!(s.insert(Hv::zero(), 10), Ok(()));
        assert_eq!(s.insert(Hv::zero(), 11), Ok(()));
        assert_eq!(s.len(), 2);
        assert_eq!(s.insert(Hv::zero(), 12), Err(StoreFull));
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn slots_exposes_only_enrolled_prefix() {
        let mut s = FixedStore::<4>::new();
        assert!(s.slots().is_empty());
        s.insert(ones(3), 1).unwrap();
        s.insert(ones(9), 2).unwrap();
        assert_eq!(s.slots().len(), 2);
        assert_eq!(s.slots()[0], ones(3));
        assert_eq!(s.slots()[1], ones(9));
    }

    #[test]
    fn nearest_on_empty_is_none() {
        let s = FixedStore::<4>::new();
        assert_eq!(s.nearest(&Hv::zero()), None);
    }

    #[test]
    fn nearest_returns_min_hamming_id() {
        let mut s = FixedStore::<4>::new();
        s.insert(ones(0), 0).unwrap();
        s.insert(ones(100), 1).unwrap();
        s.insert(ones(10), 2).unwrap();
        assert_eq!(s.nearest(&Hv::zero()), Some((0, 0)));
        assert_eq!(s.nearest(&ones(10)), Some((2, 0)));
    }

    #[test]
    fn nearest_tie_breaks_to_first_inserted() {
        let mut s = FixedStore::<4>::new();
        let mut w1 = [0u64; WORDS];
        for i in 0..5 {
            w1[0] |= 1u64 << i; // bits 0..5
        }
        let mut w2 = [0u64; WORDS];
        for i in 10..15 {
            w2[0] |= 1u64 << i; // bits 10..15 (different pattern, also 5 ones)
        }
        s.insert(Hv(w1), 100).unwrap();
        s.insert(Hv(w2), 200).unwrap();
        // both are distance 5 from zero; first-inserted id 100 wins the tie
        assert_eq!(s.nearest(&Hv::zero()), Some((100, 5)));
    }

    #[test]
    fn footprint_is_exact() {
        use core::mem::size_of;
        assert_eq!(size_of::<Hv>(), 512);
        assert_eq!(
            size_of::<FixedStore<64>>(),
            64 * size_of::<Hv>() + 64 * size_of::<u32>() + size_of::<usize>()
        );
    }

    // Reference: the pre-optimization nearest logic over a plain slice (ids = index).
    fn oracle(items: &[Hv], query: &Hv) -> Option<(u32, u32)> {
        let mut best: Option<(u32, u32)> = None;
        for (i, h) in items.iter().enumerate() {
            let d = h.hamming(query);
            match best {
                Some((_, bd)) if d >= bd => {}
                _ => best = Some((i as u32, d)),
            }
        }
        best
    }

    fn xs_n(s: &mut u64) -> u64 {
        *s ^= *s << 13;
        *s ^= *s >> 7;
        *s ^= *s << 17;
        *s
    }
    fn rand_hv_n(s: &mut u64) -> Hv {
        let mut w = [0u64; WORDS];
        for x in w.iter_mut() {
            *x = xs_n(s);
        }
        Hv(w)
    }

    fn check_nearest<const N: usize>(s: &mut u64) {
        for _ in 0..40 {
            let cnt = (xs_n(s) as usize) % (N + 1);
            let mut st: FixedStore<N> = FixedStore::new();
            let mut items: Vec<Hv> = Vec::new();
            for k in 0..cnt {
                let hv = rand_hv_n(s);
                items.push(hv.clone());
                st.insert(hv, k as u32).ok();
            }
            let q = rand_hv_n(s);
            assert_eq!(st.nearest(&q), oracle(&items, &q), "random N={} cnt={}", N, cnt);
            if cnt > 0 {
                let idx = (xs_n(s) as usize) % cnt;
                let q = items[idx].clone();
                assert_eq!(st.nearest(&q), oracle(&items, &q), "exact N={} cnt={} idx={}", N, cnt, idx);
            }
        }
    }

    #[test]
    fn nearest_matches_oracle() {
        let mut s: u64 = 0x00C0_FFEE_1234_5678;
        check_nearest::<1>(&mut s);
        check_nearest::<2>(&mut s);
        check_nearest::<3>(&mut s);
        check_nearest::<8>(&mut s);
        check_nearest::<32>(&mut s);
        check_nearest::<64>(&mut s);
    }

    #[test]
    fn nearest_tie_keeps_first_inserted() {
        let mut s: u64 = 7;
        let dup = rand_hv_n(&mut s);
        let mut st: FixedStore<4> = FixedStore::new();
        st.insert(dup.clone(), 100).ok();
        st.insert(dup.clone(), 200).ok();
        st.insert(rand_hv_n(&mut s), 300).ok();
        assert_eq!(st.nearest(&dup), Some((100, 0)));
    }
}
