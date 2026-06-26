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

## 6. IVF index: sub-linear nearest (host)

An HDC-native inverted-file index: centroid = `Hv::bundle` (majority), assign/probe = `Hv::hamming`,
k-means in Hamming. Measured vs the linear scan, N = 12,000:

| data | recall@1 | speedup |
|---|---|---|
| **clustered** (real sensor/embedding-like) | **100%** | **37×** (321 compares vs 12,000) |
| uniform-random (no structure) | ~2% | n/a (no index helps) |

On structured data (which real sensor and embedding data is), IVF finds the true nearest essentially
always, at ~√N cost, so the 278 ms PSRAM scan above would drop to **~7 ms**. On uniform-random data no
index helps: the honest curse-of-dimensionality boundary. `cargo run --release --example ivf_eval`.
