use application::{
    ApplicationError, ApplicationResult, InterpretedTask, NaturalLanguageInterpreter,
    NaturalLanguageReminderParser,
};
use async_trait::async_trait;
use chrono::Utc;
use domain::{
    DayPosition, IntervalUnit, OffsetDirection, RecurrenceFilter, RecurrencePattern,
    RecurrenceRule, Schedule, TimeOfDay, TimeSpec, TimeSpecType, User, Weekday,
};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct HttpLlmInterpreter {
    client: reqwest::Client,
    base_url: String,
}

impl HttpLlmInterpreter {
    pub fn new(base_url: impl Into<String>) -> ApplicationResult<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|err| ApplicationError::ExternalService(err.to_string()))?;
        Ok(Self {
            client,
            base_url: base_url.into(),
        })
    }

    async fn parse(&self, text: &str, user: &User) -> ApplicationResult<ReminderResponseDto> {
        let url = format!(
            "{}/api/v1/parse-reminder",
            self.base_url.trim_end_matches('/')
        );
        let now = Utc::now();
        let request = ParseReminderRequestDto {
            text,
            user_timezone: user.time_preferences.utc_offset.to_string(),
            user_datetime: now.to_rfc3339(),
        };
        let response = self
            .client
            .post(url)
            .json(&request)
            .send()
            .await
            .map_err(|err| ApplicationError::ExternalService(err.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ApplicationError::ExternalService(format!(
                "LLM API failed: {status} {body}"
            )));
        }

        response
            .json()
            .await
            .map_err(|err| ApplicationError::ExternalService(err.to_string()))
    }
}

#[async_trait]
impl NaturalLanguageReminderParser for HttpLlmInterpreter {
    async fn parse_reminder(&self, text: &str, user: &User) -> ApplicationResult<Schedule> {
        let response = self.parse(text, user).await?;
        let reminder = response.reminder.ok_or_else(|| {
            ApplicationError::ExternalService(
                response
                    .error
                    .and_then(|error| error.message)
                    .unwrap_or_else(|| "LLM response has no reminder".to_string()),
            )
        })?;
        Ok(reminder.into_schedule())
    }
}

#[async_trait]
impl NaturalLanguageInterpreter for HttpLlmInterpreter {
    async fn interpret_task(&self, text: &str, user: &User) -> ApplicationResult<InterpretedTask> {
        let response = self.parse(text, user).await?;
        let reminder = response.reminder.ok_or_else(|| {
            ApplicationError::ExternalService(
                response
                    .error
                    .and_then(|error| error.message)
                    .unwrap_or_else(|| "LLM response has no reminder".to_string()),
            )
        })?;
        let schedule = reminder.clone().into_schedule();
        let trigger_at = schedule.next_at(Utc::now(), &user.time_preferences)?;
        Ok(InterpretedTask {
            title: reminder.description,
            description: None,
            schedule,
            trigger_at,
        })
    }
}

#[derive(Debug, Serialize)]
struct ParseReminderRequestDto<'a> {
    text: &'a str,
    user_timezone: String,
    user_datetime: String,
}

#[derive(Debug, Deserialize)]
struct ReminderResponseDto {
    reminder: Option<ParsedReminderDto>,
    error: Option<ErrorDto>,
}

