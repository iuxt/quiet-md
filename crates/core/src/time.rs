//! Wall-clock time and civil dates in the system time zone.

use chrono::{DateTime, Datelike, Local, TimeZone, Timelike};

/// Wall-clock unix seconds.
pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Local civil time at this timestamp, including its daylight-saving rule.
pub fn civil(secs: u64) -> (i64, u32, u32, u32, u32) {
    let utc = DateTime::from_timestamp(secs.min(i64::MAX as u64) as i64, 0).unwrap_or(DateTime::UNIX_EPOCH);
    let t = utc.with_timezone(&Local);
    (t.year() as i64, t.month(), t.day(), t.hour(), t.minute())
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Hinnant's
/// days-from-civil).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Unix seconds of a local date and time. In a repeated hour choose the
/// first occurrence; a skipped time advances to the next valid minute.
pub fn local_to_unix(y: i64, m: u32, d: u32, h: u32, mi: u32) -> i64 {
    let raw = days_from_civil(y, m, d) * 86_400 + (h * 3600 + mi * 60) as i64;
    let Some(t) = DateTime::from_timestamp(raw, 0) else { return raw };
    let mut local = t.naive_utc();
    for _ in 0..=1440 {
        match Local.from_local_datetime(&local) {
            chrono::LocalResult::Single(t) => return t.timestamp(),
            chrono::LocalResult::Ambiguous(a, b) => return a.timestamp().min(b.timestamp()),
            chrono::LocalResult::None => {}
        }
        let Some(next) = local.checked_add_signed(chrono::Duration::minutes(1)) else { break };
        local = next;
    }
    raw
}

pub fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// 0 = Monday … 6 = Sunday.
pub fn weekday(y: i64, m: u32, d: u32) -> u32 {
    (days_from_civil(y, m, d) + 3).rem_euclid(7) as u32
}

/// Wall-clock unix milliseconds.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

pub fn local_offset() -> i64 {
    Local::now().offset().local_minus_utc() as i64
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn system_timezone() {
        match std::env::var("FDL_TIME_TEST").as_deref() {
            Ok("America/New_York") => {
                assert_eq!(local_to_unix(2027, 1, 1, 12, 0), 1_798_822_800);
                assert_eq!(local_to_unix(2027, 7, 1, 12, 0), 1_814_457_600);
                assert_eq!(civil(1_814_457_600), (2027, 7, 1, 12, 0));
                assert_eq!(local_to_unix(2026, 3, 8, 2, 30), 1_772_953_200);
                assert_eq!(local_to_unix(2026, 11, 1, 1, 30), 1_793_511_000);
            }
            Ok("Asia/Kathmandu") => {
                assert_eq!(local_to_unix(1970, 1, 2, 12, 0), 109_800);
                assert_eq!(civil(local_to_unix(2027, 7, 1, 12, 0) as u64), (2027, 7, 1, 12, 0));
            }
            Ok("UTC") => assert_eq!(local_to_unix(1970, 1, 2, 12, 0), 129_600),
            _ => {
                for zone in ["America/New_York", "Asia/Kathmandu", "UTC"] {
                    let status = std::process::Command::new(std::env::current_exe().unwrap())
                        .args(["--exact", "time::tests::system_timezone", "--nocapture"])
                        .env("TZ", zone).env("FDL_TIME_TEST", zone).env("PATH", "").status().unwrap();
                    assert!(status.success(), "{zone}");
                }
            }
        }
    }
}
