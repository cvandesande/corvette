//! Conversions between Unix seconds and the reader's local calendar and clock.

use wasm_bindgen::JsValue;

pub(crate) fn format_event_time(unix_seconds: f64) -> String {
    let date = js_sys::Date::new(&JsValue::from_f64(unix_seconds * 1_000.0));
    format_date_time_parts(
        date.get_date(),
        date.get_month(),
        date.get_full_year(),
        date.get_hours(),
        date.get_minutes(),
    )
}

fn format_date_time_parts(day: u32, month: u32, year: u32, hour: u32, minute: u32) -> String {
    format!("{day} {} {year}, {hour:02}:{minute:02}", month_name(month))
}

#[derive(Clone)]
pub(crate) struct CalendarDay {
    pub(crate) start_time: f64,
    pub(crate) date_key: String,
    pub(crate) weekday: String,
    pub(crate) day_number: u32,
    pub(crate) month: String,
    pub(crate) accessible_label: String,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CalendarSelection {
    pub(crate) start_day: f64,
    pub(crate) end_day: f64,
    pub(crate) awaiting_end: bool,
}

impl CalendarSelection {
    pub(crate) const fn single(day: f64) -> Self {
        Self {
            start_day: day,
            end_day: day,
            awaiting_end: true,
        }
    }

    pub(crate) fn contains(self, day: f64) -> bool {
        day >= self.start_day && day <= self.end_day
    }
}

pub(crate) fn select_calendar_range(
    current: Option<CalendarSelection>,
    selected_day: f64,
) -> CalendarSelection {
    let Some(current) = current.filter(|selection| selection.awaiting_end) else {
        return CalendarSelection::single(selected_day);
    };
    CalendarSelection {
        start_day: current.start_day.min(selected_day),
        end_day: current.start_day.max(selected_day),
        awaiting_end: false,
    }
}

pub(crate) fn recent_calendar_days(count: u32) -> Vec<CalendarDay> {
    // Days are stepped back from local noon, not from the current time: a fixed
    // 24-hour step crosses two date boundaries on a 23-hour daylight-saving day
    // and none on a 25-hour one, so the strip loses the short day and repeats the
    // long one. No transition moves noon far enough to change its calendar date.
    const NOON_OFFSET_MILLIS: f64 = 43_200_000.0;

    let noon = local_day_start(&js_sys::Date::new_0()).mul_add(1_000.0, NOON_OFFSET_MILLIS);
    (0..count)
        .rev()
        .map(|offset| {
            let date = js_sys::Date::new(&JsValue::from_f64(
                f64::from(offset).mul_add(-86_400_000.0, noon),
            ));
            let start_time = local_day_start(&date);
            let start = js_sys::Date::new(&JsValue::from_f64(start_time * 1_000.0));
            CalendarDay {
                start_time,
                date_key: format!(
                    "{:04}-{:02}-{:02}",
                    start.get_full_year(),
                    start.get_month() + 1,
                    start.get_date(),
                ),
                weekday: weekday_name(start.get_day()).to_owned(),
                day_number: start.get_date(),
                month: month_name(start.get_month()).to_owned(),
                accessible_label: start
                    .to_locale_date_string("en-IE", &JsValue::UNDEFINED)
                    .into(),
            }
        })
        .collect()
}

pub(crate) fn browser_timezone() -> String {
    let formatter =
        js_sys::Intl::DateTimeFormat::new(&js_sys::Array::new(), &js_sys::Object::new());
    js_sys::Reflect::get(
        &formatter.resolved_options(),
        &JsValue::from_str("timeZone"),
    )
    .ok()
    .and_then(|timezone| timezone.as_string())
    .unwrap_or_else(|| "UTC".to_owned())
}

pub(crate) fn local_day_start(date: &js_sys::Date) -> f64 {
    local_time_of_day(date, 0, 0)
}

/// Returns the Unix seconds of `clock` on `day_start`'s calendar day, or
/// `None` when `clock` is not an `HH:MM` time.
pub(crate) fn local_day_time(day_start: f64, clock: &str) -> Option<f64> {
    let (hours, minutes) = parse_clock(clock)?;
    let date = js_sys::Date::new(&JsValue::from_f64(day_start * 1_000.0));
    Some(local_time_of_day(&date, hours, minutes))
}

/// Parses an `<input type="time">` value into local hours and minutes.
///
/// Returns `None` for anything that is not `HH:MM`, which is what the element
/// reports once the user clears it.
fn parse_clock(clock: &str) -> Option<(u32, u32)> {
    let (hours, minutes) = clock.split_once(':')?;
    let hours = hours.parse::<u32>().ok().filter(|hours| *hours < 24)?;
    let minutes = minutes
        .parse::<u32>()
        .ok()
        .filter(|minutes| *minutes < 60)?;
    Some((hours, minutes))
}

fn local_time_of_day(date: &js_sys::Date, hours: u32, minutes: u32) -> f64 {
    let local = format!(
        "{:04}-{:02}-{:02}T{hours:02}:{minutes:02}:00",
        date.get_full_year(),
        date.get_month() + 1,
        date.get_date(),
    );
    // A local date built from a Date's own fields always parses; a
    // spring-forward hour that does not exist is normalized rather than
    // rejected.
    js_sys::Date::parse(&local) / 1_000.0
}

pub(crate) fn next_local_day(day_start: f64) -> f64 {
    // 36 hours: past the longest local day a daylight-saving transition can
    // produce (25 hours) and short of two days, so the result always lands on
    // the following calendar date whatever the offset does in between.
    const NEXT_DAY_OFFSET_MILLIS: f64 = 129_600_000.0;

    let date = js_sys::Date::new(&JsValue::from_f64(
        day_start.mul_add(1_000.0, NEXT_DAY_OFFSET_MILLIS),
    ));
    local_day_start(&date)
}

const fn weekday_name(day: u32) -> &'static str {
    match day {
        0 => "Sun",
        1 => "Mon",
        2 => "Tue",
        3 => "Wed",
        4 => "Thu",
        5 => "Fri",
        6 => "Sat",
        // ECMA-262 defines getDay as 0-6, so any other value means the
        // argument did not come from a Date.
        _ => panic!("weekday number out of range 0-6"),
    }
}

