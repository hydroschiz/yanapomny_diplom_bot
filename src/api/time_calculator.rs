//! Time calculation from LLM TimeSpec to actual DateTime.

use anyhow::{Context, Result};
use chrono::{
    DateTime, Datelike, Duration, FixedOffset, NaiveTime, Offset, Utc, Weekday as ChronoWeekday,
};
use chrono_tz::Tz;

use super::llm_models::{
    DayPosition, OffsetDirection, ParsedReminder, RecurrenceFilter, RecurrencePattern,
    ReminderType, TimeOfDay, TimeSpec, TimeSpecType,
};

// Re-export Weekday for tests
#[cfg(test)]
use super::llm_models::Weekday;

/// User's time preferences for morning/afternoon/evening.
///
/// Supports both IANA timezone names (e.g., "Europe/Moscow") and
/// fixed UTC offsets (e.g., "+07:00", "UTC+7").
#[derive(Debug, Clone)]
pub struct UserTimePrefs {
    pub morning: NaiveTime,
    pub afternoon: NaiveTime,
    pub evening: NaiveTime,
    /// Offset from UTC in seconds (e.g., +7 hours = 25200).
    pub offset_seconds: i32,
}

impl Default for UserTimePrefs {
    fn default() -> Self {
        Self {
            morning: NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            afternoon: NaiveTime::from_hms_opt(14, 0, 0).unwrap(),
            evening: NaiveTime::from_hms_opt(19, 0, 0).unwrap(),
            offset_seconds: 0, // UTC
        }
    }
}

impl UserTimePrefs {
    /// Parse from user database record.
    ///
    /// Priority:
    /// 1. If `timezone` is set (IANA name like "Europe/Moscow"), use it
    /// 2. Otherwise, parse `utc_offset` (formats: "+07:00", "UTC+7", "+7", etc.)
    pub fn from_db(
        morning: &str,
        afternoon: &str,
        evening: &str,
        timezone: &str,
        utc_offset: &str,
    ) -> Self {
        let parse_time = |s: &str| -> NaiveTime {
            let parts: Vec<&str> = s.split(':').collect();
            if parts.len() >= 2 {
                let hour = parts[0].parse().unwrap_or(8);
                let min = parts[1].parse().unwrap_or(0);
                NaiveTime::from_hms_opt(hour, min, 0)
                    .unwrap_or_else(|| NaiveTime::from_hms_opt(8, 0, 0).unwrap())
            } else {
                NaiveTime::from_hms_opt(8, 0, 0).unwrap()
            }
        };

        // Calculate offset in seconds
        let offset_seconds = if !timezone.is_empty() {
            // Try IANA timezone name first
            if let Ok(tz) = timezone.parse::<Tz>() {
                let now = Utc::now().with_timezone(&tz);
                now.offset().fix().local_minus_utc()
            } else {
                0
            }
        } else if !utc_offset.is_empty() && utc_offset != "nil" {
            // Parse UTC offset string (e.g., "+07:00", "UTC+7", "+7")
            parse_utc_offset_to_seconds(utc_offset).unwrap_or(0)
        } else {
            0
        };

        Self {
            morning: parse_time(morning),
            afternoon: parse_time(afternoon),
            evening: parse_time(evening),
            offset_seconds,
        }
    }

    /// Get time for time_of_day.
    pub fn time_for_day_part(&self, tod: TimeOfDay) -> NaiveTime {
        match tod {
            TimeOfDay::Morning => self.morning,
            TimeOfDay::Afternoon => self.afternoon,
            TimeOfDay::Evening => self.evening,
        }
    }

    /// Get the fixed offset for this user's timezone.
    pub fn fixed_offset(&self) -> FixedOffset {
        FixedOffset::east_opt(self.offset_seconds)
            .unwrap_or_else(|| FixedOffset::east_opt(0).unwrap())
    }

    /// Get offset in hours (for display purposes).
    pub fn offset_hours(&self) -> i32 {
        self.offset_seconds / 3600
    }
}

