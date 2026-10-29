//! Local time without tz data in the container: a fixed standard offset plus,
//! optionally, the EU daylight-saving rule (last Sunday of March 01:00 UTC →
//! last Sunday of October 01:00 UTC). Configured in config.json → `timezone`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Tz {
    /// Standard (winter) offset from UTC, minutes. Ukraine: 120.
    #[serde(default = "d_off")]
    pub offset_min: i64,
    /// Apply the EU summer-time rule (+1 h in summer).
    #[serde(default = "yes")]
    pub eu_dst: bool,
}
fn d_off() -> i64 { 120 }
fn yes() -> bool { true }

impl Default for Tz {
    fn default() -> Self { Tz { offset_min: d_off(), eu_dst: true } }
}

/// Days since 1970-01-01 for a civil date (H. Hinnant's algorithm).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// (year, month, day) for days since 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

/// UTC timestamp of the last Sunday of `month` in `year`, 01:00 UTC.
fn last_sunday_0100(year: i64, month: i64) -> i64 {
    let next = if month == 12 { days_from_civil(year + 1, 1, 1) } else { days_from_civil(year, month + 1, 1) };
    let last = next - 1;
    let dow = (last + 4).rem_euclid(7); // 0 = Sunday (1970-01-01 was a Thursday)
    (last - dow) * 86_400 + 3_600
}

impl Tz {
    pub fn offset_s(&self, ts: i64) -> i64 {
        let base = self.offset_min * 60;
        if !self.eu_dst {
            return base;
        }
        let (y, _, _) = civil_from_days(ts.div_euclid(86_400));
        if ts >= last_sunday_0100(y, 3) && ts < last_sunday_0100(y, 10) { base + 3_600 } else { base }
    }
    /// Seconds since 1970 in local wall-clock terms.
    pub fn local(&self, ts: i64) -> i64 {
        ts + self.offset_s(ts)
    }
    /// Local calendar day "YYYY-MM-DD".
    pub fn day(&self, ts: i64) -> String {
        let (y, m, d) = civil_from_days(self.local(ts).div_euclid(86_400));
        format!("{y:04}-{m:02}-{d:02}")
    }
    /// Minutes since local midnight (0..1439).
    pub fn minute_of_day(&self, ts: i64) -> i64 {
        self.local(ts).rem_euclid(86_400) / 60
    }
    /// ISO weekday, 1 = Monday … 7 = Sunday.
    pub fn weekday(&self, ts: i64) -> i64 {
        (self.local(ts).div_euclid(86_400) + 3).rem_euclid(7) + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_roundtrip() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(days_from_civil(2026, 10, 29)), (2026, 10, 29));
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
    }

    #[test]
    fn kyiv_offsets() {
        let tz = Tz::default();
        let summer = days_from_civil(2026, 7, 1) * 86_400;
        let winter = days_from_civil(2026, 12, 1) * 86_400;
        assert_eq!(tz.offset_s(summer), 3 * 3600);
        assert_eq!(tz.offset_s(winter), 2 * 3600);
        // 2026: DST ends Sunday 25 Oct 01:00 UTC
        let end = days_from_civil(2026, 10, 25) * 86_400 + 3_600;
        assert_eq!(tz.offset_s(end - 1), 3 * 3600);
        assert_eq!(tz.offset_s(end), 2 * 3600);
        // starts Sunday 29 Mar 2026 01:00 UTC
        let start = days_from_civil(2026, 3, 29) * 86_400 + 3_600;
        assert_eq!(tz.offset_s(start - 1), 2 * 3600);
        assert_eq!(tz.offset_s(start), 3 * 3600);
    }

    #[test]
    fn local_day_and_clock() {
        let tz = Tz::default();
        // 2026-10-29 22:30 UTC = 2026-10-30 00:30 in Kyiv (UTC+2)
        let t = days_from_civil(2026, 10, 29) * 86_400 + 22 * 3600 + 1800;
        assert_eq!(tz.day(t), "2026-10-30");
        assert_eq!(tz.minute_of_day(t), 30);
        assert_eq!(tz.weekday(t), 5, "30 Oct 2026 is a Friday");
    }
}
