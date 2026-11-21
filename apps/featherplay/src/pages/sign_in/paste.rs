//! What "Paste a cookie header" takes besides a bare header: a browser's whole "Copy as cURL"
//! command (bash, or Windows cmd's `^"…^"`), its "Copy as PowerShell" command, or a Netscape
//! cookies.txt file. Each becomes the `cookie:` header it carries; a bare header is passed on
//! as it is. The shapes are the ones Chromium's DevTools (`NetworkLogView.generateCurlCommand`
//! and `generatePowerShellCommand`) and Firefox's (`devtools/client/shared/curl.js`,
//! `netmonitor/src/utils/powershell.js`) write. The pasted text is never logged.

use std::borrow::Cow;

use featherplay_core::tr;

/// The cookie header in `text`. `now` is the wall clock, in seconds since the Unix epoch,
/// for the cookies.txt file's expired cookies.
pub fn cookie_text(text: &str, now: i64) -> Result<Cow<'_, str>, &'static str> {
    let t = text.trim_start();
    let first = t.split_whitespace().next().unwrap_or_default();
    let cookies = if first.eq_ignore_ascii_case("curl") || first.eq_ignore_ascii_case("curl.exe") {
        let cmd = t.contains("^\"") || t.contains(" ^\n") || t.contains(" ^\r\n");
        curl(&if cmd { cmd_args(t) } else { posix_args(t) })?
    } else if t.contains("System.Net.Cookie(") || first.eq_ignore_ascii_case("Invoke-WebRequest") {
        powershell(t)?
    } else if t.lines().any(|l| l.split('\t').count() == 7) {
        cookies_txt(t, now)
    } else {
        return Ok(Cow::Borrowed(text));
    };
    if cookies.is_empty() {
        return Err(tr!(
            "That has no cookies for YouTube Music. Copy it from a tab that is signed in."
        ));
    }
    Ok(Cow::Owned(cookies.join("; ")))
}

fn other_site() -> &'static str {
    tr!("That command is for another site. Copy it from a YouTube Music tab.")
}

/// The cookies of a cURL command (`-b`, `--cookie`, or a `cookie:` header) for YouTube.
fn curl(args: &[String]) -> Result<Vec<String>, &'static str> {
    let (mut url, mut cookies) = (None, Vec::new());
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--url" => url = it.next().or(url),
            "-b" | "--cookie" => cookies.extend(it.next().cloned()),
            "-H" | "--header" => {
                if let Some((name, value)) = it.next().and_then(|h| h.split_once(':'))
                    && name.trim().eq_ignore_ascii_case("cookie")
                {
                    cookies.push(value.trim().to_owned());
                }
            }
            _ if a.starts_with('-') => {}
            _ => url = url.or(Some(a)),
        }
    }
    if !url.is_some_and(|u| for_youtube(u)) {
        return Err(other_site());
    }
    Ok(cookies)
}

/// `$session.Cookies.Add((New-Object System.Net.Cookie("name", "value", "/", "domain")))`
/// lines and `-Uri "…"`.
fn powershell(t: &str) -> Result<Vec<String>, &'static str> {
    let url = t.split_once("-Uri ").map(|(_, rest)| ps_string(rest).0);
    if !url.is_some_and(|u| for_youtube(&u)) {
        return Err(other_site());
    }
    let mut cookies = Vec::new();
    for part in t.split("System.Net.Cookie(").skip(1) {
        let (name, rest) = ps_string(part);
        let (value, _) = ps_string(rest.trim_start().trim_start_matches(','));
        cookies.push(format!("{name}={value}"));
    }
    Ok(cookies)
}

/// The double-quoted PowerShell string `s` starts with (backticks escape), and what follows.
fn ps_string(s: &str) -> (String, &str) {
    let Some(body) = s.trim_start().strip_prefix('"') else {
        return (String::new(), s);
    };
    let (mut out, mut chars) = (String::new(), body.char_indices());
    while let Some((i, c)) = chars.next() {
        match c {
            '`' => out.extend(chars.next().map(|(_, c)| c)),
            '"' => return (out, &body[i + 1..]),
            c => out.push(c),
        }
    }
    (out, "")
}

/// The URL is on a `youtube.com` host.
fn for_youtube(url: &str) -> bool {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest
        .split(['/', '?', '#', ':', '\\'])
        .next()
        .unwrap_or_default();
    let host = host.to_ascii_lowercase();
    host == "youtube.com" || host.ends_with(".youtube.com")
}