/// Parse UTC offset string to seconds.
/// Supports formats: "+07:00", "-05:30", "UTC+7", "+7", "7", etc.
fn parse_utc_offset_to_seconds(s: &str) -> Option<i32> {
    let s = s.trim().to_uppercase();

    // Remove "UTC" or "GMT" prefix
    let s = s
        .strip_prefix("UTC")
        .or_else(|| s.strip_prefix("GMT"))
        .unwrap_or(&s)
        .trim();

    if s.is_empty() {
        return Some(0);
    }

    // Determine sign
    let (sign, rest) = if let Some(rest) = s.strip_prefix('+') {
        (1, rest)
    } else if let Some(rest) = s.strip_prefix('-') {
        (-1, rest)
    } else {
        (1, s)
    };

    // Parse hours and minutes
    let parts: Vec<&str> = rest.split([':', '.']).collect();
    let hours: i32 = parts.first()?.trim().parse().ok()?;
    let minutes: i32 = parts
        .get(1)
        .and_then(|m| m.trim().parse().ok())
        .unwrap_or(0);

    // Validate
    if hours.abs() > 14 || minutes >= 60 {
        return None;
    }

    Some(sign * (hours * 3600 + minutes * 60))
}

/// Calculate the actual DateTime from a ParsedReminder.
pub fn calculate_reminder_time(
    reminder: &ParsedReminder,
    now: DateTime<Utc>,
    prefs: &UserTimePrefs,
) -> Result<DateTime<Utc>> {
    let time_spec = reminder
        .time_spec
        .as_ref()
        .context("Reminder has no time specification")?;

    calculate_from_time_spec(time_spec, now, prefs)
}

/// Calculate DateTime from TimeSpec.
pub fn calculate_from_time_spec(
    spec: &TimeSpec,
    now: DateTime<Utc>,
    prefs: &UserTimePrefs,
) -> Result<DateTime<Utc>> {
    // Convert now to user's timezone for calculations
    let user_offset = prefs.fixed_offset();
    let now_local = now.with_timezone(&user_offset);

    let result = match spec.spec_type {
        TimeSpecType::Relative => calculate_relative(spec, now_local, prefs)?,
        TimeSpecType::Weekday => calculate_weekday(spec, now_local, prefs)?,
        TimeSpecType::Absolute => calculate_absolute(spec, now_local, prefs)?,
        TimeSpecType::Monthly => calculate_monthly(spec, now_local, prefs)?,
        TimeSpecType::Yearly => calculate_yearly(spec, now_local, prefs)?,
        TimeSpecType::Daily => calculate_daily(spec, now_local, prefs)?,
    };

    // Convert back to UTC
    Ok(result.with_timezone(&Utc))
}

