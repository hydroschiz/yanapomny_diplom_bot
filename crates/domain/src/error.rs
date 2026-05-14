use chrono::{DateTime, Utc};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("invalid UTC offset `{input}`")]
    InvalidUtcOffset { input: String },

    #[error("invalid time `{input}`")]
    InvalidTime { input: String },

    #[error("missing required field `{field}`")]
    MissingField { field: &'static str },

    #[error("invalid date")]
    InvalidDate,

    #[error("week {week} of month does not exist")]
    WeekOfMonthDoesNotExist { week: u32 },

    #[error("invalid status transition from {from} to {to}")]
    InvalidStatusTransition {
        from: &'static str,
        to: &'static str,
    },

    #[error("reminder is not due yet: due at {due_at}, now {now}")]
    ReminderNotDue {
        due_at: DateTime<Utc>,
        now: DateTime<Utc>,
    },

    #[error("max retry attempts exceeded: {attempts}")]
    MaxRetriesExceeded { attempts: u32 },

    #[error("invalid snooze minutes `{0}`")]
    InvalidSnoozeMinutes(i64),

    #[error("invalid months `{0}`")]
    InvalidMonths(i32),

    #[error("invalid money amount `{0}`")]
    InvalidMoneyAmount(i64),
}
