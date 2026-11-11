//! Local time and the OS version, for the log and the diagnostics. The time is UTC until
//! phase 5 reads the time zone.

use std::time::{SystemTime, UNIX_EPOCH};

pub use crate::common::system::*;

/// The wall-clock time now, to the millisecond: UTC for now.
pub fn local_time() -> LocalTime {
    let since = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = since.as_secs();
    let (year, month, day) = civil(secs / 86_400);
    let of_day = secs % 86_400;
    LocalTime {
        year,
        month,
        day,
        hour: (of_day / 3600) as u16,
        minute: (of_day / 60 % 60) as u16,
        second: (of_day % 60) as u16,
        millis: since.subsec_millis() as u16,
    }
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
}