/// Calculate relative time (e.g., "через 20 минут").
fn calculate_relative(
    spec: &TimeSpec,
    now: DateTime<FixedOffset>,
    prefs: &UserTimePrefs,
) -> Result<DateTime<FixedOffset>> {
    let mut result = now;
    let user_offset = prefs.fixed_offset();

    // Determine base from anchor
    if let Some(anchor) = &spec.anchor {
        match anchor.as_str() {
            "now" => { /* already set to now */ }
            "today" => {
                // Set to start of today
                result = now
                    .date_naive()
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_local_timezone(user_offset)
                    .single()
                    .unwrap_or(now);
            }
            "current_week" => {
                // Set to start of current week (Monday)
                let days_since_monday = now.weekday().num_days_from_monday();
                result = (now - Duration::days(days_since_monday as i64))
                    .date_naive()
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_local_timezone(user_offset)
                    .single()
                    .unwrap_or(now);
            }
            "next_week" => {
                // Set to start of next week (Monday)
                let days_until_monday = 7 - now.weekday().num_days_from_monday();
                result = (now + Duration::days(days_until_monday as i64))
                    .date_naive()
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_local_timezone(user_offset)
                    .single()
                    .unwrap_or(now);
            }
            _ => {
                // Try to parse as date YYYY-MM-DD or YYYY-MM-DD HH:MM
                if let Ok(dt) = chrono::NaiveDate::parse_from_str(anchor, "%Y-%m-%d") {
                    result = dt
                        .and_hms_opt(0, 0, 0)
                        .unwrap()
                        .and_local_timezone(user_offset)
                        .single()
                        .unwrap_or(now);
                } else if let Ok(dt) =
                    chrono::NaiveDateTime::parse_from_str(anchor, "%Y-%m-%d %H:%M")
                {
                    result = dt.and_local_timezone(user_offset).single().unwrap_or(now);
                }
            }
        }
    }

    // Apply offsets
    let direction = spec.offset_direction.unwrap_or(OffsetDirection::After);
    let multiplier: i64 = match direction {
        OffsetDirection::After => 1,
        OffsetDirection::Before => -1,
    };

    if spec.offset_minutes != 0 {
        result += Duration::minutes(spec.offset_minutes as i64 * multiplier);
    }
    if spec.offset_hours != 0 {
        result += Duration::hours(spec.offset_hours as i64 * multiplier);
    }
    if spec.offset_days != 0 {
        result += Duration::days(spec.offset_days as i64 * multiplier);
    }
    if spec.offset_weeks != 0 {
        result += Duration::weeks(spec.offset_weeks as i64 * multiplier);
    }
    if spec.offset_months != 0 {
        result = add_months(result, spec.offset_months * multiplier as i32, prefs);
    }
    if spec.offset_years != 0 {
        result = add_months(result, spec.offset_years * 12 * multiplier as i32, prefs);
    }

    // Apply time if specified
    result = apply_time(result, spec, prefs);

    Ok(result)
}

/// Calculate weekday-based time (e.g., "в понедельник в 18").
fn calculate_weekday(
    spec: &TimeSpec,
    now: DateTime<FixedOffset>,
    prefs: &UserTimePrefs,
) -> Result<DateTime<FixedOffset>> {
    let target_weekday = spec
        .weekday
        .as_ref()
        .context("Weekday type requires weekday field")?
        .to_chrono();

    let mut result = find_next_weekday(now, target_weekday);

    // Handle week_of_month for patterns like "вторую пятницу месяца"
    if spec.week_of_month != 0 {
        result = find_nth_weekday_of_month(now, target_weekday, spec.week_of_month, prefs)?;
    }

    // Apply time
    result = apply_time(result, spec, prefs);

    // If this time already passed today, move to next week
    if result <= now {
        result += Duration::weeks(1);
    }

    Ok(result)
}

/// Calculate absolute date (e.g., "16 сентября в 10:20").
fn calculate_absolute(
    spec: &TimeSpec,
    now: DateTime<FixedOffset>,
    prefs: &UserTimePrefs,
) -> Result<DateTime<FixedOffset>> {
    let mut result = now;

    if let Some(date_str) = &spec.date {
        // Try MM-DD format
        if let Some(parsed) = parse_date_mmdd(date_str, now, prefs) {
            result = parsed;
        }
        // Try DD.MM.YYYY format
        else if let Some(parsed) = parse_date_ddmmyyyy(date_str, prefs) {
            result = parsed;
        }
    } else if spec.day_of_month != 0 {
        // Use day_of_month
        result = set_day_of_month(now, spec.day_of_month as u32, prefs)?;
    }

    // Apply offsets if any (for "за неделю до 1 сентября")
    let direction = spec.offset_direction.unwrap_or(OffsetDirection::After);
    let multiplier: i64 = match direction {
        OffsetDirection::After => 1,
        OffsetDirection::Before => -1,
    };

    if spec.offset_weeks != 0 {
        result += Duration::weeks(spec.offset_weeks as i64 * multiplier);
    }
    if spec.offset_days != 0 {
        result += Duration::days(spec.offset_days as i64 * multiplier);
    }

    // Apply time
    result = apply_time(result, spec, prefs);

    Ok(result)
}

