//! Every integration test of `integrations` in one test program, one module per
//! former test file, so the crate links one test executable instead of 4.
//! Run them with `cargo nextest run` (one process per test); see `.config/nextest.toml`.
#![allow(
    clippy::print_stderr,
    clippy::print_stdout,
    reason = "tests report skips and measurements"
)]

mod amuse;
mod discord;
mod hotkeys_parse;
mod local_api;
mod sponsorblock;
