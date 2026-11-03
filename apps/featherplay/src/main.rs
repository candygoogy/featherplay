//! featherplay: restores the saved session paused, or with `--play <videoId>` plays that
//! track's radio. The tray icon's Quit, Ctrl+C, or closing the window (when it's set to
//! close featherplay; it asks until set, or hides the window to the tray) saves the session
//! and exits. Everything but argument parsing lives in the library (`src/lib.rs`).
//! One featherplay runs per home (`instance`): starting it again brings the running one's
//! window up (its other flags are ignored then). `--snapshot` and `--unregister` runs don't
//! check.
//!
//! Debug flags: `--play <videoId>`, `--volume <0..1>` (this run only), `--start-at <secs>`,
//! `--timing` (prints `first_frame_ms=<n>`), and the offscreen render
//! `--snapshot <page> --fixtures [--theme dark|light] [--size WxH] [--scale F] [--queue]
//! [--hover X,Y | --press X,Y] [--hold-loading | --hold-error] [--track N] [--language CODE] --out <png>`.
//! `--unregister` removes the Start menu shortcut and registry entry that a normal start
//! adds so the media card names the app (`identity`), then exits.
//! Dev overrides: `FEATHERPLAY_COOKIES_FILE` (the cookie header, read instead of the
//! credential store; debug builds only, or a release build with the `dev-cookies-file`
//! feature) and `FEATHERPLAY_SIDECARS_DIR` (instead of `{cache}/sidecars`).
//!
//! A release build is a GUI program, so starting it from the Start menu opens no console
//! window; its log goes to `{cache}/logs/featherplay.log` (`diagnostics`), and a panic
//! leaves a crash file beside it. Debug builds stay console programs, for `cargo run`, and
//! print the log to stderr too.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use featherplay::app::{self, AppOptions};
use featherplay::controller::{self, Options};
use featherplay::instance::{self, Start};
use featherplay::snapshot::{self, Hold, PAGES, SnapshotOptions};
use featherplay::{diagnostics, identity, language};
use featherplay_core::model::TrackId;
use featherplay_store::Paths;

const USAGE: &str = "usage: featherplay [--play <videoId>] [--volume <0..1>] [--start-at <secs>] [--timing]
       featherplay --unregister
       featherplay --snapshot <page> --fixtures [--theme dark|light] [--size WxH] [--scale F] [--queue] [--hover X,Y | --press X,Y] [--hold-loading | --hold-error] [--track N] [--language CODE] --out <png>";

enum Mode {
    App(Options, AppOptions),
    Snapshot(SnapshotOptions),
    Unregister,
}

fn parse_args() -> Result<Mode, String> {
    let mut opts = Options::default();
    let mut app = AppOptions::default();
    let mut snap: Option<String> = None;
    let mut fixtures = false;
    let mut dark = true;
    let mut size = (1280, 800);
    let mut scale = 1.0;
    let mut queue_open = false;
    let mut pointer: Option<(f32, f32)> = None;
    let mut press = false;
    let mut hold = Hold::None;
    let mut track = 0;
    let mut lang = String::new();
    let mut out: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--play" => opts.play = Some(TrackId(value("--play")?)),
            "--volume" => {
                let v: f32 = value("--volume")?
                    .parse()
                    .map_err(|_| "--volume takes a number from 0 to 1".to_string())?;
                if !(0.0..=1.0).contains(&v) {
                    return Err("--volume takes a number from 0 to 1".into());
                }
                opts.volume = Some(v);
            }
            "--start-at" => {
                let s: f64 = value("--start-at")?
                    .parse()
                    .map_err(|_| "--start-at takes seconds".to_string())?;
                if !s.is_finite() || s < 0.0 {
                    return Err("--start-at takes seconds".into());
                }
                opts.start_at = Some(Duration::from_secs_f64(s));
            }
            "--timing" => app.timing = true,
            "--snapshot" => snap = Some(value("--snapshot")?),
            "--fixtures" => fixtures = true,
            "--theme" => {
                dark = match value("--theme")?.as_str() {
                    "dark" => true,
                    "light" => false,
                    _ => return Err("--theme takes dark or light".into()),
                }
            }
            "--size" => {
                let v = value("--size")?;
                let parsed = v
                    .split_once('x')
                    .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)));
                size = match parsed {
                    Some((w, h)) if (320..=7680).contains(&w) && (240..=4320).contains(&h) => {
                        (w, h)
                    }
                    _ => return Err("--size takes WxH, e.g. 1280x800".into()),
                };
            }
            "--scale" => {
                scale = match value("--scale")?.parse::<f32>() {
                    Ok(s) if (1.0..=4.0).contains(&s) => s,
                    _ => return Err("--scale takes a display scale from 1 to 4, e.g. 1.5".into()),
                };
            }
            "--queue" => queue_open = true,
            "--hover" | "--press" => {
                let name = if arg == "--hover" {
                    "--hover"
                } else {
                    "--press"
                };
                let v = value(name)?;
                let parsed = v
                    .split_once(',')
                    .and_then(|(x, y)| Some((x.parse::<f32>().ok()?, y.parse::<f32>().ok()?)));
                pointer = Some(parsed.ok_or(format!("{name} takes X,Y in logical pixels"))?);
                press = arg == "--press";
            }
            "--hold-loading" => hold = Hold::Loading,
            "--hold-error" => hold = Hold::Error,
            "--track" => {
                track = value("--track")?.parse().map_err(|_| {
                    "--track takes the radio fixture's track number, from 0".to_string()
                })?;
            }
            "--language" => {
                lang = value("--language")?;
                if !(lang == "en" || language::bundled().contains(&lang.as_str())) {
                    return Err(format!(
                        "--language takes en or a bundled language: {}",
                        language::bundled().join(", ")
                    ));
                }
            }
            "--out" => out = Some(PathBuf::from(value("--out")?)),
            "--unregister" => return Ok(Mode::Unregister),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    let Some(page) = snap else {
        return Ok(Mode::App(opts, app));
    };
    if !fixtures {
        return Err("--snapshot renders fixture data only; add --fixtures".into());
    }
    if !PAGES.contains(&page.as_str()) {
        return Err(format!(
            "unknown page {page:?}; pages: {}",
            PAGES.join(", ")
        ));
    }
    let out = out.ok_or("--snapshot needs --out <png>")?;
    Ok(Mode::Snapshot(SnapshotOptions {
        page,
        dark,
        width: size.0,
        height: size.1,
        scale,
        queue_open,
        pointer,
        press,
        hold,
        track,
        language: lang,
        out,
    }))
}

fn main() -> ExitCode {
    controller::uptime();
    match parse_args() {
        Ok(Mode::App(opts, app)) => {
            // Before anything is made: if this home's featherplay is running, it shows its
            // window and this launch ends. The guard stays on this thread until the end.
            let _guard = match instance::start() {
                Start::Run(guard) => guard,
                Start::Exit => return ExitCode::SUCCESS,
            };
            // After the guard, so only the running featherplay writes the log.
            diagnostics::start(&Paths::resolve());
            // Before any window, so the window and its media session carry the AUMID.
            identity::startup();
            let code = app::run(opts, app);
            diagnostics::flush();
            code
        }
        Ok(Mode::Unregister) => {
            diagnostics::start(&Paths::resolve());
            let code = identity::unregister();
            diagnostics::flush();
            code
        }
        Ok(Mode::Snapshot(o)) => snapshot::run(o),
        #[allow(
            clippy::print_stderr,
            reason = "a command-line usage error, for the terminal"
        )]
        Err(e) => {
            eprintln!("featherplay: {e}");
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