/// Calculate monthly pattern (e.g., "каждое 28 число").
fn calculate_monthly(
    spec: &TimeSpec,
    now: DateTime<FixedOffset>,
    prefs: &UserTimePrefs,
) -> Result<DateTime<FixedOffset>> {
    let mut result = now;

    if let Some(DayPosition::Last) = &spec.day_position {
        // Last day of current month
        result = last_day_of_month(now, prefs)?;
    } else if spec.day_of_month != 0 {
        result = set_day_of_month(now, spec.day_of_month as u32, prefs)?;
        // If this day already passed this month, move to next month
        if result <= now {
            result = add_months(result, 1, prefs);
            result = set_day_of_month(result, spec.day_of_month as u32, prefs)?;
        }
    } else if spec.week_of_month != 0 {
        // N-th weekday of month
        let weekday = spec
            .weekday
            .as_ref()
            .map(|w| w.to_chrono())
            .unwrap_or(ChronoWeekday::Mon);
        result = find_nth_weekday_of_month(now, weekday, spec.week_of_month, prefs)?;
    }

    // Apply time
    result = apply_time(result, spec, prefs);

    Ok(result)
}

/// Calculate yearly pattern (e.g., "каждое 30 мая").
fn calculate_yearly(
    spec: &TimeSpec,
    now: DateTime<FixedOffset>,
    prefs: &UserTimePrefs,
) -> Result<DateTime<FixedOffset>> {
    let mut result = now;

    if let Some(date_str) = &spec.date {
        // Parse MM-DD
        if let Some((month, day)) = parse_mmdd(date_str) {
            result = set_month_day(now, month, day, prefs)?;
            // If this date already passed this year, move to next year
            if result <= now {
                result = add_months(result, 12, prefs);
            }
        }
    }

    // Apply time
    result = apply_time(result, spec, prefs);

    Ok(result)
}

/// Calculate daily pattern.
fn calculate_daily(
    spec: &TimeSpec,
    now: DateTime<FixedOffset>,
    prefs: &UserTimePrefs,
) -> Result<DateTime<FixedOffset>> {
    let mut result = now;

    // Apply time for today
    result = apply_time(result, spec, prefs);

    // If time already passed today, move to tomorrow
    if result <= now {
        result += Duration::days(1);
    }

    Ok(result)
}

// ============================================================================
// Helper functions
// ============================================================================

/// Apply time specification to a date.
fn apply_time(
    dt: DateTime<FixedOffset>,
    spec: &TimeSpec,
    prefs: &UserTimePrefs,
) -> DateTime<FixedOffset> {
    let user_offset = prefs.fixed_offset();

    if let Some(time_str) = &spec.time {
        if let Some(time) = parse_time_hhmm(time_str) {
            return dt
                .date_naive()
                .and_time(time)
                .and_local_timezone(user_offset)
                .single()
                .unwrap_or(dt);
        }
    }

    if let Some(tod) = &spec.time_of_day {
        let time = prefs.time_for_day_part(*tod);
        return dt
            .date_naive()
            .and_time(time)
            .and_local_timezone(user_offset)
            .single()
            .unwrap_or(dt);
    }

    dt
}

/// Parse HH:MM time string.
fn parse_time_hhmm(s: &str) -> Option<NaiveTime> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() >= 2 {
        let hour: u32 = parts[0].parse().ok()?;
        let min: u32 = parts[1].parse().ok()?;
        NaiveTime::from_hms_opt(hour, min, 0)
    } else {
        None
    }
}

/// Parse MM-DD date string.
fn parse_mmdd(s: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() == 2 {
        let month: u32 = parts[0].parse().ok()?;
        let day: u32 = parts[1].parse().ok()?;
        Some((month, day))
    } else {
        None
    }
}

/// Parse MM-DD and return DateTime in current/next year.
fn parse_date_mmdd(
    s: &str,
    now: DateTime<FixedOffset>,
    prefs: &UserTimePrefs,
) -> Option<DateTime<FixedOffset>> {
    let (month, day) = parse_mmdd(s)?;
    set_month_day(now, month, day, prefs).ok()
}

/// Parse DD.MM.YYYY date string.
fn parse_date_ddmmyyyy(s: &str, prefs: &UserTimePrefs) -> Option<DateTime<FixedOffset>> {
    let user_offset = prefs.fixed_offset();
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() == 3 {
        let day: u32 = parts[0].parse().ok()?;
        let month: u32 = parts[1].parse().ok()?;
        let year: i32 = parts[2].parse().ok()?;
        let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
        date.and_hms_opt(0, 0, 0)?
            .and_local_timezone(user_offset)
            .single()
    } else {
        None
    }
}

