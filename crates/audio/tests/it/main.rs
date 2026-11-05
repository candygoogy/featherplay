//! Every integration test of `audio` in one test program, one module per
//! former test file, so the crate links one test executable instead of 5.
//! Run them with `cargo nextest run` (one process per test); see `.config/nextest.toml`.
#![allow(
    clippy::print_stderr,
    clippy::print_stdout,
    reason = "tests report skips and measurements"
)]

mod engine_local;
mod live;
mod play_probe;
mod resolve_steps;
mod tmp;
mod volume;
