//! Who may use the API: the checks every request passes before its route runs.

use hyper::HeaderMap;
use hyper::header::{AUTHORIZATION, HOST, HeaderName, ORIGIN};

use super::Token;

/// Why a request was turned away.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    /// The `Host` isn't the loopback address and port: a page reached the server through a
    /// name of its own that resolves to 127.0.0.1 (DNS rebinding).
    Host,
    /// A web page made the request.
    Origin,
    /// No token, or not this one.
    Token,
}

/// Lets a request through when its `Host` names the server, its `Origin` (if any) is no web
/// page, and it carries the token. `query_token`: the token may also come as `?token=` (the
/// WebSocket, which browsers can't give an `Authorization` header).
pub(super) fn check(
    headers: &HeaderMap,
    query: Option<&str>,
    port: u16,
    token: &Token,
    query_token: bool,
) -> Result<(), Refusal> {
    if !single(headers, &HOST).is_some_and(|h| host_ok(h, port)) {
        return Err(Refusal::Host);
    }
    let mut origins = headers.get_all(ORIGIN).iter();
    match (origins.next(), origins.next()) {
        (None, _) => {}
        (Some(o), None) if o.to_str().is_ok_and(origin_ok) => {}
        _ => return Err(Refusal::Origin),
    }
    let given = match bearer(headers) {
        Some(t) => Some(t),
        None if query_token => query.and_then(|q| param(q, "token")),
        None => None,
    };
    match given {
        Some(t) if same(t.as_bytes(), token.as_str().as_bytes()) => Ok(()),
        _ => Err(Refusal::Token),
    }
}

/// The header's value when it's there once and is text.
pub(super) fn single<'h>(headers: &'h HeaderMap, name: &HeaderName) -> Option<&'h str> {
    let mut all = headers.get_all(name).iter();
    match (all.next(), all.next()) {
        (Some(v), None) => v.to_str().ok(),
        _ => None,
    }
}

/// `127.0.0.1:<port>`, `localhost:<port>` or `[::1]:<port>`.
pub(super) fn host_ok(host: &str, port: u16) -> bool {
    let Some((name, p)) = host.rsplit_once(':') else {
        return false;
    };
    p.parse::<u16>() == Ok(port)
        && (name == "127.0.0.1" || name == "[::1]" || name.eq_ignore_ascii_case("localhost"))
}

/// No web page: `null` (a local file, a sandboxed frame) or `file://`.
fn origin_ok(origin: &str) -> bool {
    origin == "null" || origin.eq_ignore_ascii_case("file://")
}

/// The token of `Authorization: Bearer <token>`.
fn bearer(headers: &HeaderMap) -> Option<&str> {
    let (scheme, token) = single(headers, &AUTHORIZATION)?.trim().split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| token.trim())
}

/// The value of `name` in a query string, as sent (tokens are hex: nothing to decode).
fn param<'q>(query: &'q str, name: &str) -> Option<&'q str> {
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
}

/// Compares in time that doesn't depend on where the bytes differ. The length isn't secret:
/// every token featherplay makes has 64 digits.
pub(super) fn same(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let diff = a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y));
    std::hint::black_box(diff) == 0
}

#[cfg(test)]
mod tests {
    use hyper::header::HeaderValue;

    use super::*;

    const PORT: u16 = 26538;

    fn token() -> Token {
        Token::new("ab".repeat(32)).expect("a token")
    }

    fn headers(pairs: &[(HeaderName, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (name, value) in pairs {
            h.append(
                name.clone(),
                HeaderValue::from_str(value).expect("a header value"),
            );
        }
        h
    }

    fn with_token(extra: &[(HeaderName, &str)]) -> HeaderMap {
        let auth = format!("Bearer {}", "ab".repeat(32));
        let mut pairs = vec![(HOST, "127.0.0.1:26538"), (AUTHORIZATION, auth.as_str())];
        pairs.extend_from_slice(extra);
        headers(&pairs)
    }

    #[test]
    fn the_loopback_names_with_the_port_are_good_hosts() {
        for host in [
            "127.0.0.1:26538",
            "localhost:26538",
            "LocalHost:26538",
            "[::1]:26538",
        ] {
            assert!(host_ok(host, PORT), "{host}");
        }
    }

    #[test]
    fn other_hosts_and_ports_are_not() {
        for host in [
            "127.0.0.1",
            "localhost",
            "127.0.0.1:80",
            "evil.example:26538",
            "127.0.0.1.evil.example:26538",
            "localhost.:26538",
            "192.168.1.5:26538",
            "",
        ] {
            assert!(!host_ok(host, PORT), "{host}");
        }
    }

    #[test]
    fn only_local_origins_pass() {
        assert!(origin_ok("null"));
        assert!(origin_ok("file://"));
        assert!(!origin_ok("https://evil.example"));
        assert!(!origin_ok("http://127.0.0.1:26538"));
    }

    #[test]
    fn check_wants_one_good_host_no_web_origin_and_the_token() {
        let auth = format!("Bearer {}", "ab".repeat(32));
        let lower = format!("bearer {}", "ab".repeat(32));
        let query = format!("a=1&token={}", "ab".repeat(32));
        let host = (HOST, "127.0.0.1:26538");
        // (case, headers, query, whether the query's token counts, the answer)
        let cases = [
            ("the token passes", with_token(&[]), None, false, Ok(())),
            (
                "the bearer scheme is case-insensitive",
                headers(&[(HOST, "localhost:26538"), (AUTHORIZATION, &lower)]),
                None,
                false,
                Ok(()),
            ),
            (
                "a missing host is refused before the token",
                headers(&[(AUTHORIZATION, &auth)]),
                None,
                false,
                Err(Refusal::Host),
            ),
            (
                "two hosts are refused",
                headers(&[
                    (HOST, "127.0.0.1:26538"),
                    (HOST, "evil.example:26538"),
                    (AUTHORIZATION, &auth),
                ]),
                None,
                false,
                Err(Refusal::Host),
            ),
            (
                "a web origin is refused even with the token",
                with_token(&[(ORIGIN, "https://evil.example")]),
                None,
                false,
                Err(Refusal::Origin),
            ),
            (
                "the query's token where it counts",
                headers(std::slice::from_ref(&host)),
                Some(&query),
                true,
                Ok(()),
            ),
            (
                "the query's token where it doesn't",
                headers(std::slice::from_ref(&host)),
                Some(&query),
                false,
                Err(Refusal::Token),
            ),
            (
                "a wrong token",
                headers(&[host.clone(), (AUTHORIZATION, "Bearer nope")]),
                None,
                false,
                Err(Refusal::Token),
            ),
            (
                "another scheme",
                headers(&[host.clone(), (AUTHORIZATION, "Basic abc")]),
                None,
                false,
                Err(Refusal::Token),
            ),
            (
                "no token at all",
                headers(&[host]),
                None,
                true,
                Err(Refusal::Token),
            ),
        ];
        for (case, h, query, query_token, expected) in cases {
            let got = check(&h, query.map(String::as_str), PORT, &token(), query_token);
            assert_eq!(got, expected, "{case}");
        }
    }

    #[test]
    fn same_compares_whole_values() {
        assert!(same(b"abcd", b"abcd"));
        assert!(!same(b"abcd", b"abce"));
        assert!(!same(b"abcd", b"abc"));
        assert!(!same(b"", b"a"));
    }
}