#[derive(Debug, Deserialize)]
struct ErrorDto {
    message: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ParsedReminderDto {
    description: String,
    #[serde(default)]
    time_spec: Option<TimeSpecDto>,
    #[serde(default)]
    recurrence: Option<RecurrenceRuleDto>,
    #[serde(default)]
    reminder_type: Option<String>,
}

impl ParsedReminderDto {
    fn into_schedule(self) -> Schedule {
        let time = self.time_spec.unwrap_or_default().into();
        if self.reminder_type.as_deref() == Some("recurring") || self.recurrence.is_some() {
            Schedule::Recurring {
                time,
                recurrence: self.recurrence.unwrap_or_default().into(),
            }
        } else {
            Schedule::OneTime(time)
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct TimeSpecDto {
    #[serde(default, rename = "type")]
    spec_type: Option<String>,
    #[serde(default)]
    anchor: Option<String>,
    #[serde(default)]
    offset_minutes: i32,
    #[serde(default)]
    offset_hours: i32,
    #[serde(default)]
    offset_days: i32,
    #[serde(default)]
    offset_weeks: i32,
    #[serde(default)]
    offset_months: i32,
    #[serde(default)]
    offset_years: i32,
    #[serde(default)]
    offset_direction: Option<String>,
    #[serde(default)]
    weekday: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    day_of_month: i32,
    #[serde(default)]
    week_of_month: i32,
    #[serde(default)]
    day_position: Option<String>,
    #[serde(default)]
    time: Option<String>,
    #[serde(default)]
    time_of_day: Option<String>,
}

impl From<TimeSpecDto> for TimeSpec {
    fn from(value: TimeSpecDto) -> Self {
        Self {
            spec_type: time_spec_type_from_str(value.spec_type.as_deref().unwrap_or("relative")),
            anchor: value.anchor,
            offset_minutes: value.offset_minutes,
            offset_hours: value.offset_hours,
            offset_days: value.offset_days,
            offset_weeks: value.offset_weeks,
            offset_months: value.offset_months,
            offset_years: value.offset_years,
            offset_direction: value
                .offset_direction
                .as_deref()
                .map(offset_direction_from_str),
            weekday: value.weekday.as_deref().map(weekday_from_str),
            date: value.date,
            day_of_month: value.day_of_month,
            week_of_month: value.week_of_month,
            day_position: value.day_position.as_deref().map(day_position_from_str),
            time: value.time,
            time_of_day: value.time_of_day.as_deref().map(time_of_day_from_str),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct RecurrenceRuleDto {
    #[serde(default)]
    pattern: Option<String>,
    #[serde(default = "default_interval")]
    interval: i32,
    #[serde(default)]
    filters: Vec<String>,
    #[serde(default)]
    interval_unit: Option<String>,
    #[serde(default)]
    week_of_month: i32,
    #[serde(default)]
    day_position: Option<String>,
}

impl From<RecurrenceRuleDto> for RecurrenceRule {
    fn from(value: RecurrenceRuleDto) -> Self {
        Self {
            pattern: recurrence_pattern_from_str(value.pattern.as_deref().unwrap_or("daily")),
            interval: value.interval,
            filters: value
                .filters
                .iter()
                .map(String::as_str)
                .map(recurrence_filter_from_str)
                .collect(),
            interval_unit: value.interval_unit.as_deref().map(interval_unit_from_str),
            week_of_month: value.week_of_month,
            day_position: value.day_position.as_deref().map(day_position_from_str),
        }
    }
}

const fn default_interval() -> i32 {
    1
}

fn time_spec_type_from_str(value: &str) -> TimeSpecType {
    match value {
        "weekday" => TimeSpecType::Weekday,
        "absolute" => TimeSpecType::Absolute,
        "monthly" => TimeSpecType::Monthly,
        "yearly" => TimeSpecType::Yearly,
        "daily" => TimeSpecType::Daily,
        _ => TimeSpecType::Relative,
    }
}

fn offset_direction_from_str(value: &str) -> OffsetDirection {
    match value {
        "before" => OffsetDirection::Before,
        _ => OffsetDirection::After,
    }
}

fn weekday_from_str(value: &str) -> Weekday {
    match value {
        "tuesday" => Weekday::Tuesday,
        "wednesday" => Weekday::Wednesday,
        "thursday" => Weekday::Thursday,
        "friday" => Weekday::Friday,
        "saturday" => Weekday::Saturday,
        "sunday" => Weekday::Sunday,
        _ => Weekday::Monday,
    }
}

fn day_position_from_str(value: &str) -> DayPosition {
    match value {
        "second" => DayPosition::Second,
        "third" => DayPosition::Third,
        "fourth" => DayPosition::Fourth,
        "last" => DayPosition::Last,
        _ => DayPosition::First,
    }
}

fn time_of_day_from_str(value: &str) -> TimeOfDay {
    match value {
        "afternoon" => TimeOfDay::Afternoon,
        "evening" => TimeOfDay::Evening,
        _ => TimeOfDay::Morning,
    }
}

fn recurrence_pattern_from_str(value: &str) -> RecurrencePattern {
    match value {
        "weekly" => RecurrencePattern::Weekly,
        "monthly" => RecurrencePattern::Monthly,
        "yearly" => RecurrencePattern::Yearly,
        "custom" => RecurrencePattern::Custom,
        _ => RecurrencePattern::Daily,
    }
}

fn recurrence_filter_from_str(value: &str) -> RecurrenceFilter {
    match value {
        "weekends" => RecurrenceFilter::Weekends,
        _ => RecurrenceFilter::Weekdays,
    }
}

fn interval_unit_from_str(value: &str) -> IntervalUnit {
    match value {
        "weeks" => IntervalUnit::Weeks,
        "months" => IntervalUnit::Months,
        "years" => IntervalUnit::Years,
        _ => IntervalUnit::Days,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsed_reminder_maps_to_recurring_schedule() {
        let parsed = ParsedReminderDto {
            description: "test".to_string(),
            time_spec: Some(TimeSpecDto {
                time: Some("10:00".to_string()),
                ..TimeSpecDto::default()
            }),
            recurrence: Some(RecurrenceRuleDto {
                pattern: Some("weekly".to_string()),
                ..RecurrenceRuleDto::default()
            }),
            reminder_type: Some("recurring".to_string()),
        };

        assert!(matches!(parsed.into_schedule(), Schedule::Recurring { .. }));
    }
}
