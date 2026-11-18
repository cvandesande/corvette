use std::fmt;
use std::time::Duration;

const MONTH_DAYS: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
const NO_SEPARATOR: SegmentError = SegmentError::MissingSeparator;
const INVALID_STAMP: SegmentError = SegmentError::InvalidStampLength;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentName {
    camera: String,
    start: Duration,
}

impl SegmentName {
    /// Parses `camera-YYYYMMDDTHHMMSSZ.ts`; `file_name` is a bare name, not a path.
    /// # Errors
    /// Each variant names the rule that failed its check.
    pub fn parse(file_name: &str) -> Result<Self, SegmentError> {
        if file_name.contains('/') {
            return Err(SegmentError::PathSeparator);
        }
        let (camera, stamp_and_extension) = file_name.rsplit_once('-').ok_or(NO_SEPARATOR)?;
        if camera.is_empty() {
            return Err(SegmentError::EmptyCamera);
        }
        let camera_valid = camera.bytes().all(is_camera_byte);
        if !camera_valid {
            return Err(SegmentError::InvalidCameraCharacter);
        }
        let stamp = stamp_and_extension
            .strip_suffix(".ts")
            .ok_or(SegmentError::InvalidExtension)?;
        if stamp.len() != 16 {
            return Err(INVALID_STAMP);
        }
        let (date, rest) = stamp.split_at_checked(8).ok_or(INVALID_STAMP)?;
        let (t, rest) = rest.split_at_checked(1).ok_or(INVALID_STAMP)?;
        let (clock, z) = rest.split_at_checked(6).ok_or(INVALID_STAMP)?;
        if t != "T" || z != "Z" {
            return Err(SegmentError::InvalidStampCharacter);
        }
        let (year_part, rest) = date.split_at_checked(4).ok_or(INVALID_STAMP)?;
        let (month_part, day_part) = rest.split_at_checked(2).ok_or(INVALID_STAMP)?;
        let (hour_part, rest) = clock.split_at_checked(2).ok_or(INVALID_STAMP)?;
        let (minute_part, second_part) = rest.split_at_checked(2).ok_or(INVALID_STAMP)?;
        let year = decimal(year_part)?;
        let month = decimal(month_part)?;
        let day = decimal(day_part)?;
        let hour = decimal(hour_part)?;
        let minute = decimal(minute_part)?;
        let second = decimal(second_part)?;
        if !(1970..=2999).contains(&year) {
            return Err(SegmentError::YearOutOfRange);
        }
        let month_days = days_in_month(year, month).ok_or(SegmentError::MonthOutOfRange)?;
        if day < 1 || day > month_days {
            return Err(SegmentError::DayOutOfRange);
        }
        if hour > 23 {
            return Err(SegmentError::HourOutOfRange);
        }
        if minute > 59 {
            return Err(SegmentError::MinuteOutOfRange);
        }
        if second > 59 {
            return Err(SegmentError::SecondOutOfRange);
        }
        let days = days_from_civil(year, month, day);
        let seconds =
            days * 86_400 + u64::from(hour) * 3_600 + u64::from(minute) * 60 + u64::from(second);
        Ok(Self {
            camera: camera.to_string(),
            start: Duration::from_secs(seconds),
        })
    }

    #[must_use]
    pub fn camera(&self) -> &str {
        &self.camera
    }

    #[must_use]
    pub const fn start(&self) -> Duration {
        self.start
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentError {
    PathSeparator,
    MissingSeparator,
    EmptyCamera,
    InvalidCameraCharacter,
    InvalidExtension,
    InvalidStampLength,
    InvalidStampCharacter,
    MonthOutOfRange,
    DayOutOfRange,
    HourOutOfRange,
    MinuteOutOfRange,
    SecondOutOfRange,
    YearOutOfRange,
}

impl fmt::Display for SegmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::PathSeparator => "file name contains a path separator",
            Self::MissingSeparator => "no dash separates the camera from the time stamp",
            Self::EmptyCamera => "camera part is empty",
            Self::InvalidCameraCharacter => "camera contains a character outside A-Z a-z 0-9 _ -",
            Self::InvalidExtension => "extension is not exactly .ts",
            Self::InvalidStampLength => "time stamp is not 16 characters",
            Self::InvalidStampCharacter => "time stamp contains an invalid character",
            Self::MonthOutOfRange => "month is outside 01-12",
            Self::DayOutOfRange => "day is outside the month",
            Self::HourOutOfRange => "hour is above 23",
            Self::MinuteOutOfRange => "minute is above 59",
            Self::SecondOutOfRange => "second is above 59",
            Self::YearOutOfRange => "year is outside 1970-2999",
        };
        f.write_str(message)
    }
}

const fn is_camera_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-'
}

fn decimal(digits: &str) -> Result<u32, SegmentError> {
    let mut value: u32 = 0;
    for byte in digits.bytes() {
        let digit = match byte.checked_sub(b'0') {
            Some(digit) if digit <= 9 => digit,
            _ => return Err(SegmentError::InvalidStampCharacter),
        };
        value = value
            .checked_mul(10)
            .and_then(|shifted| shifted.checked_add(u32::from(digit)))
            .ok_or(SegmentError::InvalidStampCharacter)?;
    }
    Ok(value)
}

const fn is_leap_year(year: u32) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn days_in_month(year: u32, month: u32) -> Option<u32> {
    let index = usize::try_from(month.checked_sub(1)?).ok()?;
    let days = MONTH_DAYS.get(index).copied()?;
    let leap = u32::from(month == 2 && is_leap_year(year));
    Some(days + leap)
}

fn days_from_civil(year: u32, month: u32, day: u32) -> u64 {
    let (year, month, day) = (u64::from(year), u64::from(month), u64::from(day));
    let era_year = if month <= 2 { year - 1 } else { year };
    let era = era_year / 400;
    let year_of_era = era_year - era * 400;
    let era_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * era_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}
