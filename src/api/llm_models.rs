//! Models for LLM API response parsing.
//!
//! These structures match the JSON format returned by the llm_api service
//! and cover ALL time specifications, recurrence patterns, anchors, and edge cases.

use serde::{Deserialize, Serialize};

// ============================================================================
// API Response wrapper
// ============================================================================

/// Top-level response from LLM API (`POST /api/v1/parse-reminder`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReminderResponse {
    pub status: String,
    #[serde(default)]
    pub reminder: Option<ParsedReminder>,
    #[serde(default)]
    pub error: Option<ErrorDetail>,
}

impl ReminderResponse {
    /// Returns true if the response indicates success.
    pub fn is_success(&self) -> bool {
        self.status == "success"
    }

    /// Returns the reminder if parsing was successful.
    pub fn into_reminder(self) -> Option<ParsedReminder> {
        if self.is_success() {
            self.reminder
        } else {
            None
        }
    }
}

/// Error details from LLM API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
}

// ============================================================================
// Parsed Reminder (from LLM)
// ============================================================================

/// Structured reminder extracted from natural language by LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedReminder {
    /// Description of what to remind about (in Russian).
    pub description: String,

    /// Type of reminder: "one_time" or "recurring".
    #[serde(rename = "type")]
    pub reminder_type: ReminderType,

    /// Time specification (when to trigger).
    #[serde(default)]
    pub time_spec: Option<TimeSpec>,

    /// Recurrence info (only for recurring reminders).
    #[serde(default)]
    pub recurrence: Option<RecurrenceInfo>,
}

/// Reminder type enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderType {
    OneTime,
    Recurring,
}

impl Default for ReminderType {
    fn default() -> Self {
        Self::OneTime
    }
}

// ============================================================================
// TimeSpec — Universal time specification
// ============================================================================

/// Time specification covering all possible temporal references.
///
/// # Examples from system prompt:
/// - "через 20 минут" → relative, anchor=now, offset_minutes=20
/// - "в понедельник в 18" → weekday=monday, time=18:00
/// - "завтра в 14" → relative, anchor=today, offset_days=1, time=14:00
/// - "16 сентября в 10:20" → absolute, date=09-16, time=10:20
/// - "каждое 28 число" → monthly, day_of_month=28
/// - "за неделю до 1 сентября" → absolute, date=09-01, offset_weeks=1, offset_direction=before
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TimeSpec {
    /// Type of time specification.
    /// Values: "relative", "weekday", "absolute", "monthly", "yearly", "daily"
    #[serde(rename = "type", default)]
    pub spec_type: TimeSpecType,

    // ========== Anchor (reference point) ==========
    /// Reference point for relative calculations.
    /// Values: "now", "today", "current_week", "next_week", "next_month", "YYYY-MM-DD", "YYYY-MM-DD HH:MM"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,

    // ========== Offsets (time shifts) ==========
    /// Offset in minutes (e.g., "через 20 минут").
    #[serde(default, skip_serializing_if = "is_zero")]
    pub offset_minutes: i32,

    /// Offset in hours (e.g., "через 2 часа").
    #[serde(default, skip_serializing_if = "is_zero")]
    pub offset_hours: i32,

    /// Offset in days (e.g., "через 3 дня", "завтра" = 1).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub offset_days: i32,

    /// Offset in weeks (e.g., "через 2 недели").
    #[serde(default, skip_serializing_if = "is_zero")]
    pub offset_weeks: i32,

    /// Offset in months (e.g., "через 3 месяца").
    #[serde(default, skip_serializing_if = "is_zero")]
    pub offset_months: i32,

    /// Offset in years (e.g., "через год").
    #[serde(default, skip_serializing_if = "is_zero")]
    pub offset_years: i32,

    /// Direction of offset: "after" (default) or "before".
    /// Example: "за неделю до" → offset_direction = "before"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset_direction: Option<OffsetDirection>,

    // ========== Weekday specification ==========
    /// Day of week for weekday-based reminders.
    /// Values: "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekday: Option<Weekday>,

    // ========== Date specification ==========
    /// Date in format "MM-DD" (e.g., "09-16" for September 16)
    /// or "DD.MM.YYYY" (e.g., "17.04.2025").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,

    /// Day of month (1-31) for monthly reminders.
    /// Example: "каждое 28 число" → day_of_month = 28
    #[serde(default, skip_serializing_if = "is_zero")]
    pub day_of_month: i32,

    // ========== Positional specification ==========
    /// Week of month for complex patterns.
    /// Values: 1 (first), 2 (second), 3 (third), 4 (fourth), -1 (last)
    /// Example: "вторую пятницу каждого месяца" → week_of_month = 2, weekday = friday
    #[serde(default, skip_serializing_if = "is_zero")]
    pub week_of_month: i32,

    /// Position descriptor for special cases.
    /// Values: "first", "second", "third", "fourth", "last"
    /// Example: "последний день месяца" → day_position = "last"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day_position: Option<DayPosition>,

    // ========== Time specification ==========
    /// Exact time in "HH:MM" format.
    /// Example: "в 14:30" → time = "14:30"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,

    /// Time of day when exact time is not specified.
    /// Values: "morning", "afternoon", "evening"
    /// Uses user's configured morning/afternoon/evening times.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_of_day: Option<TimeOfDay>,
}

