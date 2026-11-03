//! Every integration test of `ytm` in one test program, one module per
//! former test file, so the crate links one test executable instead of 2.
//! Run them with `cargo nextest run` (one process per test); see `.config/nextest.toml`.
#![allow(
    clippy::print_stderr,
    clippy::print_stdout,
    reason = "tests report skips and measurements"
)]

mod convert;
mod lenient;
mod live;
mod suggestions;
