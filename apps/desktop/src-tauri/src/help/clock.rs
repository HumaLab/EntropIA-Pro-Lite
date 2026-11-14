//! The device's local date and time (S22, D19): no new crates — `chrono`
//! and `time` are only transitive (present in `Cargo.lock` through other
//! crates, not declared in `Cargo.toml`, which this task must not touch) —
//! so the C library's localtime does it: `_localtime64_s` on Windows (which
//! follows the Windows time zone) and `localtime_r` elsewhere (which honors
//! `TZ` and the system zone), over `SystemTime` seconds. `now_local()` is
//! the production source; tests build [`LocalDateTime`] directly, so every
//! format is deterministic.

use std::os::raw::c_int;
#[cfg(not(windows))]
use std::os::raw::{c_char, c_long};
use std::time::{SystemTime, UNIX_EPOCH};

/// A weekday, Monday first for the date maths, named in both languages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl Weekday {
    /// 0 = Monday, matching the date maths in [`days_to_civil`].
    fn from_monday_based(index: u64) -> Weekday {
        match index % 7 {
            0 => Weekday::Monday,
            1 => Weekday::Tuesday,
            2 => Weekday::Wednesday,
            3 => Weekday::Thursday,
            4 => Weekday::Friday,
            5 => Weekday::Saturday,
            _ => Weekday::Sunday,
        }
    }

    /// 0 = Sunday, matching `tm_wday`.
    fn from_sunday_based(index: i32) -> Option<Weekday> {
        match index {
            0 => Some(Weekday::Sunday),
            1 => Some(Weekday::Monday),
            2 => Some(Weekday::Tuesday),
            3 => Some(Weekday::Wednesday),
            4 => Some(Weekday::Thursday),
            5 => Some(Weekday::Friday),
            6 => Some(Weekday::Saturday),
            _ => None,
        }
    }

    pub(crate) fn spanish(&self) -> &'static str {
        match self {
            Weekday::Monday => "lunes",
            Weekday::Tuesday => "martes",
            Weekday::Wednesday => "miércoles",
            Weekday::Thursday => "jueves",
            Weekday::Friday => "viernes",
            Weekday::Saturday => "sábado",
            Weekday::Sunday => "domingo",
        }
    }

    pub(crate) fn english(&self) -> &'static str {
        match self {
            Weekday::Monday => "Monday",
            Weekday::Tuesday => "Tuesday",
            Weekday::Wednesday => "Wednesday",
            Weekday::Thursday => "Thursday",
            Weekday::Friday => "Friday",
            Weekday::Saturday => "Saturday",
            Weekday::Sunday => "Sunday",
        }
    }
}

/// Spanish month names, 1-based.
const MONTHS_ES: &[&str] = &[
    "enero",
    "febrero",
    "marzo",
    "abril",
    "mayo",
    "junio",
    "julio",
    "agosto",
    "septiembre",
    "octubre",
    "noviembre",
    "diciembre",
];

/// English month names, 1-based.
const MONTHS_EN: &[&str] = &[
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// A local date and time with every field already resolved: production gets
/// it from [`now_local`], tests build it literally.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LocalDateTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub weekday: Weekday,
    pub hour: u8,
    pub minute: u8,
}

/// «viernes 14 de noviembre de 2026».
pub(crate) fn date_es(now: &LocalDateTime) -> String {
    format!(
        "{} {} de {} de {}",
        now.weekday.spanish(),
        now.day,
        MONTHS_ES
            .get(now.month.saturating_sub(1) as usize)
            .unwrap_or(&"?"),
        now.year
    )
}

/// «Friday, November 14, 2026».
pub(crate) fn date_en(now: &LocalDateTime) -> String {
    format!(
        "{}, {} {}, {}",
        now.weekday.english(),
        MONTHS_EN
            .get(now.month.saturating_sub(1) as usize)
            .unwrap_or(&"?"),
        now.day,
        now.year
    )
}

/// «14:05», 24-hour, zero-padded.
pub(crate) fn time_es(now: &LocalDateTime) -> String {
    format!("{:02}:{:02}", now.hour, now.minute)
}

/// «2:05 PM», 12-hour.
pub(crate) fn time_en(now: &LocalDateTime) -> String {
    let hour_12 = match now.hour % 12 {
        0 => 12,
        hour => hour,
    };
    let period = if now.hour < 12 { "AM" } else { "PM" };
    format!("{}:{:02} {}", hour_12, now.minute, period)
}

/// The device's local date and time: `SystemTime` seconds rendered through
/// the OS C library, so the answer follows the user's clock. Only the C
/// localtime is platform-specific; everything else is shared. When the
/// platform call fails (essentially never), UTC rendering keeps the answer
/// working rather than erroring the question.
pub(crate) fn now_local() -> LocalDateTime {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|ago| ago.as_secs() as i64)
        .unwrap_or(0);
    decompose_local(secs).unwrap_or_else(|| utc_fallback(secs))
}

#[cfg(windows)]
#[repr(C)]
struct NativeTm {
    sec: c_int,
    min: c_int,
    hour: c_int,
    mday: c_int,
    mon: c_int,
    year: c_int,
    wday: c_int,
    yday: c_int,
    isdst: c_int,
}

// The UCRT localtime the lean build already links: no new crate, no new
// Cargo feature, just the symbol out of the C runtime.
#[cfg(windows)]
extern "C" {
    fn _localtime64_s(tm: *mut NativeTm, time: *const i64) -> c_int;
}