/// Type of time specification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeSpecType {
    /// Relative to current time (offset from now/today).
    #[default]
    Relative,
    /// Based on a weekday (e.g., "в понедельник").
    Weekday,
    /// Absolute date (specific date like "16 сентября").
    Absolute,
    /// Monthly pattern (e.g., "каждое 28 число").
    Monthly,
    /// Yearly pattern (e.g., "каждое 30 мая").
    Yearly,
    /// Daily pattern (e.g., "каждый день").
    Daily,
}

/// Offset direction for time calculations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OffsetDirection {
    /// After the anchor point (default).
    #[default]
    After,
    /// Before the anchor point (e.g., "за неделю до").
    Before,
}

/// Days of the week.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    /// Convert to chrono::Weekday.
    pub fn to_chrono(&self) -> chrono::Weekday {
        match self {
            Weekday::Monday => chrono::Weekday::Mon,
            Weekday::Tuesday => chrono::Weekday::Tue,
            Weekday::Wednesday => chrono::Weekday::Wed,
            Weekday::Thursday => chrono::Weekday::Thu,
            Weekday::Friday => chrono::Weekday::Fri,
            Weekday::Saturday => chrono::Weekday::Sat,
            Weekday::Sunday => chrono::Weekday::Sun,
        }
    }
}

/// Time of day for approximate time references.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeOfDay {
    Morning,
    Afternoon,
    Evening,
}

/// Day position within a month.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DayPosition {
    First,
    Second,
    Third,
    Fourth,
    Last,
}

// ============================================================================
// RecurrenceInfo — Recurrence patterns
// ============================================================================

/// Recurrence information for repeating reminders.
///
/// # Key disambiguation (from system prompt):
///
/// ## "каждую вторую пятницу" (NO "месяца" word):
/// → ALTERNATION: pattern=weekly, interval=2
/// Means: every 2nd week on Friday
///
/// ## "вторую пятницу каждого месяца" (WITH "месяца" word):
/// → POSITION IN MONTH: week_of_month=2, pattern=monthly
/// Means: 2nd Friday of every month
///
/// ## "каждые N недель по [weekday]":
/// → ALTERNATION: pattern=weekly, interval=N
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecurrenceInfo {
    /// Recurrence pattern.
    /// Values: "daily", "weekly", "monthly", "yearly", "custom"
    #[serde(default)]
    pub pattern: RecurrencePattern,

    /// Interval between occurrences.
    /// Example: interval=2 with pattern=weekly means every 2 weeks.
    #[serde(default = "default_interval")]
    pub interval: i32,

    /// Filters for recurrence.
    /// Values: "weekdays" (пн-пт), "weekends" (сб-вс)
    /// Example: "по будням" → filters = ["weekdays"]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<RecurrenceFilter>,

    /// Interval unit for custom patterns.
    /// Values: "days", "weeks", "months", "years"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_unit: Option<IntervalUnit>,

    /// Week of month for complex monthly patterns.
    /// Example: "каждую вторую пятницу месяца" → week_of_month=2
    #[serde(default, skip_serializing_if = "is_zero")]
    pub week_of_month: i32,

    /// Day position for patterns like "последний день месяца".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day_position: Option<DayPosition>,
}

