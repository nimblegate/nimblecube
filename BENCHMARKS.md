# Benchmarks

All numbers are **measured**, not estimated: on the host for the core ops and the index, and on a real
**ESP32-S3 @ 240 MHz** for the on-device figures. Deterministic, seeded, with `black_box` and a printed
checksum so nothing is optimized away.

## 1. Footprint

| | size |
|---|---|
| one hypervector (`Hv`) | **512 B** (4096 bits) |
| one stored item | 516 B (vector + `u32` id) |
| `FixedStore<64>` | **33 KB** |
| bare-metal Cortex-M image (insert + nearest + hamming + vector table + panic handler) | **2.9 KB flash (2908 B), 0 static RAM** |

The code fits *any* MCU; only the store RAM (`N × 516 B`) scales.

## 2. Core-op throughput (host, x86-64)

`FixedStore::nearest` over N=1024, scalar vs auto-vectorized. The default x86-64 baseline lacks
`POPCNT`, so it lowers to a software popcount; enabling hardware `POPCNT` + AVX2 is a free win (same
integer result, identical checksum):

| build | ns / comparison | speedup |
|---|---|---|
| default (software popcount) | 63.9 | 1.00× |
| `target-feature=+popcnt` | 37.1 | 1.72× |
| `target-cpu=native` (AVX2 + POPCNT) | **25.8** | **2.47×** |

`cargo run --release --example hamming_bench` (set `RUSTFLAGS` for the variants).

## 3. On-device throughput (ESP32-S3 @ 240 MHz)

