//! Renders the colour Tri-vane for the tray icon into raw RGBA, at the small-icon sizes
//! Windows uses from 100 % to 300 % scaling: `tri-vane*.svg` for a dark taskbar and
//! `tri-vane-light*.svg` (the play on a crimson ring) for a light one, the same drawings as
//! the window and taskbar icon. Every size is drawn from a vector at that size, never scaled
//! from another: 16 px from the 16 px drawing, 20 and 24 px from the 24 px one, 32 px and up
//! from the master. `icons::tray_icon` picks one at runtime.
#![allow(
    clippy::print_stdout,
    reason = "cargo reads a build script's instructions from its stdout"
)]

use std::path::Path;

/// Must match `icons::TRAY_SIZES`; each with the drawing it's rendered from.
const SIZES: [(u32, &str); 7] = [
    (16, "-16"),
    (20, "-24"),
    (24, "-24"),
    (32, ""),
    (40, ""),
    (48, ""),
    (64, ""),
];

fn main() {
    let logo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/featherplay/ui/logo");
    let out = std::env::var("OUT_DIR").expect("OUT_DIR");
    for (theme, name) in [("dark", "tri-vane"), ("light", "tri-vane-light")] {
        let mut all = Vec::new();
        for (size, drawing) in SIZES {
            let svg = logo.join(format!("{name}{drawing}.svg"));
            println!("cargo:rerun-if-changed={}", svg.display());
            let data =
                std::fs::read(&svg).unwrap_or_else(|e| panic!("read {}: {e}", svg.display()));
            let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default())
                .unwrap_or_else(|e| panic!("parse {}: {e}", svg.display()));
            let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).expect("pixmap");
            let scale = size as f32 / tree.size().width();
            resvg::render(
                &tree,
                resvg::tiny_skia::Transform::from_scale(scale, scale),
                &mut pixmap.as_mut(),
            );
            // tiny-skia keeps premultiplied alpha; the tray wants straight alpha.
            for px in pixmap.pixels() {
                let c = px.demultiply();
                all.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
            }
        }
        std::fs::write(Path::new(&out).join(format!("tray-{theme}.rgba")), all)
            .expect("write tray icon");
    }
}