const fn month_name(month: u32) -> &'static str {
    match month {
        0 => "Jan",
        1 => "Feb",
        2 => "Mar",
        3 => "Apr",
        4 => "May",
        5 => "Jun",
        6 => "Jul",
        7 => "Aug",
        8 => "Sep",
        9 => "Oct",
        10 => "Nov",
        11 => "Dec",
        // ECMA-262 defines getMonth as 0-11, so any other value means the
        // argument did not come from a Date.
        _ => panic!("month number out of range 0-11"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cleared_or_malformed_time_input_is_not_a_clock() {
        assert_eq!(parse_clock("00:00"), Some((0, 0)));
        assert_eq!(parse_clock("07:05"), Some((7, 5)));
        assert_eq!(parse_clock("23:59"), Some((23, 59)));
        assert_eq!(parse_clock(""), None);
        assert_eq!(parse_clock("24:00"), None);
        assert_eq!(parse_clock("12:60"), None);
        assert_eq!(parse_clock("12"), None);
        assert_eq!(parse_clock("noon"), None);
    }

    #[test]
    #[should_panic(expected = "weekday number out of range 0-6")]
    fn a_weekday_outside_the_date_contract_is_a_bug() {
        let day = std::hint::black_box(7);
        let _ = weekday_name(day);
    }

    #[test]
    #[should_panic(expected = "month number out of range 0-11")]
    fn a_month_outside_the_date_contract_is_a_bug() {
        let month = std::hint::black_box(12);
        let _ = month_name(month);
    }

    #[test]
    fn event_time_uses_an_unambiguous_month_name_and_padded_clock() {
        assert_eq!(
            format_date_time_parts(3, 7, 2026, 6, 4),
            "3 Aug 2026, 06:04"
        );
    }

    #[test]
    fn calendar_taps_choose_a_range_then_start_a_new_one() {
        let start = select_calendar_range(None, 200.0);
        assert_eq!(start, CalendarSelection::single(200.0));

        let completed = select_calendar_range(Some(start), 100.0);
        assert_eq!(
            completed,
            CalendarSelection {
                start_day: 100.0,
                end_day: 200.0,
                awaiting_end: false,
            }
        );
        assert!(completed.contains(150.0));

        assert_eq!(
            select_calendar_range(Some(completed), 300.0),
            CalendarSelection::single(300.0)
        );
    }
}
