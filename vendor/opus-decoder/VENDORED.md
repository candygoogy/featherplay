# Vendored: opus-decoder 0.1.1

This is the crate as published on crates.io, with two changes. The workspace `Cargo.toml`
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

   The FFTs are built by hand, not by rustfft's planner (2026-11-07): the planner can pick
   any of rustfft's algorithms for a size, so all of them were linked, 0.31 MB of the release
   exe. CELT's FFT sizes are 60, 120, 240 and 480 (the MDCTs of 240 to 1920 samples), a
   power of two times 15, so each is `GoodThomasAlgorithmSmall` over a 4-, 8-, 16- or
   32-point butterfly and a 15-point Good-Thomas of the 3- and 5-point ones. Any other size
   falls back to rustfft's `Dft`. Decoding the 212 s test track, the output differs from the
   planner's in 10,189 of 20.4 M samples, each by one 16-bit step (the planner's `RadixN`
   rounds differently); it's still 59.1 dB from libopus with the same peak difference, and
   decodes at the same speed (~110–130× realtime at opt-level "z", both, under load).

2. `src/celt/vq.rs`, `extract_collapse_mask` and `PvqDecodeInfo`: the collapse mask is a
   `u32`, as libopus's `unsigned` (marked `featherplay patch`); the callers already carried
   it as one. Upstream built it in a `u8`, but a band can have up to 16 blocks (a transient
   frame whose time resolution is raised further), so `1 << i` overflowed: a panic in any
   build with overflow checks (the tests), and in a release build bits 8-15 folded onto
   0-7. The real 251 test track hit it 595 times (2026-11-05); with the fix it decodes in a
   debug build, still 59.1 dB from libopus.

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
   Delete `.cargo-ok` and `Cargo.lock` (a path dependency's lock file is never read).
2. If `flat_fft_forward` (or whatever the inverse MDCT calls) is still a naive DFT, port the
   `featherplay patch` block in `src/celt/kiss_fft.rs` and the `rustfft` dependency (the git
   history of this folder has the diff). If the inverse MDCT's sizes changed, extend `plan`
   there so none of them lands on the slow `Dft`.
   Keep change 2 unless `extract_collapse_mask` already returns a 32-bit mask.
3. Bump the `=x.y.z` pin in `crates/audio/Cargo.toml`, run the audio crate's tests, and
   measure the decode speed in a release build.

If upstream gets a fast FFT, delete this folder and the `[patch.crates-io]` entry. Sending
the fix upstream (TadeuszWolfGang/Rusopus) is the user's call.
