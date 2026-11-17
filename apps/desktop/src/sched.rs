//! Schedule-send times (M-23): presets and a typed `dd/mm/yyyy hh:mm` in the user's local time.

/// Returns the due time in UTC ms. `off` is the local UTC offset in seconds.
pub fn due_ms(choice: &str, now_ms: i64, off: i32) -> Result<i64, String> {
    let off = off as i64;
    let local_secs = now_ms.div_euclid(1000) + off;
    let day = local_secs.div_euclid(86_400);
    let due = match choice.trim() {
        "1h" => now_ms + 3_600_000,
        "3h" => now_ms + 3 * 3_600_000,
        "tomorrow" => ((day + 1) * 86_400 + 9 * 3600 - off) * 1000,
        "monday" => {
            // 1970-01-01 was a Thursday: weekday 0 = Monday.
            let wd = (day + 3).rem_euclid(7);
            let add = match (7 - wd) % 7 {
                0 => 7,
                n => n,
            };
            ((day + add) * 86_400 + 9 * 3600 - off) * 1000
        }
        custom => {
            let t = chrono::NaiveDateTime::parse_from_str(custom, "%d/%m/%Y %H:%M").map_err(|_| "Type the time like 25/12/2026 14:30".to_string())?;
            (t.and_utc().timestamp() - off) * 1000
        }
    };
    if due <= now_ms + 5_000 {
        return Err("That time has already passed.".into());
    }
    Ok(due)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32) -> i64 {
        chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap().and_hms_opt(h, mi, 0).unwrap().and_utc().timestamp() * 1000
    }

    #[test]
    fn presets_and_custom() {
        // Tuesday 17/11/2026 12:00 UTC; the user is at UTC+8 (20:00 local).
        let now = utc(2026, 11, 17, 12, 0);
        let off = 8 * 3600;
        assert_eq!(due_ms("1h", now, off).unwrap(), now + 3_600_000);
        assert_eq!(due_ms("3h", now, off).unwrap(), now + 3 * 3_600_000);
        assert_eq!(due_ms("tomorrow", now, off).unwrap(), utc(2026, 11, 18, 1, 0), "9:00 local on the 18th");
        assert_eq!(due_ms("monday", now, off).unwrap(), utc(2026, 11, 23, 1, 0), "next Monday 9:00 local");
        // Monday itself schedules the following Monday.
        assert_eq!(due_ms("monday", utc(2026, 11, 23, 3, 0), off).unwrap(), utc(2026, 11, 30, 1, 0));
        assert_eq!(due_ms("25/12/2026 14:30", now, off).unwrap(), utc(2026, 12, 25, 6, 30));
        assert!(due_ms("17/11/2026 08:00", now, off).is_err(), "past");
        assert!(due_ms("next week", now, off).is_err());
    }
}
