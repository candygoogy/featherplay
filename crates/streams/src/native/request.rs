//! The player API requests' bodies and headers, and the reading of its answer.

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::{
    CLIENT_ID, CLIENT_NAME, EMBED_URL, EmbedConfig, NativeError, Pending, USER_AGENT, VISIONOS,
    VISIONOS_ID, VISIONOS_USER_AGENT, VISIONOS_VERSION, failed,
};
use crate::expiry::{form_value, query_param};
use crate::{ResolvedStream, Via, expires_at_from_url};

/// The player API request's body, as yt-dlp's `web_embedded` client sends it.
pub(crate) fn player_request_body(video: &str, c: &EmbedConfig, sts: u64) -> Value {
    let mut client = json!({
        "clientName": CLIENT_NAME,
        "clientVersion": c.client_version,
        "userAgent": USER_AGENT,
        "hl": "en",
        "timeZone": "UTC",
        "utcOffsetMinutes": 0,
    });
    if let Some(v) = &c.visitor_data {
        client["visitorData"] = json!(v);
    }
    let mut playback = json!({
        "html5Preference": "HTML5_PREF_WANTS",
        "signatureTimestamp": sts,
    });
    if let Some(f) = &c.host_flags {
        playback["encryptedHostFlags"] = json!(f);
    }
    json!({
        "context": {
            "client": client,
            "thirdParty": { "embedUrl": EMBED_URL },
        },
        "videoId": video,
        "playbackContext": { "contentPlaybackContext": playback },
        "contentCheckOk": true,
        "racyCheckOk": true,
    })
}

/// The client headers of the player API request (the body and origin aside).
pub(crate) fn player_headers(c: &EmbedConfig) -> Vec<(&'static str, String)> {
    let mut h = vec![
        ("X-YouTube-Client-Name", CLIENT_ID.to_string()),
        ("X-YouTube-Client-Version", c.client_version.clone()),
    ];
    if let Some(v) = &c.visitor_data {
        h.push(("X-Goog-Visitor-Id", v.clone()));
    }
    h
}

/// The player API request's body as yt-dlp's `visionos` client sends it: no player, so no
/// `signatureTimestamp`.
pub(crate) fn visionos_request_body(video: &str, visitor: &str) -> Value {
    json!({
        "context": {
            "client": {
                "clientName": VISIONOS,
                "clientVersion": VISIONOS_VERSION,
                "deviceMake": "Apple",
                "deviceModel": "RealityDevice17,1",
                "osName": "visionOS",
                "osVersion": "26.5.23O471",
                "userAgent": VISIONOS_USER_AGENT,
                "hl": "en",
                "timeZone": "UTC",
                "utcOffsetMinutes": 0,
                "visitorData": visitor,
            },
        },
        "videoId": video,
        "playbackContext": {
            "contentPlaybackContext": { "html5Preference": "HTML5_PREF_WANTS" },
        },
        "contentCheckOk": true,
        "racyCheckOk": true,
    })
}

/// The visionOS client's headers of the player API request (the body and origin aside).
pub(crate) fn visionos_headers(visitor: &str) -> Vec<(&'static str, String)> {
    vec![
        ("User-Agent", VISIONOS_USER_AGENT.to_string()),
        ("X-YouTube-Client-Name", VISIONOS_ID.to_string()),
        ("X-YouTube-Client-Version", VISIONOS_VERSION.to_string()),
        ("X-Goog-Visitor-Id", visitor.to_string()),
    ]
}