/// Set day of month, handling overflow.
fn set_day_of_month(
    dt: DateTime<FixedOffset>,
    day: u32,
    prefs: &UserTimePrefs,
) -> Result<DateTime<FixedOffset>> {
    let user_offset = prefs.fixed_offset();
    let year = dt.year();
    let month = dt.month();
    let max_day = days_in_month(year, month);
    let actual_day = day.min(max_day);

    let date = chrono::NaiveDate::from_ymd_opt(year, month, actual_day).context("Invalid date")?;

    Ok(date
        .and_time(dt.time())
        .and_local_timezone(user_offset)
        .single()
        .unwrap_or(dt))
}

/// Set month and day, handling overflow.
fn set_month_day(
    dt: DateTime<FixedOffset>,
    month: u32,
    day: u32,
    prefs: &UserTimePrefs,
) -> Result<DateTime<FixedOffset>> {
    let user_offset = prefs.fixed_offset();
    let year = dt.year();
    let max_day = days_in_month(year, month);
    let actual_day = day.min(max_day);

    let date = chrono::NaiveDate::from_ymd_opt(year, month, actual_day).context("Invalid date")?;

    Ok(date
        .and_time(dt.time())
        .and_local_timezone(user_offset)
        .single()
        .unwrap_or(dt))
}

/// Get last day of month.
fn last_day_of_month(
    dt: DateTime<FixedOffset>,
    prefs: &UserTimePrefs,
) -> Result<DateTime<FixedOffset>> {
    let year = dt.year();
    let month = dt.month();
    let last_day = days_in_month(year, month);
    set_day_of_month(dt, last_day, prefs)
}

/// Get number of days in a month.
fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

/// Check if year is a leap year.
fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

/// Add months to a DateTime.
fn add_months(
    dt: DateTime<FixedOffset>,
    months: i32,
    prefs: &UserTimePrefs,
) -> DateTime<FixedOffset> {
    let user_offset = prefs.fixed_offset();
    let mut year = dt.year();
    let mut month = dt.month() as i32 + months;

    while month > 12 {
        month -= 12;
        year += 1;
    }
    while month < 1 {
        month += 12;
        year -= 1;
    }

    let max_day = days_in_month(year, month as u32);
    let day = dt.day().min(max_day);

    chrono::NaiveDate::from_ymd_opt(year, month as u32, day)
        .and_then(|d| {
            d.and_time(dt.time())
                .and_local_timezone(user_offset)
                .single()
        })
        .unwrap_or(dt)
}

/// Find next occurrence of a weekday.
fn find_next_weekday(from: DateTime<FixedOffset>, target: ChronoWeekday) -> DateTime<FixedOffset> {
    let current = from.weekday();
    let current_num = current.num_days_from_monday();
    let target_num = target.num_days_from_monday();

    let days_ahead = if target_num > current_num {
        target_num - current_num
    } else if target_num < current_num {
        7 - (current_num - target_num)
    } else {
        // Same day - if time hasn't passed, use today; otherwise next week
        0
    };

    from + Duration::days(days_ahead as i64)
}

