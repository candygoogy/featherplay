//! Compiles the Slint UI, renders the Tri-vane logo into `featherplay.ico` and its light
//! variant into `featherplay-light.ico` in `OUT_DIR` (`native::set_icon` picks one at runtime
//! by the taskbar's theme) and embeds the dark `.ico` into the exe as its icon resource,
//! which Explorer shows. Both logos also go to `installer-small{,-light}.png` and
//! `installer-panel{,-light}.png`, the installer's wizard images.
//!
//! The translations in `po/<lang>/LC_MESSAGES/featherplay.po` are bundled twice: Slint
//! compiles them into the UI's `@tr` strings, and `catalog.rs` in `OUT_DIR` holds them for
//! the strings Rust shows (`featherplay_core::i18n`, loaded by `src/language.rs`). Only a
//! debug build's catalogue has the test language.
#![allow(
    clippy::print_stdout,
    reason = "cargo reads a build script's instructions from its stdout"
)]

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};

/// The test language (`src/language.rs`), every string in brackets, for the tests and
/// Settings' list in debug builds.
const PSEUDO: &str = "qps-ploc";

/// Icon sizes and the drawing each one is rendered from: the 16 and 24 px drawings are
/// tuned for their size; 20 px uses the 24 px drawing, 32 px and up the master.
const SIZES: [(u32, &str); 6] = [
    (16, "-16"),
    (20, "-24"),
    (24, "-24"),
    (32, ""),
    (48, ""),
    (256, ""),
];

fn main() {
    // No default context: a string means one thing wherever it's used, unless its `@tr`
    // names a context (`"verb" => "Like"`); `po/update.sh` extracts them the same way.
    // Slint gets every language, the test language too in a release build: with no bundle,
    // each `@tr` compiles to a call that builds its strings at runtime, and the exe came out
    // 188 KB larger than with the bundle's string table (2026-11-07).
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_bundled_translations("po")
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/app.slint", config).expect("compile ui/app.slint");

    println!("cargo:rerun-if-changed=ui/logo");
    let out = std::env::var("OUT_DIR").expect("OUT_DIR");
    println!("cargo:rerun-if-changed=po");
    // The catalogue is what `language` offers and picks from, so a release build leaves the
    // test language out of it: the UI's bundle still has it, but nothing selects it there.
    let languages = languages(
        Path::new("po"),
        std::env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_some(),
    );
    std::fs::write(Path::new(&out).join("catalog.rs"), catalog(&languages))
        .expect("write catalog.rs");
    let ico = Path::new(&out).join("featherplay.ico");
    std::fs::write(&ico, icon("tri-vane")).expect("write icon");
    let light = Path::new(&out).join("featherplay-light.ico");
    std::fs::write(&light, icon("tri-vane-light")).expect("write light icon");
    // The installer's wizard images (packaging/windows/featherplay.iss): the small one for
    // the top corner, and the logo centred on a clear 164:314 panel for the finish page
    // (the size Inno recommends for 200 % scaling). The light logo for Windows' light mode,
    // the dark one for dark mode, as for the taskbar.
    for (name, svg) in [("", "tri-vane"), ("-light", "tri-vane-light")] {
        let master = Path::new("ui/logo").join(format!("{svg}.svg"));
        let small = Path::new(&out).join(format!("installer-small{name}.png"));
        std::fs::write(&small, render(&master, 256)).expect("write installer-small.png");
        let panel = Path::new(&out).join(format!("installer-panel{name}.png"));
        std::fs::write(&panel, render_on(&master, (430, 824), 192))
            .expect("write installer-panel.png");
    }

    // Resource ID 1 is the first icon group, the one Explorer and the shell pick.
    let rc = Path::new(&out).join("featherplay.rc");
    let ico_path = ico.display().to_string().replace('\\', "/");
    std::fs::write(&rc, format!("1 ICON \"{ico_path}\"\n")).expect("write rc");
    embed_resource::compile_for(&rc, ["featherplay"], embed_resource::NONE)
        .manifest_optional()
        .expect("compile the icon resource");
}

/// Per (context, id): the plural id ("" for none) and the forms in each language (`None`
/// where it isn't translated).
type Entries = BTreeMap<(String, String), (String, Vec<Option<Vec<String>>>)>;

