//! The plain text that Settings > About's "Copy diagnostics" puts on the clipboard: versions,
//! the output device, which services are on, the last resolver errors, and which step gave the
//! last songs played their stream. Whether an account is signed in, never which one.

/// What the text says. Built by `diagnostics::report`; tests build their own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub version: String,
    /// Which system this is (`diagnostics::os`), on the line `OS_LABEL` names.
    pub os: String,
    /// `None` until the version was read (or when the tool isn't installed).
    pub ytdlp: Option<String>,
    pub deno: Option<String>,
    /// The JavaScript engine built in for YouTube's player challenges, with its version.
    pub js_engine: String,
    /// `None`: the system's default device.
    pub output_device: Option<String>,
    pub signed_in: bool,
    pub lastfm: bool,
    pub listenbrainz: bool,
    pub discord: bool,
    /// The proxy requests go through and where it came from, without credentials
    /// (`featherplay_proxy::summary`); `None`: none.
    pub proxy: Option<String>,
    /// Whether the DNS sends YouTube's hosts away from Google (`featherplay_proxy::dns::Dns`);
    /// `None`: not checked yet.
    pub dns: Option<String>,
    /// "Look up YouTube directly": YouTube's hosts over DNS over HTTPS
    /// (`featherplay_proxy::doh`).
    pub dns_over_https: bool,
    /// The country YouTube last tied a stream to (`diagnostics::stream_region`), `IT`;
    /// `None`: no stream said.
    pub region: Option<String>,
    /// How the last helpers download failed (`SidecarError::summary`); `None`: it didn't.
    pub helpers: Option<String>,
    /// Why the system's credential store wouldn't keep the last sign-in (its error, with its
    /// code); `None`: it did, or nobody signed in this run.
    pub sign_in_save: Option<String>,
    /// The last resolver errors, oldest first, each with its time.
    pub errors: Vec<String>,
    /// The last song starts, oldest first, each with its time and the step that gave its
    /// stream (`featherplay_streams::Via::label`).
    pub played: Vec<String>,
}

impl Report {
    /// One fact per line, then the errors and the song starts (redacted, each on one line).
    pub fn text(&self) -> String {
        let on = |b: bool| if b { "on" } else { "off" };
        let known = |v: &Option<String>| v.clone().unwrap_or_else(|| "unknown".into());
        let mut out = format!(
            "featherplay {}\n\
             {}: {}\n\
             yt-dlp: {}\n\
             Deno: {}\n\
             JavaScript: {} (built in)\n\
             Output device: {}\n\
             Signed in: {}\n\
             Last.fm: {}\n\
             ListenBrainz: {}\n\
             Discord: {}\n\
             Proxy: {}\n\
             DNS: {}\n\
             Look up YouTube directly: {}\n\
             YouTube's region: {}\n\
             Last helpers download error: {}\n\
             Last sign-in save error: {}\n",
            self.version,
            super::OS_LABEL,
            self.os,
            known(&self.ytdlp),
            known(&self.deno),
            self.js_engine,
            self.output_device.as_deref().unwrap_or("System default"),
            if self.signed_in { "yes" } else { "no" },
            on(self.lastfm),
            on(self.listenbrainz),
            on(self.discord),
            self.proxy.as_deref().unwrap_or("none"),
            self.dns.as_deref().unwrap_or("not checked yet"),
            on(self.dns_over_https),
            self.region.as_deref().unwrap_or("unknown"),
            self.helpers.as_deref().unwrap_or("none"),
            self.sign_in_save.as_deref().unwrap_or("none"),
        );
        list(&mut out, "Recent resolver errors", &self.errors);
        list(&mut out, "Recent songs played via", &self.played);
        out
    }
}