/// Recurrence pattern enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecurrencePattern {
    /// Every day.
    #[default]
    Daily,
    /// Every week (or every N weeks).
    Weekly,
    /// Every month (or every N months).
    Monthly,
    /// Every year.
    Yearly,
    /// Custom interval (uses interval_unit).
    Custom,
}

/// Recurrence filter for day selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecurrenceFilter {
    /// Monday through Friday.
    Weekdays,
    /// Saturday and Sunday.
    Weekends,
}

/// Interval unit for custom recurrence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntervalUnit {
    Days,
    Weeks,
    Months,
    Years,
}

// ============================================================================
// Helper functions
// ============================================================================

fn is_zero(val: &i32) -> bool {
    *val == 0
}

fn default_interval() -> i32 {
    1
}

// ============================================================================
// Request model (for sending to LLM API)
// ============================================================================

/// Request to parse a reminder from natural language.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseReminderRequest {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_timezone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_datetime: Option<String>,
}

impl ParseReminderRequest {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            user_timezone: None,
            user_datetime: None,
        }
    }

    pub fn with_context(text: impl Into<String>, timezone: String, datetime: String) -> Self {
        Self {
            text: text.into(),
            user_timezone: Some(timezone),
            user_datetime: Some(datetime),
        }
    }
}

// ============================================================================
// Conversion to legacy format
// ============================================================================

impl RecurrenceInfo {
    /// Convert to legacy delay string for backward compatibility with old DB.
    ///
    /// Legacy values: "", "day", "week", "month", "year", "weekday", "weekend"
    pub fn to_legacy_delay(&self) -> String {
        // Check filters first
        if self.filters.contains(&RecurrenceFilter::Weekdays) {
            return "weekday".to_string();
        }
        if self.filters.contains(&RecurrenceFilter::Weekends) {
            return "weekend".to_string();
        }

        match self.pattern {
            RecurrencePattern::Daily => "day".to_string(),
            RecurrencePattern::Weekly => "week".to_string(),
            RecurrencePattern::Monthly => "month".to_string(),
            RecurrencePattern::Yearly => "year".to_string(),
            RecurrencePattern::Custom => match self.interval_unit {
                Some(IntervalUnit::Days) => "day".to_string(),
                Some(IntervalUnit::Weeks) => "week".to_string(),
                Some(IntervalUnit::Months) => "month".to_string(),
                Some(IntervalUnit::Years) => "year".to_string(),
                None => "day".to_string(),
            },
        }
    }
}

impl ParsedReminder {
    /// Convert to legacy delay string.
    pub fn to_legacy_delay(&self) -> String {
        match self.reminder_type {
            ReminderType::OneTime => String::new(),
            ReminderType::Recurring => self
                .recurrence
                .as_ref()
                .map(|r| r.to_legacy_delay())
                .unwrap_or_default(),
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_relative_time() {
        let json = r#"{
            "status": "success",
            "reminder": {
                "description": "позвонить руководителю",
                "type": "one_time",
                "time_spec": {
                    "type": "relative",
                    "offset_minutes": 20,
                    "anchor": "now"
                }
            }
        }"#;

        let response: ReminderResponse = serde_json::from_str(json).unwrap();
        assert!(response.is_success());

        let reminder = response.reminder.unwrap();
        assert_eq!(reminder.description, "позвонить руководителю");
        assert_eq!(reminder.reminder_type, ReminderType::OneTime);

        let ts = reminder.time_spec.unwrap();
        assert_eq!(ts.spec_type, TimeSpecType::Relative);
        assert_eq!(ts.offset_minutes, 20);
        assert_eq!(ts.anchor, Some("now".to_string()));
    }

    #[test]
    fn test_parse_weekday() {
        let json = r#"{
            "status": "success",
            "reminder": {
                "description": "в поликлинику",
                "type": "one_time",
                "time_spec": {
                    "type": "weekday",
                    "weekday": "monday",
                    "time": "18:00"
                }
            }
        }"#;

        let response: ReminderResponse = serde_json::from_str(json).unwrap();
        let reminder = response.reminder.unwrap();
        let ts = reminder.time_spec.unwrap();

        assert_eq!(ts.spec_type, TimeSpecType::Weekday);
        assert_eq!(ts.weekday, Some(Weekday::Monday));
        assert_eq!(ts.time, Some("18:00".to_string()));
    }