/// A Netscape cookies.txt file: `domain flag path secure expires name value`, tab-separated;
/// `#HttpOnly_` before an HttpOnly cookie's domain, other `#` lines comments. Keeps the
/// unexpired cookies that go to music.youtube.com; of two with one name, the last.
fn cookies_txt(t: &str, now: i64) -> Vec<String> {
    let mut out: Vec<(&str, &str)> = Vec::new();
    for line in t.lines() {
        let line = line.trim_end_matches('\r');
        let line = line.strip_prefix("#HttpOnly_").unwrap_or(line);
        let f: Vec<&str> = line.split('\t').collect();
        let [domain, _, _, _, expires, name, value] = f[..] else {
            continue;
        };
        let domain = domain.trim().trim_start_matches('.');
        let ours = ["youtube.com", "music.youtube.com"]
            .iter()
            .any(|d| domain.eq_ignore_ascii_case(d));
        // 0 (or nothing) is a session cookie.
        let expires = expires.trim().parse::<f64>().unwrap_or(0.0);
        if line.starts_with('#')
            || !ours
            || name.is_empty()
            || expires > 0.0 && expires <= now as f64
        {
            continue;
        }
        out.retain(|(n, _)| *n != name);
        out.push((name, value));
    }
    out.into_iter().map(|(n, v)| format!("{n}={v}")).collect()
}

/// A POSIX shell's words: `'…'`, `$'…'` (ANSI-C escapes), `"…"`, `\` and `\`-newline.
fn posix_args(t: &str) -> Vec<String> {
    let (mut args, mut cur, mut chars) = (Vec::new(), None::<String>, t.chars().peekable());
    while let Some(c) = chars.next() {
        if c.is_whitespace() {
            args.extend(cur.take());
            continue;
        }
        let word = cur.get_or_insert_with(String::new);
        match c {
            '\\' => match chars.next() {
                Some('\n') | None => {}
                Some('\r') => _ = chars.next_if_eq(&'\n'),
                Some(c) => word.push(c),
            },
            '\'' => word.extend(chars.by_ref().take_while(|&c| c != '\'')),
            '$' if chars.next_if_eq(&'\'').is_some() => {
                while let Some(c) = chars.next() {
                    match c {
                        '\'' => break,
                        '\\' => ansi_c_escape(&mut chars, word),
                        c => word.push(c),
                    }
                }
            }
            '"' => {
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => match chars.next_if(|c| matches!(c, '"' | '\\' | '$' | '`' | '\n'))
                        {
                            Some('\n') => {}
                            Some(c) => word.push(c),
                            None => word.push('\\'),
                        },
                        c => word.push(c),
                    }
                }
            }
            c => word.push(c),
        }
    }
    args.extend(cur);
    args
}

/// One escape of bash's `$'…'`, after its backslash.
fn ansi_c_escape(chars: &mut std::iter::Peekable<std::str::Chars>, word: &mut String) {
    let first = chars.next();
    let mut code = |radix: u32, max: usize, start: u32| {
        let mut n = start;
        for _ in 0..max {
            let Some(d) = chars.peek().and_then(|c| c.to_digit(radix)) else {
                break;
            };
            n = n * radix + d;
            chars.next();
        }
        char::from_u32(n).unwrap_or(char::REPLACEMENT_CHARACTER)
    };
    match first {
        Some('n') => word.push('\n'),
        Some('r') => word.push('\r'),
        Some('t') => word.push('\t'),
        Some('x') => word.push(code(16, 2, 0)),
        Some('u') => word.push(code(16, 4, 0)),
        Some('U') => word.push(code(16, 8, 0)),
        Some(d @ '0'..='7') => word.push(code(8, 2, d as u32 - '0' as u32)),
        Some(c) => word.push(c),
        None => {}
    }
}

/// Windows cmd's words: cmd.exe drops its `^` escapes (and `^`-newline) outside quotes,
/// then the C runtime splits the line (`"…"`, `""` inside quotes, `\"`).
fn cmd_args(t: &str) -> Vec<String> {
    let (mut line, mut quoted, mut chars) = (String::new(), false, t.chars().peekable());
    while let Some(c) = chars.next() {
        match c {
            '^' if !quoted => match chars.next() {
                Some('\r') => _ = chars.next_if_eq(&'\n'),
                Some('\n') | None => {}
                Some(c) => line.push(c),
            },
            c => {
                quoted ^= c == '"';
                line.push(c);
            }
        }
    }
    let (mut args, mut cur, mut quoted, mut chars) =
        (Vec::new(), None::<String>, false, line.chars().peekable());
    while let Some(c) = chars.next() {
        if c.is_whitespace() && !quoted {
            args.extend(cur.take());
            continue;
        }
        let word = cur.get_or_insert_with(String::new);
        match c {
            '\\' => {
                let mut n = 1;
                while chars.next_if_eq(&'\\').is_some() {
                    n += 1;
                }
                if chars.peek() == Some(&'"') {
                    word.extend(std::iter::repeat_n('\\', n / 2));
                    if n % 2 == 1 {
                        word.push('"');
                        chars.next();
                    }
                } else {
                    word.extend(std::iter::repeat_n('\\', n));
                }
            }
            '"' if quoted && chars.next_if_eq(&'"').is_some() => word.push('"'),
            '"' => quoted = !quoted,
            c => word.push(c),
        }
    }
    args.extend(cur);
    args
}

#[cfg(test)]
mod tests;
