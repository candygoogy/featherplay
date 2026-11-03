//! Every integration test of `streams` in one test program, one module per
//! former test file, so the crate links one test executable instead of 3.
//! Run them with `cargo nextest run` (one process per test); see `.config/nextest.toml`.
#![allow(
    clippy::print_stderr,
    clippy::print_stdout,
    reason = "tests report skips and measurements"
)]

mod expiry;
mod live;
mod ytdlp_parse;
