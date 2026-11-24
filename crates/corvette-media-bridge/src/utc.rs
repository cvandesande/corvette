//! Calendar parts of a Unix-millisecond timestamp in UTC, from the days-from-civil algorithm.

/// The UTC year, month, day and hour one Unix-millisecond timestamp lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UtcParts {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
}

/// Returns the UTC calendar date and hour of `unix_ms`.
#[must_use]
pub fn utc_parts(unix_ms: i64) -> UtcParts {
    let (year, month, day, hour) = calendar_from_millis(unix_ms);
    UtcParts {
        year,
        month,
        day,
        hour,
    }
}

fn calendar_from_millis(unix_ms: i64) -> (i32, u32, u32, u32) {
    let ms_per_hour = 3_600_000;
    let ms_per_day = 24 * ms_per_hour;
    let days = unix_ms.div_euclid(ms_per_day);
    let hour = unix_ms.rem_euclid(ms_per_day) / ms_per_hour;
    let (year, month, day) = civil_from_days(days);
    (
        i32::try_from(year).expect("a year an i64 millisecond epoch can name fits i32"),
        u32::try_from(month).expect("month is 1 to 12"),
        u32::try_from(day).expect("day is 1 to 31"),
        u32::try_from(hour).expect("hour is 0 to 23"),
    )
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days_to_civil_epoch = 719_468; // days from 0000-03-01 to the Unix epoch
    let days_per_400_year_gregorian_cycle = 146_097;
    let z = days + days_to_civil_epoch;
    let era = z.div_euclid(days_per_400_year_gregorian_cycle);
    let day_of_era = z.rem_euclid(days_per_400_year_gregorian_cycle);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    (year + i64::from(month <= 2), month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_millisecond_is_1970_01_01_hour_00() {
        assert_eq!(
            utc_parts(0),
            UtcParts {
                year: 1970,
                month: 1,
                day: 1,
                hour: 0
            }
        );
    }

    #[test]
    fn reads_2026_11_24_09_00_utc() {
        let known_ms = 1_795_510_800_000;
        assert_eq!(
            utc_parts(known_ms),
            UtcParts {
                year: 2026,
                month: 11,
                day: 24,
                hour: 9
            }
        );
    }

    #[test]
    fn reads_the_2024_leap_day() {
        let leap_day_ms = 1_709_164_800_000;
        assert_eq!(
            utc_parts(leap_day_ms),
            UtcParts {
                year: 2024,
                month: 2,
                day: 29,
                hour: 0
            }
        );
    }

    #[test]
    fn reads_the_day_after_1900_rule_february_in_2100() {
        let far_ms = 4_107_542_400_000;
        assert_eq!(
            utc_parts(far_ms),
            UtcParts {
                year: 2100,
                month: 3,
                day: 1,
                hour: 0
            }
        );
    }

    #[test]
    fn last_millisecond_of_an_hour_keeps_the_hour() {
        let last_ms = 1_795_514_399_999;
        assert_eq!(
            utc_parts(last_ms),
            UtcParts {
                year: 2026,
                month: 11,
                day: 24,
                hour: 9
            }
        );
    }

    #[test]
    fn last_millisecond_before_the_epoch_is_the_day_and_hour_before_it() {
        assert_eq!(
            utc_parts(-1),
            UtcParts {
                year: 1969,
                month: 12,
                day: 31,
                hour: 23
            }
        );
    }
}