The same `nimblecube-core` ops on the real chip. The Xtensa LX7 has **no scalar `POPCNT`** (and the Rust/LLVM
backend doesn't autovectorize to its 128-bit SIMD extensions), so `count_ones` is a software popcount -
hence the gap to a desktop with hardware POPCNT.

| op | time |
|---|---|
| `Hv::hamming` | **7.8 µs** |
| `FeatureEncoder::encode` | **103 µs** |
| `FixedStore::nearest` (matched query) | **22 µs** |
| `FixedStore::nearest` (random scan of 64) | 749 µs |
| **full sensor → encode → nearest cycle (matched)** | **≈ 125 µs** |

Ample for sensor anomaly detection (a few Hz). `cd nimblecube-esp32 && cargo run --release --bin bench`.

## 4. Optimizations (all bit-identical)

Each core-op rewrite is verified **byte-for-byte identical** to the original: a property test against the
retained scalar version on the host, *and* an unchanged on-device output checksum:

| op | change | speedup |
|---|---|---|
| `bundle` | bit-serial majority → **word-parallel bit-sliced majority** | **12.7×** (encode 1.31 ms → 103 µs) |
| `nearest` | full scan → **branch-and-bound early termination** | **33×** matched (910 µs → 22 µs)¹ |
| `hamming` | per-word `count_ones` → **Harley-Seal carry-save popcount** | 1.72× (13.4 → 7.8 µs) |

¹ Data-dependent: a clean reading that matches a stored pattern bails after one word; a random/anomaly
query (no close match) prunes less.

The matched detection path went from ~2.2 ms to ~125 µs (~18×) across these.

## 5. Capacity (ESP32-S3-N16R8, 8 MB octal PSRAM)

| store | items | nearest |
|---|---|---|
| internal SRAM (`FixedStore`) | ~256-512 (fast) | ~22 µs matched |
| PSRAM (`Vec<Hv>`) | **~12,000** (~6.1 MB) | 278 ms random / 11 ms matched |

PSRAM buys **capacity, not speed**: it has access latency internal SRAM lacks, so a linear scan over
thousands of vectors is hundreds of ms. That motivates the index below.
`cd nimblecube-esp32 && cargo run --release --bin psram_bench`.

The 278 ms is a *random* query against *random* items, where early termination never fires. On
clustered data with a query near a cluster it prunes hard: the same 12,000-item PSRAM scan measures
**59.7 ms** (§8). That, not 278 ms, is the baseline an index has to beat on realistic data.

## 6. IVF index: sub-linear nearest (host)

An HDC-native inverted-file index: centroid = `Hv::bundle` (majority), assign/probe = `Hv::hamming`,
k-means in Hamming. Measured vs the linear scan, N = 12,000:

| data | recall@1 | speedup |
|---|---|---|
| **clustered** (real sensor/embedding-like) | **100%** | **37×** (321 compares vs 12,000) |
| uniform-random (no structure) | ~2% | n/a (no index helps) |

On structured data (which real sensor and embedding data is), IVF finds the true nearest essentially
always, at ~√N cost. On uniform-random data no index helps: the honest curse-of-dimensionality
boundary. `cargo run --release --example ivf_eval`.

The 37× is a **compare-count ratio, not wall-clock**. §8 measures both for a comparable index and
finds compare count over-predicts on-chip speedup by ~4×, because an index trades many sequential
PSRAM reads for few random ones. Treat every compare-count figure here as an upper bound.

## 7. RejectNet: one-compare rejection (ESP32-S3)

`nearest` terminates early only when a close match tightens the bound, so an *anomalous* query prunes
nothing and pays the full scan. The linear scan is therefore slowest exactly on the path a detector
exists to catch. `RejectNet` bundles every enrolled vector into one `Hv`, so a single `hamming`
answers "is anything here close at all?" for the whole store.

Measured on-chip, N = 64 coherent baselines (spread 100 bits, margin 150):

| query | `nearest` | reject + scan | gain |
|---|---|---|---|
| **anomaly** (nothing close) | 765 µs | **7 µs** | **109×** |
| normal (a fresh clean reading) | 742 µs | 749 µs | ~1% cost |

Correct on 8/8 anomalies and 0/8 normals on-device. The margin is a guarantee, not a tuning knob:
Hamming is a metric, so any query within `margin` bits of an enrolled vector is *structurally* unable
to be rejected. Usefulness depends on how coherent the store is, not how large it is: 256 enrolled
baselines separate as well as 4 (separation ~1946 bits), while 256 *unrelated* vectors collapse to 7
bits and the net simply stops rejecting. That degradation is fail-safe, never wrong.
`cd nimblecube-esp32 && cargo run --release --bin reject_bench`.

## 8. Net tree: an incremental index, host and on-chip

Cells are `Hv::bundle` superpositions, assigned greedily as items arrive. No k-means, no training
pass, so enrollment stays one-shot. Host eval, N = 12,000, 200 clusters, 500 queries:

| index | recall@1 | compares | vs linear |
|---|---|---|---|
| **net tree** (cap 64) | **100%** | **260** | 46× |
| IVF (§6) | 100% | 321 | 37× |
| insertion-order cells (control) | 1.8% | 249 | n/a (wrong answers) |
| net tree, uniform-random data | 100% | 12,001 | 1.0× |

The control row matters: cells filled in arrival order score 1.8%, so the greedy assignment is doing
all the work. Enrollment costs ~2.75M compare-equivalents against IVF's ~12M, and can be done one
item at a time. On unstructured data the tree **fails safe**, degenerating to one cell per item so it
stays correct and merely stops helping, where IVF returns wrong answers at ~2% recall. The cost of
that is O(n²) enrollment when nothing clusters.

Then the same index on real hardware, N = 12,000 in PSRAM:

| | per query | recall |
|---|---|---|
| linear scan | 59,652 µs | 100% by definition |
| **net tree** (nprobe 1) | **5,401 µs** | **20/20** |
| net tree (nprobe 2) | 6,685 µs | 20/20 |

**Recall transferred; the speedup did not.** 46× of compare-count became **11×** of wall-clock,
because per-compare cost is not constant: the linear scan streams PSRAM sequentially (4.97 µs per
compare) while the tree fetches scattered cells and members by index (20.8 µs per compare, a 4.2×
penalty from access pattern alone). Building the index is 73 s for a bulk load of 12,000, about 6 ms
per insert. `cd nimblecube-esp32 && cargo run --release --bin net_tree_bench`.

Recall is the number to watch rather than speed. The detector's decision is `d > threshold`, so a
missed nearest returns an inflated `d` and shows up as a **false alarm**. At 100% recall the index is
behaviorally identical to a full scan, which is what makes it safe to substitute.
