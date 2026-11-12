//! Local time and the OS version, for the log and the diagnostics.

use std::time::{SystemTime, UNIX_EPOCH};

pub use crate::common::system::*;

/// The wall-clock time now, to the millisecond, in the system's time zone (`TZ`, else
/// `/etc/localtime`); UTC if the C library can't say.
pub fn local_time() -> LocalTime {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let millis = since.subsec_millis() as u16;
    let secs = since.as_secs();
    if let Some(t) = local(secs) {
        return LocalTime { millis, ..t };
    }
    let (year, month, day) = civil(secs / 86_400);
    let of_day = secs % 86_400;
    LocalTime {
        year,
        month,
        day,
        hour: (of_day / 3600) as u16,
        minute: (of_day / 60 % 60) as u16,
        second: (of_day % 60) as u16,
        millis,
    }
}

/// `secs` since 1970 in the local time zone (`localtime_r`), to the second.
fn local(secs: u64) -> Option<LocalTime> {
    let t = libc::time_t::try_from(secs).ok()?;
    // SAFETY: an all-zero `tm` is a valid value of the plain C struct.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: valid pointers to locals; `localtime_r` is the thread-safe form.
    if unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
        return None;
    }
    let field = |v: libc::c_int| u16::try_from(v).ok();
    Some(LocalTime {
        year: field(tm.tm_year + 1900)?,
        month: field(tm.tm_mon + 1)?,
        day: field(tm.tm_mday)?,
        hour: field(tm.tm_hour)?,
        minute: field(tm.tm_min)?,
        // A leap second reads as 60.
        second: field(tm.tm_sec.min(59))?,
        millis: 0,
    })
}

/// The year, month and day of a count of days since 1970-01-01 (Howard Hinnant's
/// `civil_from_days`, for dates from 1970 on).
fn civil(days: u64) -> (u16, u16, u16) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year as u16, month as u16, day as u16)
}

/// The distribution's name from `/etc/os-release` (`PRETTY_NAME`), else `Linux`.
pub fn os_version() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                let value = line.strip_prefix("PRETTY_NAME=")?.trim().trim_matches('"');
                (!value.is_empty()).then(|| value.to_string())
            })
        })
        .unwrap_or_else(|| "Linux".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_since_1970_are_dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(19_723), (2024, 1, 1));
        assert_eq!(civil(20_759), (2026, 11, 2));
    }

    #[test]
    fn the_local_time_is_a_date() {
        let t = local_time();
        assert!(t.year >= 2026 && (1..=12).contains(&t.month) && (1..=31).contains(&t.day));
        assert!(t.hour < 24 && t.minute < 60 && t.second < 60 && t.millis < 1000);
    }
}
