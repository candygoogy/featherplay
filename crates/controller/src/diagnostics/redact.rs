//! Masks secrets in a log line before it's written anywhere: cookie values, the `Cookie`
//! and `Authorization` headers, tokens, keys and session keys, email addresses, and the
//! user's folders (see `paths`).

mod paths;

/// What a masked value becomes.
const MASK: &str = "***";

/// Headers whose whole value is secret: masked to the end of the line.
const HEADERS: &[&str] = &["cookie", "set-cookie", "authorization"];
/// Google's sign-in cookies (any `__Secure-` or `__Host-` cookie counts too).
const COOKIES: &[&str] = &[
    "sid",
    "hsid",
    "ssid",
    "apisid",
    "sapisid",
    "sidcc",
    "login_info",
    "nid",
];
/// Names whose value is secret wherever they appear: a query, JSON, `name: value`.
const SECRETS: &[&str] = &[
    "sk",
    "api_key",
    "apikey",
    "api_sig",
    "api_secret",
    "session_key",
    "sessionkey",
    "password",
];
/// Any name ending so is secret too (`token`, `user_token`, `access_token`, `client_secret`).
const SECRET_ENDINGS: &[&str] = &["token", "secret"];

/// `text` with every secret it holds masked.
pub fn redact(text: &str) -> String {
    let text = paths::mask(text, paths::Profile::current());
    mask_emails(&mask_named(&text))
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Where a masked value stops.
fn ends_value(b: u8) -> bool {
    b.is_ascii_whitespace()
        || matches!(
            b,
            b';' | b'&' | b'"' | b'\'' | b',' | b'<' | b'>' | b')' | b']' | b'}'
        )
}

fn is_secret_name(name: &str) -> bool {
    let low = name.to_ascii_lowercase();
    COOKIES.contains(&low.as_str())
        || SECRETS.contains(&low.as_str())
        || SECRET_ENDINGS.iter().any(|e| low.ends_with(e))
        || low.starts_with("__secure-")
        || low.starts_with("__host-")
}

/// Masks the values of `HEADERS`, cookies and `SECRETS`. Names and separators are kept so
/// the line still reads.
fn mask_named(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    // `text[copied..i]` is yet to be copied out.
    let (mut i, mut copied) = (0, 0);
    while i < b.len() {
        if !is_name_byte(b[i]) || (i > 0 && is_name_byte(b[i - 1])) {
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && is_name_byte(b[i]) {
            i += 1;
        }
        let name = &text[start..i];
        // `name"` (a JSON key), spaces, then `=` or `:`.
        let mut j = i;
        if j < b.len() && b[j] == b'"' {
            j += 1;
        }
        while j < b.len() && b[j] == b' ' {
            j += 1;
        }
        let Some(&sep) = b.get(j) else { break };
        if sep != b'=' && sep != b':' {
            continue;
        }
        j += 1;
        let header = sep == b':' && HEADERS.contains(&name.to_ascii_lowercase().as_str());
        if !header && !is_secret_name(name) {
            continue;
        }
        while j < b.len() && b[j] == b' ' {
            j += 1;
        }
        if !header && j < b.len() && (b[j] == b'"' || b[j] == b'\'') {
            j += 1;
        }
        let mut end = j;
        if header {
            while end < b.len() && b[end] != b'\n' && b[end] != b'\r' {
                end += 1;
            }
        } else {
            while end < b.len() && !ends_value(b[end]) {
                end += 1;
            }
        }
        if end > j {
            out.push_str(&text[copied..j]);
            out.push_str(MASK);
            copied = end;
        }
        i = end.max(i);
    }
    out.push_str(&text[copied..]);
    out
}

/// Replaces `local@domain.tld` with `<email>`. A handle (`@name`) has no local part, so it stays.
fn mask_emails(text: &str) -> String {
    let local = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'%' | b'+' | b'-');
    let domain = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-');
    let b = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    for (at, _) in text.match_indices('@') {
        if at < copied {
            continue;
        }
        let mut start = at;
        while start > copied && local(b[start - 1]) {
            start -= 1;
        }
        let mut end = at + 1;
        while end < b.len() && domain(b[end]) {
            end += 1;
        }
        // The domain's last dot is followed by letters (`example.com`, not `x.` or `x`).
        let host = text[at + 1..end].trim_end_matches('.');
        let end = at + 1 + host.len();
        let dotted = host
            .rsplit_once('.')
            .is_some_and(|(name, tld)| !name.is_empty() && tld.len() >= 2);
        if start < at && dotted {
            out.push_str(&text[copied..start]);
            out.push_str("<email>");
            copied = end;
        }
    }
    out.push_str(&text[copied..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each secret is masked; the names, the separators and the rest of the line stay.
    #[test]
    fn secrets_are_masked() {
        let cases = [
            (
                "cookie values",
                "cookies: SAPISID=abc/def123; __Secure-3PSID=g.a000xyz; PREF=f6=4",
                "cookies: SAPISID=***; __Secure-3PSID=***; PREF=f6=4",
            ),
            (
                "a cookie header, to the end of its line",
                "request failed\nCookie: YSC=x1; VISITOR=y2 z3\nstatus 403",
                "request failed\nCookie: ***\nstatus 403",
            ),
            (
                "an authorization header",
                r#"headers {"authorization": "SAPISIDHASH 1700_abcdef", "x": 1}"#,
                r#"headers {"authorization": ***"#,
            ),
            (
                "query secrets",
                "GET https://ws.audioscrobbler.com/2.0/?method=track.scrobble&api_key=K123&sk=S456&api_sig=G789&token=T000&format=json",
                "GET https://ws.audioscrobbler.com/2.0/?method=track.scrobble&api_key=***&sk=***&api_sig=***&token=***&format=json",
            ),
            (
                "json secrets",
                r#"{"session_key": "abc123", "user_token":"lb-0001", "name": "x"}"#,
                r#"{"session_key": "***", "user_token":"***", "name": "x"}"#,
            ),
            (
                "an email address",
                "signed in as ada.lovelace+yt@example.co.uk, ok",
                "signed in as <email>, ok",
            ),
        ];
        for (label, line, want) in cases {
            assert_eq!(redact(line), want, "{label}");
        }
    }

    #[test]
    fn plain_messages_are_left_alone() {
        for line in [
            "resolve h46YpWrg55Q failed in 3.2s: yt-dlp failed (1): ERROR: [youtube] Sign in",
            "resolved abc in 0.3s: itag 140, 129 kbps, WEB_EMBEDDED_PLAYER",
            "the task=7 risk: low @handle",
            "skipping: no key",
            "ünïcödé and 日本語 stay",
        ] {
            assert_eq!(redact(line), line);
        }
    }
}
