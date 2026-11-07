//! Deflates yt-dlp's EJS solver library (`ejs/yt.solver.lib.min.js`, 152 KB of minified JS)
//! into `OUT_DIR`, a third of its size in the exe. `native::ejs_lib` inflates it for the
//! rare run that preprocesses a new player.
#![allow(
    clippy::print_stdout,
    reason = "cargo reads a build script's instructions from its stdout"
)]

use std::io::Write;
use std::path::Path;

use flate2::Compression;
use flate2::write::DeflateEncoder;

fn main() {
    let lib = "ejs/yt.solver.lib.min.js";
    println!("cargo:rerun-if-changed={lib}");
    let js = std::fs::read(lib).unwrap_or_else(|e| panic!("read {lib}: {e}"));
    let mut deflate = DeflateEncoder::new(Vec::new(), Compression::best());
    deflate.write_all(&js).expect("deflate the EJS library");
    let out = std::env::var("OUT_DIR").expect("OUT_DIR");
    std::fs::write(
        Path::new(&out).join("yt.solver.lib.min.js.deflate"),
        deflate.finish().expect("deflate the EJS library"),
    )
    .expect("write the deflated EJS library");
}