/// The language folders under `po`, sorted by name as Slint orders them; the test
/// language only `with_pseudo`.
fn languages(po: &Path, with_pseudo: bool) -> Vec<PathBuf> {
    let mut dirs: Vec<_> = std::fs::read_dir(po)
        .expect("read po/")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.join("LC_MESSAGES/featherplay.po").exists())
        .filter(|p| with_pseudo || !p.ends_with(PSEUDO))
        .collect();
    dirs.sort();
    dirs
}

/// The Rust source of a `featherplay_core::i18n::Catalog` with the languages in `dirs` and
/// every translated string, sorted by (context, id) for the lookups' binary search.
fn catalog(dirs: &[PathBuf]) -> String {
    use rspolib::prelude::*;

    let mut languages = String::new();
    let mut entries = Entries::new();
    for (i, dir) in dirs.iter().enumerate() {
        let code = dir
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        let path = dir.join("LC_MESSAGES/featherplay.po");
        let file = rspolib::pofile(path.as_path())
            .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
        let name = file.metadata.get("X-Language-Name").unwrap_or(&code);
        let plural = file
            .metadata
            .get("Plural-Forms")
            .and_then(|h| h.split(';').find_map(|p| p.trim().strip_prefix("plural=")))
            .unwrap_or("n != 1");
        writeln!(
            languages,
            "    Language {{ code: {code:?}, name: {name:?}, plural: {plural:?} }},"
        )
        .expect("write");
        for e in file
            .entries
            .iter()
            .filter(|e| e.translated() && !e.msgid.is_empty())
        {
            let forms = match e.msgid_plural {
                Some(_) => e.msgstr_plural.clone(),
                None => vec![e.msgstr.clone().unwrap_or_default()],
            };
            let key = (e.msgctxt.clone().unwrap_or_default(), e.msgid.clone());
            let slot = entries.entry(key).or_insert_with(|| {
                (
                    e.msgid_plural.clone().unwrap_or_default(),
                    vec![None; dirs.len()],
                )
            });
            slot.1[i] = Some(forms);
        }
    }
    let mut out = String::from("use featherplay_core::i18n::{Catalog, Entry, Language};\n");
    writeln!(
        out,
        "static LANGUAGES: [Language; {}] = [\n{languages}];",
        dirs.len()
    )
    .expect("write");
    writeln!(out, "static ENTRIES: [Entry; {}] = [", entries.len()).expect("write");
    for ((context, id), (plural, forms)) in &entries {
        let forms: Vec<String> = forms
            .iter()
            .map(|f| match f {
                Some(f) => format!("Some(&{f:?})"),
                None => "None".into(),
            })
            .collect();
        writeln!(
            out,
            "    Entry {{ context: {context:?}, id: {id:?}, plural: {plural:?}, forms: &[{}] }},",
            forms.join(", ")
        )
        .expect("write");
    }
    out.push_str("];\n");
    out.push_str(
        "pub static CATALOG: Catalog = Catalog { languages: &LANGUAGES, entries: &ENTRIES };\n",
    );
    out
}

/// An ICO with one PNG entry per size (Vista and later read PNG entries at every size),
/// rendered from `ui/logo/<name>{,-24,-16}.svg`.
fn icon(name: &str) -> Vec<u8> {
    let pngs: Vec<(u32, Vec<u8>)> = SIZES
        .iter()
        .map(|&(size, drawing)| {
            let svg = format!("ui/logo/{name}{drawing}.svg");
            (size, render(Path::new(&svg), size))
        })
        .collect();
    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(pngs.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * pngs.len() as u32;
    for (size, png) in &pngs {
        // 256 is stored as 0.
        let dim = if *size >= 256 { 0 } else { *size as u8 };
        ico.extend_from_slice(&[dim, dim, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes()); // colour planes
        ico.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        ico.extend_from_slice(&(png.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += png.len() as u32;
    }
    for (_, png) in &pngs {
        ico.extend_from_slice(png);
    }
    ico
}

fn render(svg: &Path, size: u32) -> Vec<u8> {
    render_on(svg, (size, size), size)
}

/// The drawing `size` px wide, centred on a transparent canvas.
fn render_on(svg: &Path, (width, height): (u32, u32), size: u32) -> Vec<u8> {
    let data = std::fs::read(svg).unwrap_or_else(|e| panic!("read {}: {e}", svg.display()));
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default())
        .unwrap_or_else(|e| panic!("parse {}: {e}", svg.display()));
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).expect("pixmap");
    let scale = size as f32 / tree.size().width();
    let x = (width as f32 - tree.size().width() * scale) / 2.0;
    let y = (height as f32 - tree.size().height() * scale) / 2.0;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, x, y),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().expect("encode png")
}
