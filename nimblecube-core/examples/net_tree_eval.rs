//! Net-tree eval: an HDC-native index whose cells are `Hv::bundle` superpositions,
//! built incrementally with no training pass. Measures recall@1, compare count,
//! enrollment cost, fallback economics, and whether self-repair makes the miss
//! rate decay with use. Host/std, deterministic.
//!   cargo run --release --example net_tree_eval
//!
//! Same generators and sizes as `ivf_eval.rs`, so rows compare directly.
//!
//! Measured 2026-09-09, n=12000, 200 clusters, flip=200, 500 queries:
//!
//! ```text
//! clustered, greedy cap=64 : 100% recall @ nprobe=1, 260 compares, 46.2x
//! clustered, IVF (existing): 100% recall,             321 compares, 37x
//! clustered, in-order ctrl :  1.8% recall  <- the assignment rule does all the work
//! uniform,   greedy        : 100% recall,           12001 compares, 1.0x
//! uniform,   IVF (existing):   ~2% recall
//! enrollment: greedy 2.75M compare-equivalents vs IVF ~12M (200 x 12000 x 5 iters)
//! ```
//!
//! Two ideas measured and rejected, both at cap=32 where 37.2% of queries miss:
//!
//! - **Distance-threshold fallback does not work.** Returned distance on hits
//!   spans 328-358 and on misses 342-364. The ranges overlap, so no threshold
//!   separates a right answer from a wrong one. A miss still lands inside the
//!   correct cluster, just not on its nearest member, so it looks identical.
//! - **Self-repair does not converge.** Duplicating the true nearest into the
//!   wrongly chosen cell left the miss rate flat across five batches
//!   (38/34/33/33/37%). The cause is not a few misfiled items, it is a cluster
//!   split across two cells, and one duplicate per miss cannot cover the 12000
//!   possible answers.
//!
//! `nprobe=2` fixes 100% of those misses for 460 compares. Widening the probe
//! beats both detecting and repairing, and its cost is deterministic, which
//! matters more than average cost on a device with a deadline.

use nimblecube_core::hv::{Hv, DIM_BITS, WORDS};

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
fn noisy(base: &Hv, nbits: usize, s: &mut u64) -> Hv {
    let mut h = base.clone();
    for _ in 0..nbits {
        let b = (xs(s) as usize) % DIM_BITS;
        h.0[b / 64] ^= 1u64 << (b % 64);
    }
    h
}

/// Ground truth: nearest distance and its id.
fn linear_nearest(items: &[Hv], q: &Hv) -> (u32, u32) {
    let mut bd = u32::MAX;
    let mut bi = 0u32;
    for (i, h) in items.iter().enumerate() {
        let d = h.hamming(q);
        if d < bd {
            bd = d;
            bi = i as u32;
        }
    }
    (bd, bi)
}

/// Cells are bundles. An item may appear in more than one cell, which is what
/// lets repair add reachability without ever removing it.
struct NetTree {
    nets: Vec<Hv>,
    members: Vec<Vec<u32>>,
    cap: usize,
    join_radius: u32,
}

impl NetTree {
    fn rebundle(&mut self, items: &[Hv], c: usize) {
        let mems: Vec<Hv> = self.members[c].iter().map(|&id| items[id as usize].clone()).collect();
        self.nets[c] = Hv::bundle(&mems);
    }

    /// Greedy incremental insert. Returns work done, in compare-equivalents:
    /// one per net probed, plus one per member touched by the re-bundle.
    fn insert(&mut self, items: &[Hv], id: u32) -> u64 {
        let mut work = self.nets.len() as u64;
        let mut best = usize::MAX;
        let mut bd = u32::MAX;
        for (c, net) in self.nets.iter().enumerate() {
            if self.members[c].len() >= self.cap {
                continue;
            }
            let d = net.hamming(&items[id as usize]);
            if d < bd {
                bd = d;
                best = c;
            }
        }
        if best == usize::MAX || bd > self.join_radius {
            self.nets.push(items[id as usize].clone());
            self.members.push(vec![id]);
            return work;
        }
        self.members[best].push(id);
        work += self.members[best].len() as u64;
        self.rebundle(items, best);
        work
    }

    /// Probe the `nprobe` nearest nets. Returns (best distance, compares, ranked nets).
    fn query(&self, items: &[Hv], q: &Hv, nprobe: usize) -> (u32, u64, Vec<usize>) {
        let mut nd: Vec<(u32, usize)> =
            self.nets.iter().enumerate().map(|(c, net)| (net.hamming(q), c)).collect();
        nd.sort_by_key(|x| x.0);
        let mut compares = self.nets.len() as u64;
        let mut bd = u32::MAX;
        for &(_, c) in nd.iter().take(nprobe.min(nd.len())) {
            for &id in &self.members[c] {
                let d = items[id as usize].hamming(q);
                compares += 1;
                if d < bd {
                    bd = d;
                }
            }
        }
        (bd, compares, nd.iter().map(|x| x.1).collect())
    }