/// Reads a player API answer: the URL and challenges of the first format in `wanted` that
/// it has (and isn't DRM-protected), and the metadata.
pub(crate) fn parse_player_response(
    body: &str,
    video: &str,
    wanted: &[u32],
) -> Result<Pending, NativeError> {
    let v: Value =
        serde_json::from_str(body).map_err(|e| failed(format!("player API JSON: {e}")))?;
    let status = v["playabilityStatus"]["status"].as_str().unwrap_or("");
    if status != "OK" {
        let reason = v["playabilityStatus"]["reason"]
            .as_str()
            .unwrap_or("no reason");
        let m = format!("{status}: {reason}");
        // `UNPLAYABLE` is a video that can't be embedded, the others a sign-in or age check.
        // `ERROR` ("This video is unavailable") is also what a request with a bad config
        // gets, so yt-dlp's own embedded run decides then.
        return Err(match status {
            "UNPLAYABLE"
            | "LOGIN_REQUIRED"
            | "AGE_CHECK_REQUIRED"
            | "AGE_VERIFICATION_REQUIRED"
            | "CONTENT_CHECK_REQUIRED" => NativeError::Skip(m),
            _ => failed(m),
        });
    }
    if v["videoDetails"]["videoId"].as_str() != Some(video) {
        return Err(failed("the answer is for another video"));
    }
    let gate = preroll_gate(&v)?;
    let formats = v["streamingData"]["adaptiveFormats"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    let there: Vec<(u32, &Value)> = wanted
        .iter()
        .filter_map(|&itag| {
            let f = formats
                .iter()
                .find(|f| f["itag"].as_u64() == Some(u64::from(itag)))?;
            Some((itag, f))
        })
        .collect();
    if there.is_empty() {
        let names: Vec<String> = wanted.iter().map(u32::to_string).collect();
        return Err(NativeError::Skip(format!(
            "no format {}",
            names.join(" or ")
        )));
    }
    let (itag, f) = there
        .into_iter()
        .find(|(_, f)| f.get("drmFamilies").is_none())
        .ok_or_else(|| NativeError::Skip("the stream is DRM-protected".into()))?;
    let (url, sig) = match f["url"].as_str() {
        Some(u) => (u.to_string(), None),
        None => {
            let sc = f["signatureCipher"]
                .as_str()
                .ok_or_else(|| NativeError::NoUrl(format!("format {itag} has no URL")))?;
            let url = form_value(sc, "url").ok_or_else(|| failed("a cipher without a URL"))?;
            let s = form_value(sc, "s").ok_or_else(|| failed("a cipher without s"))?;
            let sp = form_value(sc, "sp").unwrap_or_else(|| "signature".into());
            (url, Some((sp, s)))
        }
    };
    if !url.starts_with("https://") {
        return Err(failed(format!("format {itag}'s URL isn't https")));
    }
    let mime = query_param(&url, "mime")
        .filter(|m| m.contains('/'))
        .or_else(|| {
            f["mimeType"]
                .as_str()
                .map(|m| m.split(';').next().unwrap_or(m).trim().to_string())
        })
        .unwrap_or_else(|| "audio/mp4".into());
    let num = |x: &Value| x.as_u64().or_else(|| x.as_str()?.parse().ok());
    let kbps = num(&f["averageBitrate"])
        .or_else(|| num(&f["bitrate"]))
        .unwrap_or(0);
    let d = &v["videoDetails"];
    Ok(Pending {
        itag,
        n: query_param(&url, "n").filter(|n| !n.is_empty()),
        url,
        sig,
        mime,
        bitrate_kbps: ((kbps + 500) / 1000) as u32,
        content_length: num(&f["contentLength"]),
        duration: num(&d["lengthSeconds"])
            .filter(|s| *s > 0)
            .map(Duration::from_secs)
            .or_else(|| num(&f["approxDurationMs"]).map(Duration::from_millis)),
        video_title: text(&d["title"]),
        first_tag: text(&d["keywords"][0]),
        // Per format, as the web player reads it; else the one for the whole video.
        loudness_db: f["loudnessDb"]
            .as_f64()
            .or_else(|| v["playerConfig"]["audioConfig"]["loudnessDb"].as_f64())
            .map(|db| db as f32),
        gate,
    })
}

/// How long the URL of an answer with ads before the song is refused: each ad until it
/// could be skipped, else its length, added up (as yt-dlp reads them). The embedded client
/// sends them as `adSlots` since 2026-11-05; `adPlacements` is the older form. An ad before
/// the song whose length isn't given is an error: yt-dlp gets the resolve.
fn preroll_gate(v: &Value) -> Result<Option<Duration>, NativeError> {
    let mut ads: Vec<&Value> = Vec::new();
    let mut unreadable = false;
    for p in v["adPlacements"].as_array().into_iter().flatten() {
        let p = &p["adPlacementRenderer"];
        if p["config"]["adPlacementConfig"]["kind"] == "AD_PLACEMENT_KIND_START" {
            let ad = &p["renderer"]["instreamVideoAdRenderer"];
            if ad.is_object() {
                ads.push(ad);
            } else {
                unreadable = true;
            }
        }
    }
    for s in v["adSlots"].as_array().into_iter().flatten() {
        let s = &s["adSlotRenderer"];
        if s["adSlotMetadata"]["triggerEvent"] != "SLOT_TRIGGER_EVENT_BEFORE_CONTENT" {
            continue;
        }
        let content = &s["fulfillmentContent"]["fulfilledLayout"]["playerBytesAdLayoutRenderer"]["renderingContent"];
        let pod = content["playerBytesSequentialLayoutRenderer"]["sequentialLayouts"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|l| &l["playerBytesAdLayoutRenderer"]["renderingContent"]);
        let before = ads.len();
        ads.extend(
            std::iter::once(content)
                .chain(pod)
                .map(|c| &c["instreamVideoAdRenderer"])
                .filter(|ad| ad.is_object()),
        );
        unreadable |= ads.len() == before;
    }
    let mut gate = Duration::ZERO;
    for ad in ads {
        let skip = ad["skipOffsetMilliseconds"].as_f64().map(|ms| ms / 1000.0);
        let length = ad["playerVars"]
            .as_str()
            .and_then(|q| form_value(q, "length_seconds"))
            .and_then(|s| s.parse::<f64>().ok());
        match skip
            .or(length)
            .and_then(|s| Duration::try_from_secs_f64(s).ok())
        {
            Some(wait) => gate = gate.saturating_add(wait),
            None => unreadable = true,
        }
    }
    if unreadable {
        return Err(failed("an ad before the song, of unknown length"));
    }
    Ok((!gate.is_zero()).then_some(gate))
}

fn text(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The solver's answers, by challenge.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Answers {
    pub n: HashMap<String, String>,
    pub sig: HashMap<String, String>,
}

/// The stream, with the answered challenges put in its URL.
pub(crate) fn finish(
    p: Pending,
    a: &Answers,
    now: SystemTime,
) -> Result<ResolvedStream, NativeError> {
    let mut url = p.url;
    if let Some(n) = &p.n {
        let answer = a.n.get(n).ok_or_else(|| failed("no answer for n"))?;
        url = with_param(&url, "n", answer);
    }
    if let Some((sp, s)) = &p.sig {
        let answer = a.sig.get(s).ok_or_else(|| failed("no answer for sig"))?;
        url = with_param(&url, sp, answer);
    }
    // Counted from the next whole second, as yt-dlp does (its URLs played so, 2026-11-07).
    // A gate past what the clock can hold (a damaged answer) is left out.
    let available_at = p.gate.and_then(|gate| {
        let since = now.duration_since(UNIX_EPOCH).unwrap_or_default();
        let whole = since.as_secs() + u64::from(since.subsec_nanos() > 0);
        crate::expiry::unix_secs(whole)?.checked_add(gate)
    });
    Ok(ResolvedStream {
        expires_at: expires_at_from_url(&url, now),
        url,
        mime: p.mime,
        itag: p.itag,
        bitrate_kbps: p.bitrate_kbps,
        content_length: p.content_length,
        duration: p.duration,
        available_at,
        video_title: p.video_title,
        first_tag: p.first_tag,
        loudness_db: p.loudness_db,
        via: Via::NativeEmbedded,
    })
}

/// `url` with `key` set to `value` (replaced in place, or added at the end).
pub(crate) fn with_param(url: &str, key: &str, value: &str) -> String {
    let encoded = percent_encode(value);
    let (base, query) = url.split_once('?').unwrap_or((url, ""));
    let mut found = false;
    let mut pairs: Vec<String> = query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            if p.split('=').next() == Some(key) && !found {
                found = true;
                format!("{key}={encoded}")
            } else {
                p.to_string()
            }
        })
        .collect();
    if !found {
        pairs.push(format!("{key}={encoded}"));
    }
    format!("{base}?{}", pairs.join("&"))
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Quality;
    use crate::native::tests::{AD_SLOT, PLAYER, VIDEO, VISIONOS_PLAYER, config};

    #[test]
    fn the_request_is_the_embedded_clients() {
        let body = player_request_body(VIDEO, &config(), 20752);
        let client = &body["context"]["client"];
        assert_eq!(client["clientName"], "WEB_EMBEDDED_PLAYER");
        assert_eq!(client["clientVersion"], "2.20261028.01.00");
        assert_eq!(client["visitorData"], "CgtTY3J1YmJlZA%3D%3D");
        assert_eq!(body["context"]["thirdParty"]["embedUrl"], EMBED_URL);
        assert_eq!(body["videoId"], VIDEO);
        let pb = &body["playbackContext"]["contentPlaybackContext"];
        assert_eq!(pb["signatureTimestamp"], 20752);
        assert_eq!(pb["encryptedHostFlags"], "AEZHscrubbed==");
        assert_eq!(body["contentCheckOk"], true);
        assert_eq!(body["racyCheckOk"], true);
        let h = player_headers(&config());
        assert!(h.contains(&("X-YouTube-Client-Name", "56".into())));
        assert!(h.contains(&("X-YouTube-Client-Version", "2.20261028.01.00".into())));
        assert!(h.contains(&("X-Goog-Visitor-Id", "CgtTY3J1YmJlZA%3D%3D".into())));
        // No visitor data or host flags: neither the fields nor the header.
        let bare = EmbedConfig {
            visitor_data: None,
            host_flags: None,
            ..config()
        };
        let body = player_request_body(VIDEO, &bare, 1);
        assert!(body["context"]["client"].get("visitorData").is_none());
        let pb = &body["playbackContext"]["contentPlaybackContext"];
        assert!(pb.get("encryptedHostFlags").is_none());
        assert_eq!(player_headers(&bare).len(), 2);
    }

    #[test]
    fn the_visionos_request_is_yt_dlps() {
        let body = visionos_request_body(VIDEO, "visitor");
        let client = &body["context"]["client"];
        assert_eq!(client["clientName"], "VISIONOS");
        assert_eq!(client["clientVersion"], "1.02");
        assert_eq!(client["deviceModel"], "RealityDevice17,1");
        assert_eq!(client["visitorData"], "visitor");
        assert!(body["context"].get("thirdParty").is_none());
        let pb = &body["playbackContext"]["contentPlaybackContext"];
        assert!(pb.get("signatureTimestamp").is_none(), "no player");
        assert_eq!(body["videoId"], VIDEO);
        let h = visionos_headers("visitor");
        assert!(h.contains(&("X-YouTube-Client-Name", "101".into())));
        assert!(h.contains(&("X-Goog-Visitor-Id", "visitor".into())));
        assert!(h.contains(&("User-Agent", VISIONOS_USER_AGENT.into())));
    }

    #[test]
    fn a_recorded_visionos_answer_has_a_url_without_challenges() {
        let p = parse_player_response(VISIONOS_PLAYER, VIDEO, Quality::High.itags()).unwrap();
        assert_eq!((p.itag, p.mime.as_str()), (251, "audio/webm"));
        assert_eq!(query_param(&p.url, "c").as_deref(), Some("VISIONOS"));
        assert_eq!((p.n, p.sig, p.gate), (None, None, None));
        assert_eq!(p.loudness_db, Some(7.17));
        assert_eq!(p.video_title.as_deref(), Some("affection"));
    }

    #[test]
    fn a_recorded_answer_gives_format_140_and_its_challenges() {
        let p = parse_player_response(PLAYER, VIDEO, Quality::Standard.itags()).unwrap();
        assert!(
            p.url
                .starts_with("https://rr1---sn-scrubbed.googlevideo.com/videoplayback?")
        );
        assert_eq!(query_param(&p.url, "itag").as_deref(), Some("140"));
        assert_eq!(p.n.as_deref(), Some("tbgZJm7jUNULHGjSAEd"));
        let (sp, s) = p.sig.clone().unwrap();
        assert_eq!((sp.as_str(), s.len()), ("sig", 107));
        assert_eq!(p.mime, "audio/mp4");
        assert_eq!(p.bitrate_kbps, 130);
        assert_eq!(p.content_length, Some(3_809_688));
        assert_eq!(p.duration, Some(Duration::from_secs(235)));
        assert_eq!(p.video_title.as_deref(), Some("affection"));
        assert!(p.first_tag.is_some());

        let answers = Answers {
            n: HashMap::from([("tbgZJm7jUNULHGjSAEd".into(), "solved-n".into())]),
            sig: HashMap::from([(s.clone(), "AB=C/D".into())]),
        };
        let now = SystemTime::now();
        let stream = finish(p.clone(), &answers, now).unwrap();
        assert_eq!(query_param(&stream.url, "n").as_deref(), Some("solved-n"));
        assert_eq!(query_param(&stream.url, "sig").as_deref(), Some("AB=C/D"));
        assert!(stream.url.contains("&sig=AB%3DC%2FD"));
        assert_eq!(stream.url.matches("&n=").count(), 1);
        assert_eq!(stream.itag, 140);
        assert_eq!(stream.available_at, None);
        assert_eq!(
            stream.expires_at,
            UNIX_EPOCH + Duration::from_secs(1_793_234_740)
        );
        // An unanswered challenge is an error, not a URL that fails later.
        assert!(finish(p, &Answers::default(), now).is_err());
    }

    #[test]
    fn the_answer_gives_the_songs_loudness() {
        let p = parse_player_response(PLAYER, VIDEO, Quality::High.itags()).unwrap();
        assert_eq!(p.loudness_db, Some(7.17));
        let (_, s) = p.sig.clone().unwrap();
        let answers = Answers {
            n: HashMap::from([("tbgZJm7jUNULHGjSAEd".into(), "solved-n".into())]),
            sig: HashMap::from([(s, "solved-sig".into())]),
        };
        let stream = finish(p, &answers, SystemTime::now()).unwrap();
        assert_eq!(stream.loudness_db, Some(7.17));

        // Without one per format, the player's own (`playerConfig.audioConfig`); else none.
        let bare = PLAYER.replace("\"loudnessDb\": 7.17,", "");
        let p = parse_player_response(&bare, VIDEO, Quality::High.itags()).unwrap();
        assert_eq!(p.loudness_db, None);
        let config = bare.replacen(
            '{',
            r#"{ "playerConfig": { "audioConfig": { "loudnessDb": -2.5 } },"#,
            1,
        );
        let p = parse_player_response(&config, VIDEO, Quality::High.itags()).unwrap();
        assert_eq!(p.loudness_db, Some(-2.5));
    }

    #[test]
    fn high_quality_picks_format_251_as_webm() {
        let p = parse_player_response(PLAYER, VIDEO, Quality::High.itags()).unwrap();
        assert_eq!(query_param(&p.url, "itag").as_deref(), Some("251"));
        assert_eq!(p.mime, "audio/webm");
        assert_eq!(p.content_length, Some(3_856_919));
        let (_, s) = p.sig.clone().unwrap();
        let answers = Answers {
            n: HashMap::from([("tbgZJm7jUNULHGjSAEd".into(), "solved-n".into())]),
            sig: HashMap::from([(s, "solved-sig".into())]),
        };
        let stream = finish(p, &answers, SystemTime::now()).unwrap();
        assert_eq!(stream.itag, 251);
        assert_eq!(stream.mime, "audio/webm");
    }

    #[test]
    fn high_quality_falls_back_to_format_140_without_251() {
        let body = PLAYER.replace("\"itag\": 251", "\"itag\": 250");
        let p = parse_player_response(&body, VIDEO, Quality::High.itags()).unwrap();
        assert_eq!(query_param(&p.url, "itag").as_deref(), Some("140"));
        assert_eq!(p.mime, "audio/mp4");
        assert_eq!(p.itag, 140);
    }

    #[test]
    fn a_missing_format_names_the_ones_wanted() {
        let skip = |body: &str, q: Quality| match parse_player_response(body, VIDEO, q.itags()) {
            Err(NativeError::Skip(m)) => m,
            other => panic!("{other:?}"),
        };
        let neither = PLAYER
            .replace("\"itag\": 251", "\"itag\": 250")
            .replace("\"itag\": 140", "\"itag\": 139");
        assert_eq!(skip(&neither, Quality::High), "no format 251 or 140");
        let no_140 = PLAYER.replace("\"itag\": 140", "\"itag\": 139");
        assert_eq!(skip(&no_140, Quality::Standard), "no format 140");
    }

    #[test]
    fn answers_that_mean_another_client_or_yt_dlp() {
        let skip = |body: &str| {
            matches!(
                parse_player_response(body, VIDEO, Quality::Standard.itags()),
                Err(NativeError::Skip(_))
            )
        };
        let fail = |body: &str| {
            matches!(
                parse_player_response(body, VIDEO, Quality::Standard.itags()),
                Err(NativeError::Failed(_))
            )
        };
        assert!(skip(
            r#"{"playabilityStatus":{"status":"UNPLAYABLE","reason":"Playback on other websites has been disabled by the video owner"}}"#
        ));
        assert!(skip(
            r#"{"playabilityStatus":{"status":"LOGIN_REQUIRED","reason":"Sign in to confirm your age"}}"#
        ));
        // What a request with a bad config gets too.
        assert!(fail(
            r#"{"playabilityStatus":{"status":"ERROR","reason":"This video is unavailable"}}"#
        ));
        assert!(fail("not json"));
        assert!(
            fail(&PLAYER.replace(VIDEO, "aaaaaaaaaaa")),
            "another video's answer"
        );
        assert!(
            skip(&PLAYER.replace("\"itag\": 140", "\"itag\": 139")),
            "no format 140"
        );
        let preroll = PLAYER.replacen(
            '{',
            r#"{"adPlacements":[{"adPlacementRenderer":{"config":{"adPlacementConfig":{"kind":"AD_PLACEMENT_KIND_START"}}}}],"#,
            1,
        );
        assert!(
            fail(&preroll),
            "a pre-roll of unknown length is left to yt-dlp"
        );
    }

    /// `PLAYER` with `ads` (a JSON member) in front.
    fn with_ads(ads: &str) -> String {
        PLAYER.replacen('{', &format!("{{{ads},"), 1)
    }

    #[test]
    fn a_pre_roll_ad_slot_gates_the_url_until_it_could_be_skipped() {
        let p = parse_player_response(&with_ads(AD_SLOT), VIDEO, Quality::High.itags()).unwrap();
        assert_eq!(p.gate, Some(Duration::from_secs(5)));
        let (_, s) = p.sig.clone().unwrap();
        let answers = Answers {
            n: HashMap::from([("tbgZJm7jUNULHGjSAEd".into(), "solved-n".into())]),
            sig: HashMap::from([(s, "solved-sig".into())]),
        };
        // As yt-dlp counts it: from the next whole second.
        let now = UNIX_EPOCH + Duration::from_millis(1_794_043_690_250);
        let stream = finish(p, &answers, now).unwrap();
        assert_eq!(
            stream.available_at,
            Some(UNIX_EPOCH + Duration::from_secs(1_794_043_696))
        );
    }

    /// A gate past what the clock can hold (a damaged answer) is left out, without a panic.
    #[test]
    fn a_gate_too_far_ahead_is_left_out() {
        let mut p =
            parse_player_response(&with_ads(AD_SLOT), VIDEO, Quality::High.itags()).unwrap();
        p.gate = Some(Duration::MAX);
        let (_, s) = p.sig.clone().unwrap();
        let answers = Answers {
            n: HashMap::from([("tbgZJm7jUNULHGjSAEd".into(), "solved-n".into())]),
            sig: HashMap::from([(s, "solved-sig".into())]),
        };
        let stream = finish(p, &answers, SystemTime::now()).unwrap();
        assert_eq!(stream.available_at, None);
    }

    #[test]
    fn an_ad_that_cant_be_skipped_gates_the_url_for_its_length() {
        let ads = AD_SLOT.replace(r#""skipOffsetMilliseconds":5000,"#, "");
        let p = parse_player_response(&with_ads(&ads), VIDEO, Quality::High.itags()).unwrap();
        assert_eq!(p.gate, Some(Duration::from_secs(15)));
    }

    /// A pod of two ads before the song, and an older-style placement: the waits add up.
    #[test]
    fn the_waits_of_every_ad_before_the_song_add_up() {
        let pod = r#""adSlots":[{"adSlotRenderer":{"adSlotMetadata":{"triggerEvent":"SLOT_TRIGGER_EVENT_BEFORE_CONTENT"},"fulfillmentContent":{"fulfilledLayout":{"playerBytesAdLayoutRenderer":{"renderingContent":{"playerBytesSequentialLayoutRenderer":{"sequentialLayouts":[{"playerBytesAdLayoutRenderer":{"renderingContent":{"instreamVideoAdRenderer":{"skipOffsetMilliseconds":5000}}}},{"playerBytesAdLayoutRenderer":{"renderingContent":{"instreamVideoAdRenderer":{"playerVars":"length_seconds=6"}}}}]}}}}}}}],"adPlacements":[{"adPlacementRenderer":{"config":{"adPlacementConfig":{"kind":"AD_PLACEMENT_KIND_START"}},"renderer":{"instreamVideoAdRenderer":{"skipOffsetMilliseconds":2500}}}},{"adPlacementRenderer":{"config":{"adPlacementConfig":{"kind":"AD_PLACEMENT_KIND_END"}},"renderer":{"instreamVideoAdRenderer":{"skipOffsetMilliseconds":9000}}}}]"#;
        let p = parse_player_response(&with_ads(pod), VIDEO, Quality::High.itags()).unwrap();
        assert_eq!(p.gate, Some(Duration::from_millis(13_500)));
    }

    #[test]
    fn an_answer_without_ads_before_the_song_is_not_gated() {
        let after = AD_SLOT.replace(
            "SLOT_TRIGGER_EVENT_BEFORE_CONTENT",
            "SLOT_TRIGGER_EVENT_LAYOUT_ID_ENTERED",
        );
        let p = parse_player_response(&with_ads(&after), VIDEO, Quality::High.itags()).unwrap();
        assert_eq!(p.gate, None);
        let p = parse_player_response(PLAYER, VIDEO, Quality::High.itags()).unwrap();
        assert_eq!(p.gate, None);
    }

    #[test]
    fn params_are_replaced_or_added() {
        assert_eq!(
            with_param("https://h/p?a=1&n=x&b=2", "n", "y z"),
            "https://h/p?a=1&n=y%20z&b=2"
        );
        assert_eq!(
            with_param("https://h/p?a=1", "sig", "s"),
            "https://h/p?a=1&sig=s"
        );
        assert_eq!(
            with_param("https://h/p?an=1", "n", "2"),
            "https://h/p?an=1&n=2"
        );
    }
}