    #[test]
    fn test_parse_recurring_daily() {
        let json = r#"{
            "status": "success",
            "reminder": {
                "description": "домой",
                "type": "recurring",
                "time_spec": {
                    "type": "daily",
                    "time": "18:00"
                },
                "recurrence": {
                    "pattern": "daily",
                    "interval": 1
                }
            }
        }"#;

        let response: ReminderResponse = serde_json::from_str(json).unwrap();
        let reminder = response.reminder.unwrap();

        assert_eq!(reminder.reminder_type, ReminderType::Recurring);
        assert_eq!(reminder.to_legacy_delay(), "day");

        let rec = reminder.recurrence.unwrap();
        assert_eq!(rec.pattern, RecurrencePattern::Daily);
        assert_eq!(rec.interval, 1);
    }

    #[test]
    fn test_parse_recurring_weekdays() {
        let json = r#"{
            "status": "success",
            "reminder": {
                "description": "планёрка",
                "type": "recurring",
                "time_spec": {
                    "type": "daily",
                    "time": "10:00"
                },
                "recurrence": {
                    "pattern": "daily",
                    "interval": 1,
                    "filters": ["weekdays"]
                }
            }
        }"#;

        let response: ReminderResponse = serde_json::from_str(json).unwrap();
        let reminder = response.reminder.unwrap();

        assert_eq!(reminder.to_legacy_delay(), "weekday");

        let rec = reminder.recurrence.unwrap();
        assert!(rec.filters.contains(&RecurrenceFilter::Weekdays));
    }

    #[test]
    fn test_parse_monthly_with_day() {
        let json = r#"{
            "status": "success",
            "reminder": {
                "description": "оплатить интернет",
                "type": "recurring",
                "time_spec": {
                    "type": "monthly",
                    "day_of_month": 28,
                    "time": "20:00"
                },
                "recurrence": {
                    "pattern": "monthly",
                    "interval": 1
                }
            }
        }"#;

        let response: ReminderResponse = serde_json::from_str(json).unwrap();
        let reminder = response.reminder.unwrap();

        // Check legacy delay before moving time_spec
        assert_eq!(reminder.to_legacy_delay(), "month");

        let ts = reminder.time_spec.unwrap();
        assert_eq!(ts.spec_type, TimeSpecType::Monthly);
        assert_eq!(ts.day_of_month, 28);
    }

    #[test]
    fn test_parse_yearly() {
        let json = r#"{
            "status": "success",
            "reminder": {
                "description": "подарок на годовщину",
                "type": "recurring",
                "time_spec": {
                    "type": "yearly",
                    "date": "05-30"
                },
                "recurrence": {
                    "pattern": "yearly",
                    "interval": 1
                }
            }
        }"#;

        let response: ReminderResponse = serde_json::from_str(json).unwrap();
        let reminder = response.reminder.unwrap();

        // Check legacy delay before moving time_spec
        assert_eq!(reminder.to_legacy_delay(), "year");

        let ts = reminder.time_spec.unwrap();
        assert_eq!(ts.spec_type, TimeSpecType::Yearly);
        assert_eq!(ts.date, Some("05-30".to_string()));
    }

    #[test]
    fn test_parse_offset_before() {
        let json = r#"{
            "status": "success",
            "reminder": {
                "description": "подготовка",
                "type": "one_time",
                "time_spec": {
                    "type": "absolute",
                    "date": "09-01",
                    "offset_weeks": 1,
                    "offset_direction": "before"
                }
            }
        }"#;

        let response: ReminderResponse = serde_json::from_str(json).unwrap();
        let reminder = response.reminder.unwrap();
        let ts = reminder.time_spec.unwrap();

        assert_eq!(ts.spec_type, TimeSpecType::Absolute);
        assert_eq!(ts.offset_weeks, 1);
        assert_eq!(ts.offset_direction, Some(OffsetDirection::Before));
    }

    #[test]
    fn test_parse_week_of_month() {
        // "вторую пятницу каждого месяца"
        let json = r#"{
            "status": "success",
            "reminder": {
                "description": "",
                "type": "recurring",
                "time_spec": {
                    "type": "weekday",
                    "weekday": "friday",
                    "week_of_month": 2,
                    "time": "18:00"
                },
                "recurrence": {
                    "pattern": "monthly",
                    "interval": 1
                }
            }
        }"#;

        let response: ReminderResponse = serde_json::from_str(json).unwrap();
        let reminder = response.reminder.unwrap();
        let ts = reminder.time_spec.unwrap();

        assert_eq!(ts.weekday, Some(Weekday::Friday));
        assert_eq!(ts.week_of_month, 2);

        let rec = reminder.recurrence.unwrap();
        assert_eq!(rec.pattern, RecurrencePattern::Monthly);
    }

    #[test]
    fn test_parse_alternating_weeks() {
        // "каждую вторую пятницу" (без "месяца") = каждые 2 недели
        let json = r#"{
            "status": "success",
            "reminder": {
                "description": "",
                "type": "recurring",
                "time_spec": {
                    "type": "weekday",
                    "weekday": "friday",
                    "time": "18:00"
                },
                "recurrence": {
                    "pattern": "weekly",
                    "interval": 2
                }
            }
        }"#;

        let response: ReminderResponse = serde_json::from_str(json).unwrap();
        let reminder = response.reminder.unwrap();

        let rec = reminder.recurrence.unwrap();
        assert_eq!(rec.pattern, RecurrencePattern::Weekly);
        assert_eq!(rec.interval, 2); // Every 2 weeks!
    }

    #[test]
    fn test_parse_last_day_of_month() {
        let json = r#"{
            "status": "success",
            "reminder": {
                "description": "",
                "type": "recurring",
                "time_spec": {
                    "type": "monthly",
                    "day_position": "last",
                    "time": "20:00"
                },
                "recurrence": {
                    "pattern": "monthly",
                    "interval": 1
                }
            }
        }"#;

        let response: ReminderResponse = serde_json::from_str(json).unwrap();
        let reminder = response.reminder.unwrap();
        let ts = reminder.time_spec.unwrap();

        assert_eq!(ts.day_position, Some(DayPosition::Last));
    }

    #[test]
    fn test_parse_time_of_day() {
        let json = r#"{
            "status": "success",
            "reminder": {
                "description": "оформить документы",
                "type": "one_time",
                "time_spec": {
                    "type": "weekday",
                    "weekday": "wednesday",
                    "time_of_day": "morning"
                }
            }
        }"#;

        let response: ReminderResponse = serde_json::from_str(json).unwrap();
        let reminder = response.reminder.unwrap();
        let ts = reminder.time_spec.unwrap();

        assert_eq!(ts.weekday, Some(Weekday::Wednesday));
        assert_eq!(ts.time_of_day, Some(TimeOfDay::Morning));
        assert!(ts.time.is_none()); // No exact time
    }

    #[test]
    fn test_parse_error_response() {
        let json = r#"{
            "status": "error",
            "error": {
                "code": "LLM_ERROR",
                "message": "Failed to parse"
            }
        }"#;

        let response: ReminderResponse = serde_json::from_str(json).unwrap();
        assert!(!response.is_success());
        assert!(response.reminder.is_none());

        let error = response.error.unwrap();
        assert_eq!(error.code, "LLM_ERROR");
    }

    #[test]
    fn test_weekday_to_chrono() {
        assert_eq!(Weekday::Monday.to_chrono(), chrono::Weekday::Mon);
        assert_eq!(Weekday::Friday.to_chrono(), chrono::Weekday::Fri);
        assert_eq!(Weekday::Sunday.to_chrono(), chrono::Weekday::Sun);
    }

    #[test]
    fn test_legacy_delay_conversion() {
        let rec_daily = RecurrenceInfo {
            pattern: RecurrencePattern::Daily,
            interval: 1,
            ..Default::default()
        };
        assert_eq!(rec_daily.to_legacy_delay(), "day");

        let rec_weekly = RecurrenceInfo {
            pattern: RecurrencePattern::Weekly,
            interval: 1,
            ..Default::default()
        };
        assert_eq!(rec_weekly.to_legacy_delay(), "week");

        let rec_weekdays = RecurrenceInfo {
            pattern: RecurrencePattern::Daily,
            interval: 1,
            filters: vec![RecurrenceFilter::Weekdays],
            ..Default::default()
        };
        assert_eq!(rec_weekdays.to_legacy_delay(), "weekday");
    }
}
