use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use featherplay_core::model::TrackId;
use featherplay_streams::{
    ResolveError, Sidecars, StreamResolver, YtDlpResolver, classify_failure, parse_ytdlp_json,
};

const FIXTURE_140: &str = include_str!("../fixtures/ytdlp_140.json");
const UNAVAILABLE_STDERR: &str = include_str!("../fixtures/unavailable.stderr.txt");

#[test]
fn parse_picks_requested_format() {
    let now = UNIX_EPOCH + Duration::from_secs(1_793_021_330);
    let s = parse_ytdlp_json(FIXTURE_140, now).expect("fixture parses");
    assert_eq!(s.itag, 140);
    assert_eq!(s.mime, "audio/mp4");
    assert_eq!(s.bitrate_kbps, 130);
    assert_eq!(s.content_length, Some(6_953_417));
    assert_eq!(s.duration, Some(Duration::from_secs(430)));
    assert_eq!(
        s.expires_at,
        UNIX_EPOCH + Duration::from_secs(1_793_042_928)
    );
    assert_eq!(
        s.available_at,
        Some(UNIX_EPOCH + Duration::from_secs(1_793_021_334))
    );
    // The scrobbler's alternates: for a song, the tags start with the artist.
    assert_eq!(s.video_title.as_deref(), Some("Around the World"));
    assert_eq!(s.first_tag.as_deref(), Some("Daft Punk"));
    assert!(
        s.url.contains("itag=140"),
        "url is the selected format's, not another one"
    );
}

/// An `available_at` past what the clock can hold is left out, without a panic.
#[test]
fn parse_leaves_out_a_gate_too_far_ahead() {
    let mut v: serde_json::Value = serde_json::from_str(FIXTURE_140).unwrap();
    v["available_at"] = u64::MAX.into();
    let now = UNIX_EPOCH + Duration::from_secs(1_793_021_330);
    let s = parse_ytdlp_json(&v.to_string(), now).expect("parses");
    assert_eq!(s.available_at, None);
}

#[test]
fn parse_unavailable_maps_error() {
    // yt-dlp -J prints `null` and exits 1 for an unavailable video.
    assert!(
        matches!(classify_failure(1, UNAVAILABLE_STDERR), ResolveError::Unavailable(m) if m.contains("unavailable"))
    );
    assert!(matches!(
        parse_ytdlp_json("null", SystemTime::now()),
        Err(ResolveError::Unavailable(_))
    ));
    // Anything else is a tool failure, which earns an update and a retry.
    let bot = "ERROR: [youtube] x: Sign in to confirm you're not a bot";
    assert!(matches!(
        classify_failure(1, bot),
        ResolveError::ToolFailed { code: 1, .. }
    ));
}

/// `-f 251/140` picked Opus: the mime is WebM's, from the URL or else from the extension.
#[test]
fn parse_reports_webm_for_format_251() {
    let now = SystemTime::now();
    let json =
        |url: &str| format!(r#"{{"format_id":"251","ext":"webm","abr":135.2,"url":"{url}"}}"#);
    let s = parse_ytdlp_json(
        &json("https://media.example.invalid/videoplayback?itag=251&mime=audio%2Fwebm"),
        now,
    )
    .unwrap();
    assert_eq!(
        (s.itag, s.mime.as_str(), s.bitrate_kbps),
        (251, "audio/webm", 135)
    );
    let s = parse_ytdlp_json(
        &json("https://media.example.invalid/videoplayback?itag=251"),
        now,
    )
    .unwrap();
    assert_eq!(s.mime, "audio/webm");
}

/// A fresh dir under FEATHERPLAY_HOME, laid out like the app's: `{home}/sidecars`, `{home}/tmp`.
fn test_home(name: &str) -> PathBuf {
    let base =
        std::env::var("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME to a D: dir for tests");
    let home = PathBuf::from(base)
        .join("test-streams")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(home.join("sidecars")).unwrap();
    home
}

#[tokio::test]
async fn cookie_tempfile_removed_after_error() {
    let home = test_home("cookie");
    // No yt-dlp.exe in this sidecar dir, so the spawn fails after the cookie file is written.
    let r = YtDlpResolver::new(
        Sidecars {
            dir: home.join("sidecars"),
        },
        Some("SID=abc; HSID=def".into()),
    );
    let err = r.resolve(&TrackId("Jb6gcoR266U".into())).await.unwrap_err();
    assert!(matches!(err, ResolveError::ToolMissing), "got {err:?}");
    let tmp = home.join("tmp");
    assert!(tmp.is_dir(), "the cookie file was written to {{home}}/tmp");
    let left: Vec<_> = std::fs::read_dir(&tmp).unwrap().collect();
    assert!(left.is_empty(), "cookie temp file left behind: {left:?}");
    std::fs::remove_dir_all(&home).unwrap();
}

#[test]
fn a_cookie_file_left_by_a_killed_process_is_swept() {
    let home = test_home("cookie-sweep");
    let tmp = home.join("tmp");
    std::fs::create_dir_all(&tmp).unwrap();
    let stale = tmp.join("cookies-1-123-0.txt");
    std::fs::write(&stale, "SID\tabc").unwrap();
    let audio = tmp.join("featherplay-audio-x");
    std::fs::write(&audio, "").unwrap();
    let _r = YtDlpResolver::new(
        Sidecars {
            dir: home.join("sidecars"),
        },
        Some("SID=abc".into()),
    );
    assert!(!stale.exists(), "the stale cookie file is still there");
    assert!(audio.exists(), "only cookie files are swept");
    std::fs::remove_dir_all(&home).unwrap();
}