/// `title: none`, or `title:` and an indented line for each entry.
fn list(out: &mut String, title: &str, entries: &[String]) {
    out.push_str(title);
    if entries.is_empty() {
        out.push_str(": none\n");
        return;
    }
    out.push_str(":\n");
    for e in entries {
        let one_line = e.lines().map(str::trim).collect::<Vec<_>>().join(" / ");
        out.push_str("  ");
        out.push_str(&super::redact(&one_line));
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows names itself; elsewhere the line holds the distribution's name.
    const OS: &str = if cfg!(windows) { "Windows" } else { "System" };

    /// Every fact known and every service on but one: each has its line, an error stays on
    /// one line, and the last songs name the step that gave their stream.
    #[test]
    fn every_fact_has_its_line() {
        let text = Report {
            version: "0.1.0".into(),
            os: "Windows 11 25H2, build 26300.7171".into(),
            ytdlp: Some("2026.10.26".into()),
            deno: Some("2.5.4".into()),
            js_engine: "QuickJS-NG 0.16.2".into(),
            output_device: Some("Speakers (Realtek(R) Audio)".into()),
            signed_in: true,
            lastfm: true,
            listenbrainz: false,
            discord: true,
            proxy: Some("http://proxy.corp:3128".into()),
            dns: Some(
                "www.youtube.com resolves to 203.0.113.7, outside Google's own ranges \
                 and unlike music.youtube.com (142.250.180.14)"
                    .into(),
            ),
            dns_over_https: true,
            region: Some("IT".into()),
            helpers: Some("blocked, os error 32".into()),
            sign_in_save: Some(
                "credential store: Platform failure: Windows error code 1783".into(),
            ),
            errors: vec![
                "14:03:11 abc: yt-dlp failed (1): ERROR: one\nsecond line".into(),
                "14:05:00 def: unavailable: private video".into(),
            ],
            played: vec![
                "14:06:02 abc: native embedded (QuickJS)".into(),
                "14:09:40 def: yt-dlp default".into(),
            ],
        }
        .text();
        let want = format!(
            "featherplay 0.1.0\n\
             {OS}: Windows 11 25H2, build 26300.7171\n\
             yt-dlp: 2026.10.26\n\
             Deno: 2.5.4\n\
             JavaScript: QuickJS-NG 0.16.2 (built in)\n\
             Output device: Speakers (Realtek(R) Audio)\n\
             Signed in: yes\n\
             Last.fm: on\n\
             ListenBrainz: off\n\
             Discord: on\n\
             Proxy: http://proxy.corp:3128\n\
             DNS: www.youtube.com resolves to 203.0.113.7, outside Google's own ranges \
             and unlike music.youtube.com (142.250.180.14)\n\
             Look up YouTube directly: on\n\
             YouTube's region: IT\n\
             Last helpers download error: blocked, os error 32\n\
             Last sign-in save error: credential store: Platform failure: Windows error code 1783\n\
             Recent resolver errors:\n  \
             14:03:11 abc: yt-dlp failed (1): ERROR: one / second line\n  \
             14:05:00 def: unavailable: private video\n\
             Recent songs played via:\n  \
             14:06:02 abc: native embedded (QuickJS)\n  \
             14:09:40 def: yt-dlp default\n"
        );
        assert_eq!(text, want);
    }

    /// Nothing known yet and everything off: each line says so.
    #[test]
    fn every_unknown_fact_is_said_so() {
        let text = Report {
            version: "0.1.0".into(),
            os: "Windows 11 25H2, build 26300.7171".into(),
            js_engine: "QuickJS-NG 0.16.2".into(),
            ..Report::default()
        }
        .text();
        let want = format!(
            "featherplay 0.1.0\n\
             {OS}: Windows 11 25H2, build 26300.7171\n\
             yt-dlp: unknown\n\
             Deno: unknown\n\
             JavaScript: QuickJS-NG 0.16.2 (built in)\n\
             Output device: System default\n\
             Signed in: no\n\
             Last.fm: off\n\
             ListenBrainz: off\n\
             Discord: off\n\
             Proxy: none\n\
             DNS: not checked yet\n\
             Look up YouTube directly: off\n\
             YouTube's region: unknown\n\
             Last helpers download error: none\n\
             Last sign-in save error: none\n\
             Recent resolver errors: none\n\
             Recent songs played via: none\n"
        );
        assert_eq!(text, want);
    }

    #[test]
    fn folders_secrets_and_emails_in_errors_are_masked() {
        let text = Report {
            errors: vec![
                r"10:00:00 x: no yt-dlp in C:\Users\bob\AppData\Local\x, failed for ada@example.com with SAPISID=s3cr3t".into(),
            ],
            ..Report::default()
        }
        .text();
        for secret in ["bob", "ada@example.com", "s3cr3t"] {
            assert!(!text.contains(secret), "{secret} in\n{text}");
        }
    }
}