#[cfg(not(windows))]
#[repr(C)]
struct NativeTm {
    sec: c_int,
    min: c_int,
    hour: c_int,
    mday: c_int,
    mon: c_int,
    year: c_int,
    wday: c_int,
    yday: c_int,
    isdst: c_int,
    // `long` and a pointer, like the platform `struct tm` on 64-bit Unix:
    // the call writes past the 9 ints, so the shape must hold them.
    gmtoff: c_long,
    zone: *const c_char,
}

#[cfg(not(windows))]
extern "C" {
    fn localtime_r(time: *const i64, tm: *mut NativeTm) -> *mut NativeTm;
}

/// Renders `secs` (Unix seconds) in the device zone, or `None` when the
/// platform call fails or hands back out-of-range fields.
fn decompose_local(secs: i64) -> Option<LocalDateTime> {
    // Zeroed first: the struct is only read after a successful call.
    let mut native: NativeTm = unsafe { std::mem::zeroed() };
    #[cfg(windows)]
    let ok = unsafe { _localtime64_s(&mut native, &secs) == 0 };
    #[cfg(not(windows))]
    let ok = unsafe { !localtime_r(&secs, &mut native).is_null() };
    if !ok {
        return None;
    }
    if !(0..=11).contains(&native.mon)
        || !(1..=31).contains(&native.mday)
        || !(0..=23).contains(&native.hour)
        || !(0..=59).contains(&native.min)
    {
        return None;
    }
    Some(LocalDateTime {
        year: native.year + 1900,
        month: (native.mon + 1) as u8,
        day: native.mday as u8,
        weekday: Weekday::from_sunday_based(native.wday)?,
        hour: native.hour as u8,
        minute: native.min as u8,
    })
}

/// UTC rendering of `secs` for the impossible case of a failed localtime:
/// Howard Hinnant's days-from-civil algorithm over whole days plus the
/// clock fields straight from the remainder. 1970-01-01 was a Thursday.
fn utc_fallback(secs: i64) -> LocalDateTime {
    let days = secs.div_euclid(86_400);
    let (year, month, day) = days_to_civil(days);
    let clock = secs.rem_euclid(86_400);
    LocalDateTime {
        year,
        month,
        day,
        // Day 0 (1970-01-01) was a Thursday: Monday-based index 3.
        weekday: Weekday::from_monday_based((days + 3).rem_euclid(7) as u64),
        hour: (clock / 3600) as u8,
        minute: ((clock % 3600) / 60) as u8,
    }
}

/// Days since 1970-01-01 to (year, month, day).
fn days_to_civil(days: i64) -> (i32, u8, u8) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = (year_of_era + era * 400) as i32;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_part = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_part + 2) / 5 + 1) as u8;
    let month = (if month_part < 10 {
        month_part + 3
    } else {
        month_part - 9
    }) as u8;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed() -> LocalDateTime {
        // Saturday, 14 November 2026, 14:05.
        LocalDateTime {
            year: 2026,
            month: 11,
            day: 14,
            weekday: Weekday::Saturday,
            hour: 14,
            minute: 5,
        }
    }

    #[test]
    fn spanish_date_and_time_match_the_owner_example_shape() {
        // S22: «Son las 14:05 del viernes 14 de noviembre de 2026.»
        let now = LocalDateTime {
            weekday: Weekday::Friday,
            ..fixed()
        };
        assert_eq!(date_es(&now), "viernes 14 de noviembre de 2026");
        assert_eq!(time_es(&now), "14:05");
    }

    #[test]
    fn english_date_and_time_use_twelve_hours() {
        let now = fixed();
        assert_eq!(date_en(&now), "Saturday, November 14, 2026");
        assert_eq!(time_en(&now), "2:05 PM");
    }

    #[test]
    fn midnight_and_noon_use_twelve() {
        let mut now = fixed();
        now.hour = 0;
        now.minute = 0;
        assert_eq!(time_en(&now), "12:00 AM");
        now.hour = 12;
        assert_eq!(time_en(&now), "12:00 PM");
        assert_eq!(time_es(&now), "12:00");
        now.hour = 9;
        now.minute = 7;
        assert_eq!(time_es(&now), "09:07");
        assert_eq!(time_en(&now), "9:07 AM");
    }

    #[test]
    fn utc_fallback_renders_a_known_day() {
        // 2026-11-14 00:00:00 UTC is a Saturday.
        let rendered = utc_fallback(1_794_614_400);
        assert_eq!(rendered.year, 2026);
        assert_eq!(rendered.month, 11);
        assert_eq!(rendered.day, 14);
        assert_eq!(rendered.weekday, Weekday::Saturday);
        assert_eq!(rendered.hour, 0);
        assert_eq!(rendered.minute, 0);
    }

    #[test]
    fn utc_fallback_matches_the_epoch() {
        let rendered = utc_fallback(0);
        assert_eq!(
            rendered,
            LocalDateTime {
                year: 1970,
                month: 1,
                day: 1,
                weekday: Weekday::Thursday,
                hour: 0,
                minute: 0,
            }
        );
    }

    #[test]
    fn now_local_returns_sane_fields() {
        // The only non-deterministic clock test: it asserts shape, never a
        // value, so it holds in every zone.
        let now = now_local();
        assert!((2020..2100).contains(&now.year), "{now:?}");
        assert!((1..=12).contains(&now.month), "{now:?}");
        assert!((1..=31).contains(&now.day), "{now:?}");
        assert!(now.hour < 24, "{now:?}");
        assert!(now.minute < 60, "{now:?}");
    }
}