/// Find N-th weekday of a month.
fn find_nth_weekday_of_month(
    now: DateTime<FixedOffset>,
    weekday: ChronoWeekday,
    week_of_month: i32,
    prefs: &UserTimePrefs,
) -> Result<DateTime<FixedOffset>> {
    let user_offset = prefs.fixed_offset();
    let year = now.year();
    let month = now.month();

    // Start of month
    let first_of_month = chrono::NaiveDate::from_ymd_opt(year, month, 1).context("Invalid date")?;

    if week_of_month == -1 {
        // Last weekday of month
        let last_day = days_in_month(year, month);
        let last_of_month =
            chrono::NaiveDate::from_ymd_opt(year, month, last_day).context("Invalid date")?;

        let mut day = last_of_month;
        while day.weekday() != weekday {
            day = day.pred_opt().context("Date underflow")?;
        }

        Ok(day
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(user_offset)
            .single()
            .context("Timezone conversion failed")?)
    } else {
        // N-th weekday (1-based)
        let n = week_of_month.max(1) as u32;

        // Find first occurrence of weekday in month
        let mut day = first_of_month;
        while day.weekday() != weekday {
            day = day.succ_opt().context("Date overflow")?;
        }

        // Add (n-1) weeks
        day += Duration::weeks((n - 1) as i64);

        // Check if still in same month
        if day.month() != month {
            anyhow::bail!("Week {} of month doesn't exist", n);
        }

        Ok(day
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(user_offset)
            .single()
            .context("Timezone conversion failed")?)
    }
}

/// Calculate next occurrence for a recurring reminder.
pub fn calculate_next_occurrence(
    current: DateTime<Utc>,
    reminder: &ParsedReminder,
    prefs: &UserTimePrefs,
) -> Result<Option<DateTime<Utc>>> {
    if reminder.reminder_type != ReminderType::Recurring {
        return Ok(None);
    }

    let recurrence = reminder
        .recurrence
        .as_ref()
        .context("Recurring reminder has no recurrence info")?;

    let interval = recurrence.interval.max(1);
    let user_offset = prefs.fixed_offset();
    let current_local = current.with_timezone(&user_offset);

    let next = match recurrence.pattern {
        RecurrencePattern::Daily => {
            let mut next = current_local + Duration::days(interval as i64);

            // Apply filters
            if recurrence.filters.contains(&RecurrenceFilter::Weekdays) {
                while !is_weekday(next.weekday()) {
                    next += Duration::days(1);
                }
            } else if recurrence.filters.contains(&RecurrenceFilter::Weekends) {
                while !is_weekend(next.weekday()) {
                    next += Duration::days(1);
                }
            }

            next
        }
        RecurrencePattern::Weekly => current_local + Duration::weeks(interval as i64),
        RecurrencePattern::Monthly => add_months(current_local, interval, prefs),
        RecurrencePattern::Yearly => add_months(current_local, interval * 12, prefs),
        RecurrencePattern::Custom => match recurrence.interval_unit.as_ref() {
            Some(super::llm_models::IntervalUnit::Days) => {
                current_local + Duration::days(interval as i64)
            }
            Some(super::llm_models::IntervalUnit::Weeks) => {
                current_local + Duration::weeks(interval as i64)
            }
            Some(super::llm_models::IntervalUnit::Months) => {
                add_months(current_local, interval, prefs)
            }
            Some(super::llm_models::IntervalUnit::Years) => {
                add_months(current_local, interval * 12, prefs)
            }
            None => current_local + Duration::days(interval as i64),
        },
    };

    Ok(Some(next.with_timezone(&Utc)))
}

fn is_weekday(wd: ChronoWeekday) -> bool {
    matches!(
        wd,
        ChronoWeekday::Mon
            | ChronoWeekday::Tue
            | ChronoWeekday::Wed
            | ChronoWeekday::Thu
            | ChronoWeekday::Fri
    )
}

