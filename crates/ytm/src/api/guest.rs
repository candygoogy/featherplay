//! The visitor's token, and the web client version it sends.

use std::borrow::Cow;
use std::time::SystemTime;

use ytmapi_rs::auth::{AuthToken, RawResult};
use ytmapi_rs::parse::ProcessedResult;

use super::query::{BROWSER_USER_AGENT, YTM_ORIGIN};

/// A visitor's requests: no cookies and no visitor id, and the web client's version from
/// today's date, as ytmapi-rs's visitor token and ytmusicapi send it. ytmapi-rs 0.3.3's own
/// `NoAuthToken` first reads the YouTube Music page for its `VISITOR_DATA`, which the
/// signed-out page no longer has in the config it reads (2026-11-06: only
/// `EOM_VISITOR_DATA`), so it never gets built; the API answers without one.
#[derive(Clone, Debug)]
pub(super) struct GuestToken;

impl AuthToken for GuestToken {
    fn headers(&self) -> ytmapi_rs::Result<impl IntoIterator<Item = (&str, Cow<'_, str>)>> {
        Ok([
            ("User-Agent", BROWSER_USER_AGENT.into()),
            ("X-Origin", YTM_ORIGIN.into()),
            ("Content-Type", "application/json".into()),
        ])
    }
    /// Worked out for each request, so a long run never sends an old one.
    fn client_version(&self) -> Cow<'_, str> {
        web_client_version(SystemTime::now()).into()
    }
    fn deserialize_response<Q>(raw: RawResult<Q, Self>) -> ytmapi_rs::Result<ProcessedResult<Q>> {
        ProcessedResult::try_from(raw)
    }
}

/// Whether `v` can be a web client version (`1.20261106.01.00`): digits and dots, short.
pub(super) fn plausible_version(v: &str) -> bool {
    (1..=32).contains(&v.len())
        && v.starts_with(|c: char| c.is_ascii_digit())
        && v.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// The web client's version for `now`: `1.YYYYMMDD.01.00` (UTC).
pub(super) fn web_client_version(now: SystemTime) -> String {
    let days = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0);
    let (y, m, d) = civil_date(days);
    format!("1.{y:04}{m:02}{d:02}.01.00")
}

/// The calendar date `days` after 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil_date(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + u64::from(m <= 2);
    (y, m, d)
}
