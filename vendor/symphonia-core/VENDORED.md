# Vendored: symphonia-core 0.5.5

This is the crate as published on crates.io, with one change. The workspace `Cargo.toml`
swaps it in with `[patch.crates-io]`. It's under the MPL-2.0, as upstream; the changed file
keeps its licence header.

1. `src/dsp/fft.rs`, `fft_twiddle_table!`: each FFT twiddle table (64 to 65536 points) is
   built into a boxed slice on its first use, behind a `std::sync::OnceLock` (marked
   `featherplay patch`). Upstream declared each one as a `lazy_static` array. Every table
   is reachable from `fft_twiddle_factors`, so all eleven were linked, and a `lazy_static`
   array isn't all zeros (its `Option` tag and `Once` are stored with it), so it sat in the
   data section rather than in `.bss`: 524,592 bytes of the Linux binary, and about as much of
   the Windows exe (2026-11-13). The values are computed by the same code, so they're the
   same bits; the AAC decoder's IMDCT uses only the small ones.

`Cargo.toml` is upstream's: it still lists `lazy_static`, which nothing in the crate uses
now.

## Why

The release binaries have size budgets (`release.yml`). This was 0.5 MB of zeros in each.

## Updating

Before moving symphonia past 0.5.5, check whether upstream still keeps the tables in
`lazy_static` arrays. If it doesn't, drop this copy and its `[patch.crates-io]` line.