fn is_weekend(wd: ChronoWeekday) -> bool {
    matches!(wd, ChronoWeekday::Sat | ChronoWeekday::Sun)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    fn utc_prefs() -> UserTimePrefs {
        UserTimePrefs::default()
    }

    #[test]
    fn test_relative_minutes() {
        let now = Utc::now();
        let spec = TimeSpec {
            spec_type: TimeSpecType::Relative,
            anchor: Some("now".to_string()),
            offset_minutes: 20,
            ..Default::default()
        };

        let result = calculate_from_time_spec(&spec, now, &utc_prefs()).unwrap();
        let diff = result - now;
        assert!(diff.num_minutes() >= 19 && diff.num_minutes() <= 21);
    }

    #[test]
    fn test_relative_hours() {
        let now = Utc::now();
        let spec = TimeSpec {
            spec_type: TimeSpecType::Relative,
            anchor: Some("now".to_string()),
            offset_hours: 2,
            ..Default::default()
        };

        let result = calculate_from_time_spec(&spec, now, &utc_prefs()).unwrap();
        let diff = result - now;
        assert!(diff.num_hours() >= 1 && diff.num_hours() <= 2);
    }

    #[test]
    fn test_weekday() {
        let now = Utc::now();
        let spec = TimeSpec {
            spec_type: TimeSpecType::Weekday,
            weekday: Some(Weekday::Monday),
            time: Some("18:00".to_string()),
            ..Default::default()
        };

        let result = calculate_from_time_spec(&spec, now, &utc_prefs()).unwrap();
        assert_eq!(result.weekday(), ChronoWeekday::Mon);
        assert_eq!(result.hour(), 18);
    }

    #[test]
    fn test_time_of_day() {
        let now = Utc::now();
        let spec = TimeSpec {
            spec_type: TimeSpecType::Daily,
            time_of_day: Some(TimeOfDay::Morning),
            ..Default::default()
        };

        let prefs = UserTimePrefs {
            morning: NaiveTime::from_hms_opt(9, 30, 0).unwrap(),
            ..Default::default()
        };

        let result = calculate_from_time_spec(&spec, now, &prefs).unwrap();
        assert_eq!(result.hour(), 9);
        assert_eq!(result.minute(), 30);
    }

    #[test]
    fn test_days_in_month() {
        assert_eq!(days_in_month(2024, 2), 29); // leap year
        assert_eq!(days_in_month(2023, 2), 28); // non-leap
        assert_eq!(days_in_month(2023, 1), 31);
        assert_eq!(days_in_month(2023, 4), 30);
    }

    #[test]
    fn test_timezone_offset_parsing() {
        // Test UTC+7 parsing
        let prefs = UserTimePrefs::from_db("8:00", "14:00", "19:00", "", "+07:00");
        assert_eq!(prefs.offset_seconds, 7 * 3600); // 7 hours in seconds

        // Test negative offset
        let prefs = UserTimePrefs::from_db("8:00", "14:00", "19:00", "", "-05:00");
        assert_eq!(prefs.offset_seconds, -5 * 3600);

        // Test UTC (default)
        let prefs = UserTimePrefs::from_db("8:00", "14:00", "19:00", "", "");
        assert_eq!(prefs.offset_seconds, 0);
    }

    #[test]
    fn test_time_conversion_with_timezone() {
        // Scenario: User in UTC+7 says "tomorrow at 9:30"
        // Expected: The time should be stored as 02:30 UTC (9:30 - 7 = 02:30)

        use chrono::TimeZone;

        // Create a fixed "now" for deterministic testing
        // Let's say it's 2025-12-11 03:00 UTC (10:00 in UTC+7)
        let now = Utc.with_ymd_and_hms(2025, 12, 11, 3, 0, 0).unwrap();

        // User timezone: UTC+7
        let prefs = UserTimePrefs {
            morning: NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            afternoon: NaiveTime::from_hms_opt(14, 0, 0).unwrap(),
            evening: NaiveTime::from_hms_opt(19, 0, 0).unwrap(),
            offset_seconds: 7 * 3600, // UTC+7
        };

        // TimeSpec: "tomorrow at 9:30" (in user's timezone)
        let spec = TimeSpec {
            spec_type: TimeSpecType::Relative,
            anchor: Some("today".to_string()),
            offset_days: 1,
            time: Some("09:30".to_string()),
            ..Default::default()
        };

        let result = calculate_from_time_spec(&spec, now, &prefs).unwrap();

        // In user's timezone (UTC+7), "now" is 2025-12-11 10:00
        // Tomorrow at 9:30 in UTC+7 = 2025-12-12 09:30 (UTC+7)
        // Convert to UTC: 2025-12-12 09:30 - 7h = 2025-12-12 02:30 UTC
        assert_eq!(result.year(), 2025);
        assert_eq!(result.month(), 12);
        assert_eq!(result.day(), 12);
        assert_eq!(result.hour(), 2); // 09:30 - 7h = 02:30 UTC
        assert_eq!(result.minute(), 30);
    }
}
