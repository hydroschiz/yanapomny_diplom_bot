use chrono::{
    DateTime, Datelike, Duration, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, Utc,
    Weekday as ChronoWeekday,
};

use crate::{DomainError, TimePreferences};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeSpecType {
    #[default]
    Relative,
    Weekday,
    Absolute,
    Monthly,
    Yearly,
    Daily,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OffsetDirection {
    #[default]
    After,
    Before,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Weekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

impl Weekday {
    pub const fn to_chrono(self) -> ChronoWeekday {
        match self {
            Self::Monday => ChronoWeekday::Mon,
            Self::Tuesday => ChronoWeekday::Tue,
            Self::Wednesday => ChronoWeekday::Wed,
            Self::Thursday => ChronoWeekday::Thu,
            Self::Friday => ChronoWeekday::Fri,
            Self::Saturday => ChronoWeekday::Sat,
            Self::Sunday => ChronoWeekday::Sun,
        }
    }
}

impl From<ChronoWeekday> for Weekday {
    fn from(value: ChronoWeekday) -> Self {
        match value {
            ChronoWeekday::Mon => Self::Monday,
            ChronoWeekday::Tue => Self::Tuesday,
            ChronoWeekday::Wed => Self::Wednesday,
            ChronoWeekday::Thu => Self::Thursday,
            ChronoWeekday::Fri => Self::Friday,
            ChronoWeekday::Sat => Self::Saturday,
            ChronoWeekday::Sun => Self::Sunday,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeOfDay {
    Morning,
    Afternoon,
    Evening,
}

impl TimePreferences {
    pub fn time_for_day_part(&self, value: TimeOfDay) -> NaiveTime {
        match value {
            TimeOfDay::Morning => self.morning,
            TimeOfDay::Afternoon => self.afternoon,
            TimeOfDay::Evening => self.evening,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DayPosition {
    First,
    Second,
    Third,
    Fourth,
    Last,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TimeSpec {
    pub spec_type: TimeSpecType,
    pub anchor: Option<String>,
    pub offset_minutes: i32,
    pub offset_hours: i32,
    pub offset_days: i32,
    pub offset_weeks: i32,
    pub offset_months: i32,
    pub offset_years: i32,
    pub offset_direction: Option<OffsetDirection>,
    pub weekday: Option<Weekday>,
    pub date: Option<String>,
    pub day_of_month: i32,
    pub week_of_month: i32,
    pub day_position: Option<DayPosition>,
    pub time: Option<String>,
    pub time_of_day: Option<TimeOfDay>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RecurrencePattern {
    #[default]
    Daily,
    Weekly,
    Monthly,
    Yearly,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecurrenceFilter {
    Weekdays,
    Weekends,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntervalUnit {
    Days,
    Weeks,
    Months,
    Years,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecurrenceRule {
    pub pattern: RecurrencePattern,
    pub interval: i32,
    pub filters: Vec<RecurrenceFilter>,
    pub interval_unit: Option<IntervalUnit>,
    pub week_of_month: i32,
    pub day_position: Option<DayPosition>,
}

impl Default for RecurrenceRule {
    fn default() -> Self {
        Self {
            pattern: RecurrencePattern::Daily,
            interval: 1,
            filters: Vec::new(),
            interval_unit: None,
            week_of_month: 0,
            day_position: None,
        }
    }
}

impl RecurrenceRule {
    pub fn to_legacy_delay(&self) -> &'static str {
        if self.filters.contains(&RecurrenceFilter::Weekdays) {
            return "weekday";
        }
        if self.filters.contains(&RecurrenceFilter::Weekends) {
            return "weekend";
        }

        match self.pattern {
            RecurrencePattern::Daily => "day",
            RecurrencePattern::Weekly => "week",
            RecurrencePattern::Monthly => "month",
            RecurrencePattern::Yearly => "year",
            RecurrencePattern::Custom => match self.interval_unit {
                Some(IntervalUnit::Days) => "day",
                Some(IntervalUnit::Weeks) => "week",
                Some(IntervalUnit::Months) => "month",
                Some(IntervalUnit::Years) => "year",
                None => "day",
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Schedule {
    OneTime(TimeSpec),
    Recurring {
        time: TimeSpec,
        recurrence: RecurrenceRule,
    },
}

impl Schedule {
    pub fn next_at(
        &self,
        now: DateTime<Utc>,
        prefs: &TimePreferences,
    ) -> Result<DateTime<Utc>, DomainError> {
        match self {
            Self::OneTime(spec) => calculate_from_time_spec(spec, now, prefs),
            Self::Recurring { time, .. } => calculate_from_time_spec(time, now, prefs),
        }
    }

    pub fn next_after(
        &self,
        current: DateTime<Utc>,
        prefs: &TimePreferences,
    ) -> Result<Option<DateTime<Utc>>, DomainError> {
        match self {
            Self::OneTime(_) => Ok(None),
            Self::Recurring { recurrence, .. } => {
                calculate_next_occurrence(current, recurrence, prefs).map(Some)
            }
        }
    }

    pub fn legacy_delay(&self) -> &'static str {
        match self {
            Self::OneTime(_) => "",
            Self::Recurring { recurrence, .. } => recurrence.to_legacy_delay(),
        }
    }
}

pub fn calculate_from_time_spec(
    spec: &TimeSpec,
    now: DateTime<Utc>,
    prefs: &TimePreferences,
) -> Result<DateTime<Utc>, DomainError> {
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

    Ok(result.with_timezone(&Utc))
}

fn calculate_relative(
    spec: &TimeSpec,
    now: DateTime<FixedOffset>,
    prefs: &TimePreferences,
) -> Result<DateTime<FixedOffset>, DomainError> {
    let mut result = now;
    let user_offset = prefs.fixed_offset();

    if let Some(anchor) = &spec.anchor {
        match anchor.as_str() {
            "now" => {}
            "today" => {
                result = local_midnight(now.date_naive(), user_offset, now);
            }
            "current_week" => {
                let days_since_monday = now.weekday().num_days_from_monday();
                result = local_midnight(
                    (now - Duration::days(days_since_monday as i64)).date_naive(),
                    user_offset,
                    now,
                );
            }
            "next_week" => {
                let days_until_monday = 7 - now.weekday().num_days_from_monday();
                result = local_midnight(
                    (now + Duration::days(days_until_monday as i64)).date_naive(),
                    user_offset,
                    now,
                );
            }
            value => {
                if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
                    result = local_midnight(date, user_offset, now);
                } else if let Ok(date_time) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M")
                {
                    result = date_time
                        .and_local_timezone(user_offset)
                        .single()
                        .unwrap_or(now);
                }
            }
        }
    }

    let multiplier = direction_multiplier(spec.offset_direction);

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
        result = add_months_local(result, spec.offset_months * multiplier as i32, prefs);
    }
    if spec.offset_years != 0 {
        result = add_months_local(result, spec.offset_years * 12 * multiplier as i32, prefs);
    }

    Ok(apply_time(result, spec, prefs))
}

fn calculate_weekday(
    spec: &TimeSpec,
    now: DateTime<FixedOffset>,
    prefs: &TimePreferences,
) -> Result<DateTime<FixedOffset>, DomainError> {
    let target_weekday = spec
        .weekday
        .ok_or(DomainError::MissingField { field: "weekday" })?
        .to_chrono();

    let mut result = if spec.week_of_month != 0 {
        find_nth_weekday_of_month(now, target_weekday, spec.week_of_month, prefs)?
    } else {
        find_next_weekday(now, target_weekday)
    };

    result = apply_time(result, spec, prefs);
    if result <= now {
        result += Duration::weeks(1);
    }
    Ok(result)
}

fn calculate_absolute(
    spec: &TimeSpec,
    now: DateTime<FixedOffset>,
    prefs: &TimePreferences,
) -> Result<DateTime<FixedOffset>, DomainError> {
    let mut result = now;

    if let Some(date_str) = &spec.date {
        if let Some(parsed) = parse_date_mmdd(date_str, now, prefs) {
            result = parsed;
        } else if let Some(parsed) = parse_date_ddmmyyyy(date_str, prefs) {
            result = parsed;
        }
    } else if spec.day_of_month != 0 {
        result = set_day_of_month(now, spec.day_of_month as u32, prefs)?;
    }

    let multiplier = direction_multiplier(spec.offset_direction);
    if spec.offset_weeks != 0 {
        result += Duration::weeks(spec.offset_weeks as i64 * multiplier);
    }
    if spec.offset_days != 0 {
        result += Duration::days(spec.offset_days as i64 * multiplier);
    }

    Ok(apply_time(result, spec, prefs))
}

fn calculate_monthly(
    spec: &TimeSpec,
    now: DateTime<FixedOffset>,
    prefs: &TimePreferences,
) -> Result<DateTime<FixedOffset>, DomainError> {
    let mut result = now;

    if matches!(spec.day_position, Some(DayPosition::Last)) {
        result = last_day_of_month(now, prefs)?;
    } else if spec.day_of_month != 0 {
        result = set_day_of_month(now, spec.day_of_month as u32, prefs)?;
        if result <= now {
            result = add_months_local(result, 1, prefs);
            result = set_day_of_month(result, spec.day_of_month as u32, prefs)?;
        }
    } else if spec.week_of_month != 0 {
        let weekday = spec
            .weekday
            .map(Weekday::to_chrono)
            .unwrap_or(ChronoWeekday::Mon);
        result = find_nth_weekday_of_month(now, weekday, spec.week_of_month, prefs)?;
    }

    Ok(apply_time(result, spec, prefs))
}

fn calculate_yearly(
    spec: &TimeSpec,
    now: DateTime<FixedOffset>,
    prefs: &TimePreferences,
) -> Result<DateTime<FixedOffset>, DomainError> {
    let mut result = now;

    if let Some(date_str) = &spec.date {
        if let Some((month, day)) = parse_mmdd(date_str) {
            result = set_month_day(now, month, day, prefs)?;
            if result <= now {
                result = add_months_local(result, 12, prefs);
            }
        }
    }

    Ok(apply_time(result, spec, prefs))
}

fn calculate_daily(
    spec: &TimeSpec,
    now: DateTime<FixedOffset>,
    prefs: &TimePreferences,
) -> Result<DateTime<FixedOffset>, DomainError> {
    let mut result = apply_time(now, spec, prefs);
    if result <= now {
        result += Duration::days(1);
    }
    Ok(result)
}

pub fn calculate_next_occurrence(
    current: DateTime<Utc>,
    recurrence: &RecurrenceRule,
    prefs: &TimePreferences,
) -> Result<DateTime<Utc>, DomainError> {
    let interval = recurrence.interval.max(1);
    let current_local = current.with_timezone(&prefs.fixed_offset());

    let next = match recurrence.pattern {
        RecurrencePattern::Daily => {
            let mut next = current_local + Duration::days(interval as i64);
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
        RecurrencePattern::Monthly => add_months_local(current_local, interval, prefs),
        RecurrencePattern::Yearly => add_months_local(current_local, interval * 12, prefs),
        RecurrencePattern::Custom => match recurrence.interval_unit {
            Some(IntervalUnit::Days) => current_local + Duration::days(interval as i64),
            Some(IntervalUnit::Weeks) => current_local + Duration::weeks(interval as i64),
            Some(IntervalUnit::Months) => add_months_local(current_local, interval, prefs),
            Some(IntervalUnit::Years) => add_months_local(current_local, interval * 12, prefs),
            None => current_local + Duration::days(interval as i64),
        },
    };

    Ok(next.with_timezone(&Utc))
}

fn apply_time(
    dt: DateTime<FixedOffset>,
    spec: &TimeSpec,
    prefs: &TimePreferences,
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

    if let Some(time_of_day) = spec.time_of_day {
        return dt
            .date_naive()
            .and_time(prefs.time_for_day_part(time_of_day))
            .and_local_timezone(user_offset)
            .single()
            .unwrap_or(dt);
    }

    dt
}

fn direction_multiplier(direction: Option<OffsetDirection>) -> i64 {
    match direction.unwrap_or(OffsetDirection::After) {
        OffsetDirection::After => 1,
        OffsetDirection::Before => -1,
    }
}

fn local_midnight(
    date: NaiveDate,
    offset: FixedOffset,
    fallback: DateTime<FixedOffset>,
) -> DateTime<FixedOffset> {
    date.and_hms_opt(0, 0, 0)
        .and_then(|date_time| date_time.and_local_timezone(offset).single())
        .unwrap_or(fallback)
}

fn parse_time_hhmm(value: &str) -> Option<NaiveTime> {
    let parts: Vec<&str> = value.split(':').collect();
    if parts.len() < 2 {
        return None;
    }

    let hour: u32 = parts[0].parse().ok()?;
    let minute: u32 = parts[1].parse().ok()?;
    NaiveTime::from_hms_opt(hour, minute, 0)
}

fn parse_mmdd(value: &str) -> Option<(u32, u32)> {
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() != 2 {
        return None;
    }

    Some((parts[0].parse().ok()?, parts[1].parse().ok()?))
}

fn parse_date_mmdd(
    value: &str,
    now: DateTime<FixedOffset>,
    prefs: &TimePreferences,
) -> Option<DateTime<FixedOffset>> {
    let (month, day) = parse_mmdd(value)?;
    set_month_day(now, month, day, prefs).ok()
}

fn parse_date_ddmmyyyy(value: &str, prefs: &TimePreferences) -> Option<DateTime<FixedOffset>> {
    let offset = prefs.fixed_offset();
    let parts: Vec<&str> = value.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    let day: u32 = parts[0].parse().ok()?;
    let month: u32 = parts[1].parse().ok()?;
    let year: i32 = parts[2].parse().ok()?;
    NaiveDate::from_ymd_opt(year, month, day)?
        .and_hms_opt(0, 0, 0)?
        .and_local_timezone(offset)
        .single()
}

fn set_day_of_month(
    dt: DateTime<FixedOffset>,
    day: u32,
    prefs: &TimePreferences,
) -> Result<DateTime<FixedOffset>, DomainError> {
    let max_day = days_in_month(dt.year(), dt.month());
    let actual_day = day.min(max_day);
    let date = NaiveDate::from_ymd_opt(dt.year(), dt.month(), actual_day)
        .ok_or(DomainError::InvalidDate)?;

    Ok(date
        .and_time(dt.time())
        .and_local_timezone(prefs.fixed_offset())
        .single()
        .unwrap_or(dt))
}

fn set_month_day(
    dt: DateTime<FixedOffset>,
    month: u32,
    day: u32,
    prefs: &TimePreferences,
) -> Result<DateTime<FixedOffset>, DomainError> {
    let max_day = days_in_month(dt.year(), month);
    let actual_day = day.min(max_day);
    let date =
        NaiveDate::from_ymd_opt(dt.year(), month, actual_day).ok_or(DomainError::InvalidDate)?;

    Ok(date
        .and_time(dt.time())
        .and_local_timezone(prefs.fixed_offset())
        .single()
        .unwrap_or(dt))
}

fn last_day_of_month(
    dt: DateTime<FixedOffset>,
    prefs: &TimePreferences,
) -> Result<DateTime<FixedOffset>, DomainError> {
    set_day_of_month(dt, days_in_month(dt.year(), dt.month()), prefs)
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 30,
    }
}

pub fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

pub fn add_months(dt: DateTime<Utc>, months: i32) -> DateTime<Utc> {
    let prefs = TimePreferences::default();
    add_months_local(dt.with_timezone(&prefs.fixed_offset()), months, &prefs).with_timezone(&Utc)
}

fn add_months_local(
    dt: DateTime<FixedOffset>,
    months: i32,
    prefs: &TimePreferences,
) -> DateTime<FixedOffset> {
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

    NaiveDate::from_ymd_opt(year, month as u32, day)
        .and_then(|date| {
            date.and_time(dt.time())
                .and_local_timezone(prefs.fixed_offset())
                .single()
        })
        .unwrap_or(dt)
}

fn find_next_weekday(from: DateTime<FixedOffset>, target: ChronoWeekday) -> DateTime<FixedOffset> {
    let current_num = from.weekday().num_days_from_monday();
    let target_num = target.num_days_from_monday();
    let days_ahead = if target_num >= current_num {
        target_num - current_num
    } else {
        7 - (current_num - target_num)
    };

    from + Duration::days(days_ahead as i64)
}

fn find_nth_weekday_of_month(
    now: DateTime<FixedOffset>,
    weekday: ChronoWeekday,
    week_of_month: i32,
    prefs: &TimePreferences,
) -> Result<DateTime<FixedOffset>, DomainError> {
    let year = now.year();
    let month = now.month();
    let first_of_month = NaiveDate::from_ymd_opt(year, month, 1).ok_or(DomainError::InvalidDate)?;

    if week_of_month == -1 {
        let mut day = NaiveDate::from_ymd_opt(year, month, days_in_month(year, month))
            .ok_or(DomainError::InvalidDate)?;
        while day.weekday() != weekday {
            day = day.pred_opt().ok_or(DomainError::InvalidDate)?;
        }
        return date_at_midnight(day, prefs);
    }

    let n = week_of_month.max(1) as u32;
    let mut day = first_of_month;
    while day.weekday() != weekday {
        day = day.succ_opt().ok_or(DomainError::InvalidDate)?;
    }

    day += Duration::weeks((n - 1) as i64);
    if day.month() != month {
        return Err(DomainError::WeekOfMonthDoesNotExist { week: n });
    }

    date_at_midnight(day, prefs)
}

fn date_at_midnight(
    date: NaiveDate,
    prefs: &TimePreferences,
) -> Result<DateTime<FixedOffset>, DomainError> {
    date.and_hms_opt(0, 0, 0)
        .and_then(|date_time| date_time.and_local_timezone(prefs.fixed_offset()).single())
        .ok_or(DomainError::InvalidDate)
}

pub fn is_weekday(weekday: ChronoWeekday) -> bool {
    matches!(
        weekday,
        ChronoWeekday::Mon
            | ChronoWeekday::Tue
            | ChronoWeekday::Wed
            | ChronoWeekday::Thu
            | ChronoWeekday::Fri
    )
}

pub fn is_weekend(weekday: ChronoWeekday) -> bool {
    matches!(weekday, ChronoWeekday::Sat | ChronoWeekday::Sun)
}
