//! Plays one local `.m4a` or `.webm` through `AudioEngine` on the default output device, the
//! way the app plays a song (`AudioEngine::new`, `set_volume`, `load`, `play`, then its events),
//! and prints what happened. For the Linux sound rig (`scripts/rig/sound/`), which records the
//! sound server's sink while this plays:
//!
//! ```text
//! cargo run -p featherplay-audio --example play_file -- <file> [--volume 0..1] [--seconds N] [--tmp DIR]
//! ```
//!
//! It plays to the end (or for `--seconds`, then pauses). Every line the engine logs is echoed
//! to stdout as `log: …`, among them its glitch report: `late output callback(s)` and `ring
//! ran dry` (`glitches.rs`), which the rig counts as underruns. The last lines are `rig: key=value`
//! facts. Exits 1 on an engine error, no device, or no `Started`/`Ended` in time.
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a command-line tool: its output is the report"
)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use featherplay_audio::{AudioEngine, AudioEvent, StreamSource};

/// From `load` to the first sound reaching the device.
const START_TIMEOUT: Duration = Duration::from_secs(10);

struct Echo;

static T0: OnceLock<Instant> = OnceLock::new();

impl log::Log for Echo {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        m.level() <= log::Level::Info
    }

    fn log(&self, r: &log::Record<'_>) {
        if self.enabled(r.metadata()) {
            let ms = T0.get().map_or(0, |t| t.elapsed().as_millis());
            println!("log: [{ms:>6} ms] {} {}", r.level(), r.args());
        }
    }

    fn flush(&self) {}
}

static ECHO: Echo = Echo;

struct Args {
    file: PathBuf,
    volume: f32,
    seconds: Option<f64>,
    tmp: PathBuf,
}

fn args() -> Result<Args, String> {
    let mut file = None;
    let mut volume = 1.0;
    let mut seconds = None;
    let mut tmp = std::env::temp_dir().join("featherplay-play-file");
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut value = || it.next().ok_or(format!("{a} needs a value"));
        match a.as_str() {
            "--volume" => {
                volume = value()?
                    .parse()
                    .ok()
                    .filter(|v| (0.0..=1.0).contains(v))
                    .ok_or("--volume takes 0 to 1")?;
            }
            "--seconds" => {
                seconds = Some(
                    value()?
                        .parse()
                        .ok()
                        .filter(|s: &f64| *s > 0.0)
                        .ok_or("--seconds takes a positive number")?,
                );
            }
            "--tmp" => tmp = PathBuf::from(value()?),
            _ if file.is_none() && !a.starts_with("--") => file = Some(PathBuf::from(a)),
            _ => return Err(format!("unknown argument {a}")),
        }
    }
    Ok(Args {
        file: file.ok_or("usage: play_file <file> [--volume 0..1] [--seconds N] [--tmp DIR]")?,
        volume,
        seconds,
        tmp,
    })
}

/// The `StreamSource` the app's fixtures give a local file (`perf::fixture_stream`).
fn source(path: &std::path::Path) -> Result<StreamSource, String> {
    let path = std::path::absolute(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let len = std::fs::metadata(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .len();
    let webm = path.extension().is_some_and(|e| e == "webm");
    let p = path.to_string_lossy().replace('\\', "/");
    Ok(StreamSource {
        url: format!("file:///{}", p.trim_start_matches('/')),
        mime: if webm { "audio/webm" } else { "audio/mp4" }.into(),
        content_length: Some(len),
        duration: None,
        loudness_db: None,
    })
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("play_file: {e}");
            println!("rig: result=error");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let a = args()?;
    let src = source(&a.file)?;
    T0.get_or_init(Instant::now);
    log::set_logger(&ECHO).map_err(|e| e.to_string())?;
    log::set_max_level(log::LevelFilter::Info);

    std::fs::create_dir_all(&a.tmp).map_err(|e| format!("{}: {e}", a.tmp.display()))?;
    let engine = AudioEngine::new(None, &a.tmp)?;
    let events = engine.events();
    engine.set_volume(a.volume);
    let t = Instant::now();
    engine.load(src, Duration::ZERO);
    engine.play();

    let mut started = None;
    let mut buffering = 0u32;
    let mut stop_at = None;
    let ended;
    loop {
        let deadline = match (started, stop_at) {
            (None, _) => t + START_TIMEOUT,
            (Some(_), Some(at)) => at,
            // A whole track: the engine says when it ends; this only stops a hang.
            (Some(s), None) => s + Duration::from_secs(3600),
        };
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left) {
            Ok(AudioEvent::Started) => {
                let now = Instant::now();
                started = Some(now);
                stop_at = a.seconds.map(|s| now + Duration::from_secs_f64(s));
                println!("rig: started_after_ms={}", (now - t).as_millis());
            }
            Ok(AudioEvent::Ended) => {
                ended = true;
                break;
            }
            Ok(AudioEvent::Buffering(true)) => buffering += 1,
            Ok(AudioEvent::Error(e)) => return Err(format!("engine error: {e}")),
            Ok(AudioEvent::NoDevice) => return Err("no output device".into()),
            Ok(AudioEvent::DeviceLost) => return Err("the output device was lost".into()),
            Ok(_) => {}
            Err(_) if started.is_none() => {
                return Err(format!("no sound within {START_TIMEOUT:?}"));
            }
            Err(_) => {
                // `--seconds` is up: a pause makes the engine report its glitches.
                engine.pause();
                ended = false;
                break;
            }
        }
    }
    let played = started.map_or(Duration::ZERO, |s| s.elapsed());
    let position = engine.position();
    // The glitch report goes out on the engine thread as it handles the pause.
    std::thread::sleep(Duration::from_millis(300));
    println!("rig: ended={ended}");
    println!("rig: played_ms={}", played.as_millis());
    println!("rig: position_ms={}", position.as_millis());
    println!("rig: buffering={buffering}");
    println!("rig: result=ok");
    Ok(())
}
