//! A play's report (`PlayTracking`): the start URL read from a `player` answer, and the call
//! built on it.

use std::collections::HashMap;
use std::time::Duration;

use featherplay_ytm::PlayTracking;
use proptest::prelude::*;

const PLAYER: &str = include_str!("../fixtures/player_tracking.json");
const BASE: &str = "https://s.youtube.com/api/stats/playback?cl=100000000&docid=Jb6gcoR266U&ei=EIxample&fexp=&ns=yt&plid=PLIDexample&el=detailpage&len=430&of=OFexample&vm=VMexample";

/// The start call, as the web player sends it: the answer's own query kept, then the
/// client, a nonce and the media time.
#[test]
fn the_call_is_built_from_the_player_answer() {
    let t = PlayTracking::from_player(PLAYER).expect("tracking");
    let url = t.url(Duration::from_millis(5250), "1.20261118.01.00");
    assert_eq!(url.host_str(), Some("s.youtube.com"));
    assert_eq!(url.path(), "/api/stats/playback");
    let q: HashMap<String, String> = url
        .query_pairs()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
    for (k, v) in [
        ("docid", "Jb6gcoR266U"),
        ("len", "430"),
        ("el", "detailpage"),
        ("ver", "2"),
        ("c", "WEB_REMIX"),
        ("cver", "1.20261118.01.00"),
        ("cmt", "5.250"),
    ] {
        assert_eq!(q.get(k).map(String::as_str), Some(v), "{k}");
    }
    assert_eq!(q["cpn"].len(), 16);
    // One nonce per play: a new answer read is a new play.
    let again = PlayTracking::from_player(PLAYER).expect("tracking");
    let cpn = |u: reqwest::Url| {
        u.query_pairs()
            .find(|(k, _)| k == "cpn")
            .map(|(_, v)| v.into_owned())
    };
    assert_ne!(cpn(url), cpn(again.url(Duration::ZERO, "1.x")));
}

/// A host, scheme or path the session's cookies mustn't go to is refused.
#[test]
fn only_youtube_stats_urls_are_taken() {
    for bad in [
        "http://s.youtube.com/api/stats/playback?len=1",
        "https://evil.example/api/stats/playback",
        "https://s.youtube.com.evil.example/api/stats/playback",
        "https://s.youtube.com/api/stats/watchtime",
        "https://user@s.youtube.com/api/stats/playback",
        "https://s.youtube.com:8443/api/stats/playback",
    ] {
        assert!(
            PlayTracking::from_player(&PLAYER.replace(BASE, bad)).is_err(),
            "{bad}"
        );
    }
}

proptest! {
    /// Any answer gives tracking or an error, never a panic. Tracking is taken exactly when
    /// the URL is an HTTPS start URL on YouTube or Google, and its call keeps the URL's own
    /// query in order before its five keys.
    #[test]
    fn any_answer_is_read_safely(
        scheme in prop::sample::select(vec!["https", "http", "ftp"]),
        host in prop::sample::select(vec!["s.youtube.com", "music.youtube.com", "youtube.com.example", "example.com", "google.com"]),
        path in prop::sample::select(vec!["/api/stats/playback", "/api/stats/watchtime", "/x"]),
        query in "[a-z=&%0-9]{0,40}",
        junk in ".{0,40}",
    ) {
        prop_assert!(PlayTracking::from_player(&junk).is_err());
        let base = format!("{scheme}://{host}{path}?{query}");
        let raw = serde_json::json!({
            "playbackTracking": { "videostatsPlaybackUrl": { "baseUrl": base } },
        }).to_string();
        let allowed = scheme == "https" && path == "/api/stats/playback"
            && (host.ends_with(".youtube.com") || host == "google.com");
        let read = PlayTracking::from_player(&raw);
        prop_assert_eq!(read.is_ok(), allowed);
        if let Ok(t) = read {
            let url = t.url(Duration::ZERO, "1.x");
            let own: Vec<_> = reqwest::Url::parse(&base).unwrap().query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
            let all: Vec<_> = url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
            prop_assert_eq!(&all[..own.len()], &own[..]);
            let added: Vec<&str> = all[own.len()..].iter().map(|(k, _)| k.as_str()).collect();
            prop_assert_eq!(added, ["ver", "c", "cver", "cpn", "cmt"]);
        }
    }
}
