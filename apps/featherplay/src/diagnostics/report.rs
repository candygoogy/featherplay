//! The plain text that Settings > About's "Copy diagnostics" puts on the clipboard: versions,
//! the output device, which services are on, and the last resolver errors. Whether an account
//! is signed in, never which one.

/// What the text says. Built by `diagnostics::report`; tests build their own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub version: String,
    pub windows: String,
    /// `None` until the version was read (or when the tool isn't installed).
    pub ytdlp: Option<String>,
    pub deno: Option<String>,
    /// `None`: the system's default device.
    pub output_device: Option<String>,
    pub signed_in: bool,
    pub lastfm: bool,
    pub listenbrainz: bool,
    pub discord: bool,
    /// The proxy requests go through and where it came from, without credentials
    /// (`featherplay_proxy::summary`); `None`: none.
    pub proxy: Option<String>,
    /// How the last helpers download failed (`SidecarError::summary`); `None`: it didn't.
    pub helpers: Option<String>,
    /// Why Windows wouldn't store the last sign-in (the credential store's error, with its
    /// code); `None`: it did, or nobody signed in this run.
    pub sign_in_save: Option<String>,
    /// The last resolver errors, oldest first, each with its time.
    pub errors: Vec<String>,
}

impl Report {
    /// One fact per line, then the errors (redacted, each on one line).
    pub fn text(&self) -> String {
        let on = |b: bool| if b { "on" } else { "off" };
        let known = |v: &Option<String>| v.clone().unwrap_or_else(|| "unknown".into());
        let mut out = format!(
            "featherplay {}\n\
             Windows: {}\n\
             yt-dlp: {}\n\
             Deno: {}\n\
             Output device: {}\n\
             Signed in: {}\n\
             Last.fm: {}\n\
             ListenBrainz: {}\n\
             Discord: {}\n\
             Proxy: {}\n\
             Last helpers download error: {}\n\
             Last sign-in save error: {}\n",
            self.version,
            self.windows,
            known(&self.ytdlp),
            known(&self.deno),
            self.output_device.as_deref().unwrap_or("System default"),
            if self.signed_in { "yes" } else { "no" },
            on(self.lastfm),
            on(self.listenbrainz),
            on(self.discord),
            self.proxy.as_deref().unwrap_or("none"),
            self.helpers.as_deref().unwrap_or("none"),
            self.sign_in_save.as_deref().unwrap_or("none"),
        );
        if self.errors.is_empty() {
            out.push_str("Recent resolver errors: none\n");
        } else {
            out.push_str("Recent resolver errors:\n");
            for e in &self.errors {
                let one_line = e.lines().map(str::trim).collect::<Vec<_>>().join(" / ");
                out.push_str("  ");
                out.push_str(&super::redact(&one_line));
                out.push('\n');
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Report {
        Report {
            version: "0.1.0".into(),
            windows: "Windows 11 25H2, build 26300.7171".into(),
            ytdlp: Some("2026.10.26".into()),
            deno: None,
            output_device: Some("Speakers (Realtek(R) Audio)".into()),
            signed_in: true,
            lastfm: true,
            listenbrainz: false,
            discord: true,
            proxy: None,
            helpers: None,
            sign_in_save: None,
            errors: vec![
                "14:03:11 abc: yt-dlp failed (1): ERROR: one\nsecond line".into(),
                "14:05:00 def: unavailable: private video".into(),
            ],
        }
    }

    #[test]
    fn every_fact_has_its_line() {
        let text = report().text();
        for line in [
            "featherplay 0.1.0",
            "Windows: Windows 11 25H2, build 26300.7171",
            "yt-dlp: 2026.10.26",
            "Deno: unknown",
            "Output device: Speakers (Realtek(R) Audio)",
            "Signed in: yes",
            "Last.fm: on",
            "ListenBrainz: off",
            "Discord: on",
            "Recent resolver errors:",
            "  14:05:00 def: unavailable: private video",
        ] {
            assert!(text.lines().any(|l| l == line), "no {line:?} in\n{text}");
        }
    }

    #[test]
    fn an_error_stays_on_one_line() {
        let text = report().text();
        assert!(
            text.lines()
                .any(|l| l == "  14:03:11 abc: yt-dlp failed (1): ERROR: one / second line"),
            "{text}"
        );
    }

    #[test]
    fn the_default_device_and_no_errors_are_said_so() {
        let text = Report {
            output_device: None,
            errors: Vec::new(),
            ..report()
        }
        .text();
        assert!(text.contains("Output device: System default"), "{text}");
        assert!(text.contains("Recent resolver errors: none"), "{text}");
    }

    #[test]
    fn the_proxy_and_the_last_helpers_error_have_their_lines() {
        let text = Report {
            proxy: Some("http://proxy.corp:3128".into()),
            helpers: Some("blocked, os error 32".into()),
            ..report()
        }
        .text();
        for line in [
            "Proxy: http://proxy.corp:3128",
            "Last helpers download error: blocked, os error 32",
        ] {
            assert!(text.lines().any(|l| l == line), "no {line:?} in\n{text}");
        }
        let text = report().text();
        assert!(text.contains("Proxy: none\n"), "{text}");
        assert!(
            text.contains("Last helpers download error: none\n"),
            "{text}"
        );
    }

    #[test]
    fn a_sign_in_that_wasnt_saved_has_its_line() {
        let text = Report {
            sign_in_save: Some(
                "credential store: Platform failure: Windows error code 1783".into(),
            ),
            ..report()
        }
        .text();
        let line =
            "Last sign-in save error: credential store: Platform failure: Windows error code 1783";
        assert!(text.lines().any(|l| l == line), "no {line:?} in\n{text}");
        let text = report().text();
        assert!(text.contains("Last sign-in save error: none\n"), "{text}");
    }

    #[test]
    fn a_users_folder_in_an_error_is_masked() {
        let text = Report {
            errors: vec![r"10:00:00 x: no yt-dlp in C:\Users\bob\AppData\Local\x".into()],
            ..report()
        }
        .text();
        assert!(!text.contains("bob"), "{text}");
    }

    #[test]
    fn secrets_and_emails_in_errors_are_masked() {
        let text = Report {
            errors: vec!["10:00:00 x: failed for ada@example.com with SAPISID=s3cr3t".into()],
            ..report()
        }
        .text();
        assert!(
            !text.contains("ada@example.com") && !text.contains("s3cr3t"),
            "{text}"
        );
    }
}
