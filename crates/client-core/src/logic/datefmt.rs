//! 4.6 Date and time, default Australian (`en-AU`): `DD/MM/YYYY`, "Today", "Yesterday",
//! weekday names within a week, `3:45 pm` (12-hour) with a 24-hour option, weeks start Monday.
//! Server times are UTC milliseconds; the caller supplies the device's UTC offset in seconds.

const DAY_MS: i64 = 86_400_000;

pub const WEEKDAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
pub const WEEKDAYS_SHORT: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
pub const MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December",
];

/// Days since 1970-01-01 -> (year, month 1-12, day 1-31).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// (year, month, day) -> days since 1970-01-01.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Local day number of a UTC millisecond timestamp.
pub fn local_day(ts_ms: i64, offset_secs: i32) -> i64 {
    (ts_ms + offset_secs as i64 * 1000).div_euclid(DAY_MS)
}

/// Weekday with Monday = 0 .. Sunday = 6.
pub fn weekday_monday0(day: i64) -> usize {
    // 1970-01-01 was a Thursday (index 3).
    (day + 3).rem_euclid(7) as usize
}

pub fn local_hm(ts_ms: i64, offset_secs: i32) -> (u32, u32) {
    let ms = (ts_ms + offset_secs as i64 * 1000).rem_euclid(DAY_MS);
    ((ms / 3_600_000) as u32, ((ms / 60_000) % 60) as u32)
}

/// `3:45 pm` or `15:45`.
pub fn format_time(ts_ms: i64, offset_secs: i32, hour24: bool) -> String {
    let (h, m) = local_hm(ts_ms, offset_secs);
    if hour24 {
        format!("{h:02}:{m:02}")
    } else {
        let suffix = if h >= 12 { "pm" } else { "am" };
        let h12 = match h % 12 {
            0 => 12,
            x => x,
        };
        format!("{h12}:{m:02} {suffix}")
    }
}

/// `DD/MM/YYYY`.
pub fn format_date(ts_ms: i64, offset_secs: i32) -> String {
    let (y, m, d) = civil_from_days(local_day(ts_ms, offset_secs));
    format!("{d:02}/{m:02}/{y:04}")
}

/// Day divider text: "Today", "Yesterday", a weekday name within the last week, else `DD/MM/YYYY`.
pub fn day_label(ts_ms: i64, now_ms: i64, offset_secs: i32) -> String {
    let (d, today) = (local_day(ts_ms, offset_secs), local_day(now_ms, offset_secs));
    match today - d {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        2..=6 => WEEKDAYS[weekday_monday0(d)].into(),
        _ => format_date(ts_ms, offset_secs),
    }
}

/// Short stamp for the conversation list: time today, "Yesterday", short weekday, else date.
pub fn list_stamp(ts_ms: i64, now_ms: i64, offset_secs: i32, hour24: bool) -> String {
    let (d, today) = (local_day(ts_ms, offset_secs), local_day(now_ms, offset_secs));
    match today - d {
        0 => format_time(ts_ms, offset_secs, hour24),
        1 => "Yesterday".into(),
        2..=6 => WEEKDAYS_SHORT[weekday_monday0(d)].into(),
        _ => format_date(ts_ms, offset_secs),
    }
}

/// "Last active 3:45 pm", "Last active yesterday at 3:45 pm", ... (P-05).
pub fn last_active(ts_ms: i64, now_ms: i64, offset_secs: i32, hour24: bool) -> String {
    let (d, today) = (local_day(ts_ms, offset_secs), local_day(now_ms, offset_secs));
    let time = format_time(ts_ms, offset_secs, hour24);
    match today - d {
        0 => format!("Last active {time}"),
        1 => format!("Last active yesterday at {time}"),
        2..=6 => format!("Last active {} at {time}", WEEKDAYS[weekday_monday0(d)]),
        _ => format!("Last active {} at {time}", format_date(ts_ms, offset_secs)),
    }
}

/// Parses `DD/MM/YYYY` into the UTC millisecond timestamp of local midnight (for "jump to date").
pub fn parse_date(s: &str, offset_secs: i32) -> Option<i64> {
    let mut it = s.trim().split('/');
    let d: u32 = it.next()?.trim().parse().ok()?;
    let m: u32 = it.next()?.trim().parse().ok()?;
    let y: i64 = it.next()?.trim().parse().ok()?;
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) || !(1900..=3000).contains(&y) {
        return None;
    }
    let days = days_from_civil(y, m, d);
    if civil_from_days(days) != (y, m, d) {
        return None;
    }
    Some(days * DAY_MS - offset_secs as i64 * 1000)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-11-17 15:45:00 UTC (a Tuesday)
    const T: i64 = 1_794_930_300_000;

    #[test]
    fn civil_roundtrip() {
        for d in [-1000, 0, 1, 365, 20_000, 30_000] {
            let (y, m, dd) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn sample_timestamp_is_what_we_think() {
        assert_eq!(format_date(T, 0), "17/11/2026");
        assert_eq!(format_time(T, 0, false), "3:45 pm");
        assert_eq!(WEEKDAYS[weekday_monday0(local_day(T, 0))], "Tuesday");
    }

    #[test]
    fn twelve_and_twenty_four_hour() {
        assert_eq!(format_time(T, 0, true), "15:45");
        let midnight = T - (15 * 3600 + 45 * 60) * 1000;
        assert_eq!(format_time(midnight, 0, false), "12:00 am");
        assert_eq!(format_time(midnight + 12 * 3600 * 1000, 0, false), "12:00 pm");
        assert_eq!(format_time(midnight + 3600 * 1000 * 9 + 5 * 60_000, 0, false), "9:05 am");
    }

    #[test]
    fn perth_offset() {
        // AWST is UTC+8: 15:45 UTC is 11:45 pm the same day.
        assert_eq!(format_time(T, 8 * 3600, false), "11:45 pm");
        // 17:00 UTC is 1:00 am the next day in Perth.
        assert_eq!(format_date(T + 75 * 60_000, 8 * 3600), "18/11/2026");
    }

    #[test]
    fn labels() {
        let day = DAY_MS;
        assert_eq!(day_label(T, T, 0), "Today");
        assert_eq!(day_label(T - day, T, 0), "Yesterday");
        assert_eq!(day_label(T - 2 * day, T, 0), "Sunday");
        assert_eq!(day_label(T - 6 * day, T, 0), "Wednesday");
        assert_eq!(day_label(T - 7 * day, T, 0), "10/11/2026");
        assert_eq!(list_stamp(T - 3 * day, T, 0, false), "Sat");
        assert_eq!(list_stamp(T - 30 * day, T, 0, false), "18/10/2026");
        assert_eq!(list_stamp(T, T, 0, false), "3:45 pm");
    }

    #[test]
    fn last_active_text() {
        assert_eq!(last_active(T, T, 0, false), "Last active 3:45 pm");
        assert_eq!(last_active(T - DAY_MS, T, 0, true), "Last active yesterday at 15:45");
    }

    #[test]
    fn week_starts_monday() {
        assert_eq!(weekday_monday0(days_from_civil(2026, 11, 16)), 0);
        assert_eq!(weekday_monday0(days_from_civil(2026, 11, 22)), 6);
    }

    #[test]
    fn parse() {
        assert_eq!(parse_date("17/11/2026", 0), Some(T - (15 * 3600 + 45 * 60) * 1000));
        assert_eq!(parse_date("31/02/2026", 0), None);
        assert_eq!(parse_date("17-11-2026", 0), None);
        assert_eq!(parse_date("1/1/2026", 0).map(|t| format_date(t, 0)), Some("01/01/2026".into()));
    }
}