    fn total_members(&self) -> usize {
        self.members.iter().map(|m| m.len()).sum()
    }
}

fn build_greedy(items: &[Hv], cap: usize, join_radius: u32) -> (NetTree, u64) {
    let mut t = NetTree { nets: Vec::new(), members: Vec::new(), cap, join_radius };
    let mut work = 0u64;
    for i in 0..items.len() {
        work += t.insert(items, i as u32);
    }
    (t, work)
}

/// Control: cells filled in arrival order, no assignment logic at all. If this
/// scores as well as greedy, the assignment rule is buying nothing.
fn build_inorder(items: &[Hv], cap: usize) -> (NetTree, u64) {
    let mut t = NetTree { nets: Vec::new(), members: Vec::new(), cap, join_radius: u32::MAX };
    for (i, _) in items.iter().enumerate() {
        if t.members.last().map(|m| m.len() >= cap).unwrap_or(true) {
            t.members.push(Vec::new());
            t.nets.push(Hv::zero());
        }
        t.members.last_mut().unwrap().push(i as u32);
    }
    let mut work = 0u64;
    for c in 0..t.nets.len() {
        work += t.members[c].len() as u64;
        t.rebundle(items, c);
    }
    (t, work)
}

struct Truth {
    d: Vec<u32>,
    id: Vec<u32>,
}

fn truth_of(items: &[Hv], queries: &[Hv]) -> Truth {
    let mut d = Vec::with_capacity(queries.len());
    let mut id = Vec::with_capacity(queries.len());
    for q in queries {
        let (bd, bi) = linear_nearest(items, q);
        d.push(bd);
        id.push(bi);
    }
    Truth { d, id }
}

/// Stage 1a: recall and cost. The kill gate.
fn run_recall(items: &[Hv], queries: &[Hv], truth: &Truth, label: &str) {
    println!("{}:", label);
    println!(
        "  {:<20} {:>5} {:>7} {:>9} {:>8} {:>13} {:>10}",
        "variant", "cells", "nprobe", "recall@1", "speedup", "avg_compares", "enroll"
    );
    let n = items.len() as f64;
    for &cap in &[32usize, 64, 128] {
        let (greedy, gwork) = build_greedy(items, cap, (DIM_BITS / 4) as u32);
        let (inorder, iwork) = build_inorder(items, cap);
        for (name, tree, work) in
            [("greedy", &greedy, gwork), ("in-order (control)", &inorder, iwork)]
        {
            for &nprobe in &[1usize, 2, 4] {
                let mut hit = 0usize;
                let mut total = 0u64;
                for (qi, q) in queries.iter().enumerate() {
                    let (bd, cmp, _) = tree.query(items, q, nprobe);
                    if bd == truth.d[qi] {
                        hit += 1;
                    }
                    total += cmp;
                }
                let avg = total as f64 / queries.len() as f64;
                println!(
                    "  {:<20} {:>5} {:>7} {:>8.1}% {:>7.1}x {:>13.0} {:>10}",
                    name,
                    tree.nets.len(),
                    nprobe,
                    100.0 * hit as f64 / queries.len() as f64,
                    n / avg,
                    avg,
                    work
                );
            }
        }
    }
    println!();
}

/// Stage 1a instrumentation: can a distance threshold tell a good answer from a
/// wrong one, and would probing one more cell have fixed the misses anyway?
fn run_fallback(items: &[Hv], queries: &[Hv], truth: &Truth, cap: usize) {
    let (tree, _) = build_greedy(items, cap, (DIM_BITS / 4) as u32);
    let mut hit_d: Vec<u32> = Vec::new();
    let mut miss_d: Vec<u32> = Vec::new();
    let mut miss_in_2nd = 0usize;
    for (qi, q) in queries.iter().enumerate() {
        let (bd, _, ranked) = tree.query(items, q, 1);
        if bd == truth.d[qi] {
            hit_d.push(bd);
        } else {
            miss_d.push(bd);
            if ranked.len() > 1 && tree.members[ranked[1]].contains(&truth.id[qi]) {
                miss_in_2nd += 1;
            }
        }
    }
    hit_d.sort_unstable();
    miss_d.sort_unstable();
    let pct = |v: &Vec<u32>, p: f64| -> i64 {
        if v.is_empty() {
            -1
        } else {
            v[((v.len() - 1) as f64 * p) as usize] as i64
        }
    };
    let p = 100.0 * miss_d.len() as f64 / queries.len() as f64;
    println!("fallback analysis (cap={}, nprobe=1):", cap);
    println!("  miss rate p           : {:.1}%  ({} of {})", p, miss_d.len(), queries.len());
    println!(
        "  returned d on hits    : min {} median {} max {}",
        pct(&hit_d, 0.0),
        pct(&hit_d, 0.5),
        pct(&hit_d, 1.0)
    );
    println!(
        "  returned d on misses  : min {} median {} max {}",
        pct(&miss_d, 0.0),
        pct(&miss_d, 0.5),
        pct(&miss_d, 1.0)
    );
    let verdict = if miss_d.is_empty() {
        "n/a (no misses to detect)"
    } else if hit_d.is_empty() || miss_d[0] as i64 > pct(&hit_d, 1.0) {
        "YES (ranges disjoint)"
    } else {
        "NO (ranges overlap)"
    };
    println!("  threshold separable   : {}", verdict);
    if !miss_d.is_empty() {
        println!(
            "  misses fixed by nprobe=2: {:.0}%  ({} of {})",
            100.0 * miss_in_2nd as f64 / miss_d.len() as f64,
            miss_in_2nd,
            miss_d.len()
        );
    }
    let n = items.len() as f64;
    let base = tree.nets.len() as f64 + cap as f64;
    println!(
        "  expected compares w/ full-scan fallback: {:.0}  ({:.1}x)",
        base + p / 100.0 * n,
        n / (base + p / 100.0 * n)
    );
    println!();
}

