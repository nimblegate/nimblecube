# nimblecube-core

The engine: an integer-only, `#![no_std]`, alloc-free, zero-dependency hyperdimensional-computing core.

- **`Hv`** (4096-bit hypervector, `[u64; 64]`): `bind` (XOR), `bundle` (bitwise majority / superposition),
  `hamming` (popcount distance), `permute` (rotate). Word-parallel `bundle` and Harley-Seal `hamming`.
- **`FixedStore<N>`** (inline `[Hv; N]` associative memory): `insert` + nearest-by-Hamming with
  branch-and-bound early termination. No heap, no allocation; ties resolve to the first-inserted.
- **`FeatureEncoder<CH, L>`**: quantized integer features → one `Hv` (level-encode → bind-per-channel →
  bundle), so close inputs get close codes.

## Test

```bash
cargo test -p nimblecube-core
```

Includes property tests that verify each optimized core op (`bundle`, `nearest`, `hamming`) is
**bit-identical** to a retained scalar reference.

## Verify no_std (bare metal)

```bash
rustup target add thumbv7em-none-eabi
cargo build -p nimblecube-core --release --target thumbv7em-none-eabi
```

Compiles for a target where `std` does not exist; any `std`/alloc use would fail.

## Examples (host)

```bash
cargo run --release --example smart_sensor    # synthetic sensor anomaly path
cargo run --release --example ivf_eval         # IVF index recall/speed frontier
cargo run --release --example hamming_bench     # core-op throughput
```

See [`../BENCHMARKS.md`](../BENCHMARKS.md) for measured numbers (host + on-device).
