//! The local wall-clock time, for log lines and crash file names.

/// A local date and time to the millisecond.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stamp {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub millis: u16,
}

#[repr(C)]
#[derive(Default)]
struct SystemTime {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    millis: u16,
}

// Declared here rather than turning on another `windows` feature.
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetLocalTime(time: *mut SystemTime);
}

impl Stamp {
    pub fn now() -> Stamp {
        let mut t = SystemTime::default();
        // SAFETY: a valid out pointer to a SYSTEMTIME-shaped struct.
        unsafe { GetLocalTime(&mut t) };
        Stamp {
            year: t.year,
            month: t.month,
            day: t.day,
            hour: t.hour,
            minute: t.minute,
            second: t.second,
            millis: t.millis,
        }
    }

    /// `2026-11-02 14:03:11.123`, for log lines.
    pub fn line(&self) -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
            self.year, self.month, self.day, self.hour, self.minute, self.second, self.millis
        )
    }

    /// `20261102-140311`, for file names.
    pub fn file(&self) -> String {
        format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }

    /// `14:03:11`.
    pub fn time(&self) -> String {
        format!("{:02}:{:02}:{:02}", self.hour, self.minute, self.second)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: Stamp = Stamp {
        year: 2026,
        month: 11,
        day: 2,
        hour: 4,
        minute: 3,
        second: 9,
        millis: 7,
    };

    #[test]
    fn formats_pad_every_field() {
        assert_eq!(AT.line(), "2026-11-02 04:03:09.007");
        assert_eq!(AT.file(), "20261102-040309");
        assert_eq!(AT.time(), "04:03:09");
    }

    #[test]
    fn now_is_a_plausible_date() {
        let now = Stamp::now();
        assert!(now.year >= 2024 && (1..=12).contains(&now.month) && (1..=31).contains(&now.day));
    }
}
