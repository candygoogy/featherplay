use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Used when a URL has no readable `expire`. Real URLs expire about 6 h after resolving.
const FALLBACK_LIFETIME: Duration = Duration::from_secs(5 * 3600);

/// The `expire` query parameter (unix seconds) of a stream URL, else `fallback_now` + 5 h.
pub fn expires_at_from_url(url: &str, fallback_now: SystemTime) -> SystemTime {
    query_param(url, "expire")
        .and_then(|v| v.parse::<u64>().ok())
        .map(|secs| UNIX_EPOCH + Duration::from_secs(secs))
        .unwrap_or(fallback_now + FALLBACK_LIFETIME)
}

/// The percent-decoded value of the first `key=` parameter in `url`'s query string.
pub(crate) fn query_param(url: &str, key: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    form_value(query.split('#').next().unwrap_or_default(), key)
}

/// The percent-decoded value of the first `key=` pair in `a=1&b=2` (a `signatureCipher`).
pub(crate) fn form_value(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (k == key).then(|| percent_decode(v))
    })
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        match (
            b[i],
            b.get(i + 1).copied().and_then(hex),
            b.get(i + 2).copied().and_then(hex),
        ) {
            (b'%', Some(hi), Some(lo)) => {
                out.push((hi * 16 + lo) as u8);
                i += 3;
            }
            (c, ..) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