/// Stage 1b: does learning from a fallback make the miss rate decay?
///
/// On a miss the full scan has already found the true nearest, so its id is free.
/// Add it to the cell we wrongly descended into, leaving it in its original cell
/// too. That only ever adds reachability. Overlap is what hard assignment cannot do.
fn run_repair(items: &[Hv], queries: &[Hv], truth: &Truth, cap: usize, batches: usize) {
    // Repair needs headroom above the insert cap. A cell at capacity is precisely
    // the cell that splits a cluster and causes the miss, so gating repair on the
    // insert cap would block it exactly when it is needed.
    let repair_cap = cap * 2;
    let (mut tree, _) = build_greedy(items, cap, (DIM_BITS / 4) as u32);
    let per = queries.len() / batches;
    println!("self-repair (cap={}, nprobe=1, {} batches of {}):", cap, batches, per);
    println!("  {:>5} {:>10} {:>12} {:>12}", "batch", "miss rate", "duplicates", "cells");
    for b in 0..batches {
        let mut miss = 0usize;
        let mut added = 0usize;
        for (qi, query) in queries.iter().enumerate().skip(b * per).take(per) {
            let (bd, _, ranked) = tree.query(items, query, 1);
            if bd != truth.d[qi] {
                miss += 1;
                let c = ranked[0];
                if tree.members[c].len() < repair_cap && !tree.members[c].contains(&truth.id[qi]) {
                    tree.members[c].push(truth.id[qi]);
                    tree.rebundle(items, c);
                    added += 1;
                }
            }
        }
        println!(
            "  {:>5} {:>9.1}% {:>12} {:>12}",
            b,
            100.0 * miss as f64 / per as f64,
            added,
            tree.nets.len()
        );
    }
    let biggest = tree.members.iter().map(|m| m.len()).max().unwrap_or(0);
    println!(
        "  total stored refs: {} (items: {}), largest cell: {} (insert cap {}, repair cap {})",
        tree.total_members(),
        items.len(),
        biggest,
        cap,
        repair_cap
    );
    println!();
}

fn main() {
    let mut s: u64 = 0x000A_11CE_5EED;
    let n = 12000usize;
    let g = 200usize;
    let flip = 200usize;
    let q = 500usize;

    let bases: Vec<Hv> = (0..g).map(|_| rand_hv(&mut s)).collect();
    let items: Vec<Hv> = (0..n).map(|i| noisy(&bases[i % g], flip, &mut s)).collect();
    let queries: Vec<Hv> = (0..q)
        .map(|_| {
            let b = (xs(&mut s) as usize) % g;
            noisy(&bases[b], flip, &mut s)
        })
        .collect();
    let rand_items: Vec<Hv> = (0..n).map(|_| rand_hv(&mut s)).collect();
    let rand_queries: Vec<Hv> = (0..q).map(|_| rand_hv(&mut s)).collect();

    println!("net_tree_eval  n={} queries={} clusters={} flip={}", n, q, g, flip);
    println!("IVF bar to beat (from ivf_eval): 100% recall at 321 avg compares\n");

    let truth = truth_of(&items, &queries);
    run_recall(&items, &queries, &truth, "clustered");
    // cap=64 holds a whole cluster, so nothing misses. cap=32 splits every
    // cluster in two, which is where fallback and repair actually get exercised.
    run_fallback(&items, &queries, &truth, 64);
    run_fallback(&items, &queries, &truth, 32);
    run_repair(&items, &queries, &truth, 32, 5);

    let rtruth = truth_of(&rand_items, &rand_queries);
    run_recall(&rand_items, &rand_queries, &rtruth, "uniform-random (no structure expected)");
}
