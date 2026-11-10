use std::time::{Duration, SystemTime, UNIX_EPOCH};

use featherplay_streams::expires_at_from_url;

#[test]
fn expiry_reads_expire_param() {
    let url = "https://media.example.invalid/videoplayback?ei=x&expire=1761400000&itag=140";
    let got = expires_at_from_url(url, SystemTime::now());
    assert_eq!(got, UNIX_EPOCH + Duration::from_secs(1_761_400_000));
}

/// An `expire` past what the clock can hold counts as unreadable, without a panic.
#[test]
fn expiry_too_far_ahead_falls_back_to_5h() {
    let now = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let url = format!(
        "https://media.example.invalid/videoplayback?expire={}",
        u64::MAX
    );
    assert_eq!(
        expires_at_from_url(&url, now),
        now + Duration::from_secs(5 * 3600)
    );
}

#[test]
fn expiry_falls_back_to_5h() {
    let now = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
    let five_h = now + Duration::from_secs(5 * 3600);
    assert_eq!(
        expires_at_from_url("https://media.example.invalid/videoplayback?itag=140", now),
        five_h
    );
    // A parameter that only ends in "expire", or one that isn't a number, doesn't count.
    assert_eq!(
        expires_at_from_url("https://x.invalid/v?noexpire=1&expire=soon", now),
        five_h
    );
}
