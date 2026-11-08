//! Every integration test of `store` in one test program, one module per
//! former test file, so the crate links one test executable instead of 2.
//! Run them with `cargo nextest run` (one process per test); see `.config/nextest.toml`.
#![allow(
    clippy::print_stderr,
    clippy::print_stdout,
    reason = "tests report skips and measurements"
)]

mod home;
mod library_cache;
mod silence_cache;
mod store;
