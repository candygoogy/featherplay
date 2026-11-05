# Vendored: opus-decoder 0.1.1

This is the crate as published on crates.io, with one change. The workspace `Cargo.toml`
swaps it in with `[patch.crates-io]`. featherplay uses it to decode the Opus streams
(YouTube's format 251, Opus in WebM) behind symphonia's `Decoder` trait
(`crates/audio/src/opus.rs`); symphonia 0.5 has no Opus decoder of its own.

1. `src/celt/kiss_fft.rs`, `flat_fft_forward`: the forward FFT the CELT inverse MDCT runs
   (`src/celt/mdct.rs`) is `rustfft`'s, with one plan per size cached in a thread-local
   (marked `featherplay patch`). Upstream computes a naive O(N²) DFT that calls `sin_cos` in
   f64 inside its inner loop. `Cargo.toml` gains the dependency (also marked):

   ```toml
   [dependencies.rustfft]
   version = "=6.4.1"
   default-features = false
   ```

   `default-features = false` leaves out rustfft's SIMD back ends (`avx`, `sse`, `neon`),
   as the spike measured it; the scalar FFT is already far faster than needed.

## Why

As published, the decoder was too slow to play with: a 212 s YouTube track (format 251,
138 kbps) decoded in 45–57 s, 4–5× realtime, about 20–25 % of one core while playing.
Nearly all of it was `flat_fft_forward`.

Measured on 2026-11-05 (release build, the same machine under the same load from other
builds; the full numbers are in the project's notes):

| | 212 s track | Speed |
|---|---|---|
| Published | 45–57 s | 4–5× realtime |
| Patched | 0.65 s | ~325× realtime |

The output is unchanged in quality. Against ffmpeg's libopus decode of the same file, after
dropping the pre-skip (alignment offset 0):

| Decoder | SNR vs libopus | Peak difference | Samples off by more than 0.01 |
|---|---|---|---|
| Patched `opus-decoder` | 59.1 dB | 0.079 | 2,610 of 20.4 M |
| FFmpeg's built-in Opus decoder | 58.5 dB | 0.096 | 2,690 |

`decode_float` goes through i16 internally, so the output has 16-bit precision; that's fine
for playback.

Licence: unchanged, MIT OR Apache-2.0 (see `Cargo.toml`; the published crate ships no
licence files). It's a port of libopus, BSD 3-Clause (see `README.md`).

## Redo it on an upgrade

1. Copy the new `opus-decoder-<version>/` from the cargo registry
   (`$CARGO_HOME/registry/src/index.crates.io-*/`) over this folder, keeping this file.
   Delete `.cargo-ok`.
2. If `flat_fft_forward` (or whatever the inverse MDCT calls) is still a naive DFT, port the
   `featherplay patch` block in `src/celt/kiss_fft.rs` and the `rustfft` dependency (the git
   history of this folder has the diff).
3. Bump the `=x.y.z` pin in `crates/audio/Cargo.toml`, run the audio crate's tests, and
   measure the decode speed in a release build.

If upstream gets a fast FFT, delete this folder and the `[patch.crates-io]` entry. Sending
the fix upstream (TadeuszWolfGang/Rusopus) is the user's call.
