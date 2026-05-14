use chrono::{DateTime, Duration, Utc};

use crate::{ChatId, DomainError, ReminderId, Schedule, TimePreferences};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReminderStatus {
    Active,
    Processing,
    Retry {
        attempt: u32,
        retry_at: DateTime<Utc>,
    },
    Sent,
    Failed,
}

impl ReminderStatus {
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Processing => "processing",
            Self::Retry { .. } => "retry",
            Self::Sent => "sent",
            Self::Failed => "failed",
        }
    }

    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Sent | Self::Failed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl RetryPolicy {
    pub fn retry_delay(self, retry_count: u32) -> Duration {
        let multiplier = 1_i64 << retry_count.min(30);
        let seconds = self
            .base_delay
            .num_seconds()
            .saturating_mul(multiplier)
            .min(self.max_delay.num_seconds());
        Duration::seconds(seconds)
    }

    pub fn retry_at(self, now: DateTime<Utc>, retry_count: u32) -> DateTime<Utc> {
        now + self.retry_delay(retry_count)
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::seconds(30),
            max_delay: Duration::minutes(4),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reminder {
    pub id: Option<ReminderId>,
    pub chat_id: ChatId,
    pub text: String,
    pub schedule: Schedule,
    pub next_at: DateTime<Utc>,
    pub status: ReminderStatus,
    pub message_id: Option<i32>,
    pub snooze_until: Option<DateTime<Utc>>,
    pub retry_count: u32,
    pub retry_at: Option<DateTime<Utc>>,
}

impl Reminder {
    pub fn new(
        chat_id: ChatId,
        text: impl Into<String>,
        schedule: Schedule,
        next_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id: None,
            chat_id,
            text: text.into(),
            schedule,
            next_at,
            status: ReminderStatus::Active,
            message_id: None,
            snooze_until: None,
            retry_count: 0,
            retry_at: None,
        }
    }

    pub fn assign_id(&mut self, id: ReminderId) {
        self.id = Some(id);
    }

    pub fn claim(&mut self, now: DateTime<Utc>) -> Result<(), DomainError> {
        match &self.status {
            ReminderStatus::Active if self.next_at <= now => {
                self.status = ReminderStatus::Processing;
                Ok(())
            }
            ReminderStatus::Retry { retry_at, .. } if *retry_at <= now => {
                self.status = ReminderStatus::Processing;
                Ok(())
            }
            ReminderStatus::Active => Err(DomainError::ReminderNotDue {
                due_at: self.next_at,
                now,
            }),
            ReminderStatus::Retry { retry_at, .. } => Err(DomainError::ReminderNotDue {
                due_at: *retry_at,
                now,
            }),
            status => Err(invalid_transition(status, "processing")),
        }
    }

    pub fn mark_sent(&mut self) -> Result<(), DomainError> {
        match self.status {
            ReminderStatus::Processing => {
                self.status = ReminderStatus::Sent;
                self.retry_at = None;
                Ok(())
            }
            ref status => Err(invalid_transition(status, "sent")),
        }
    }

    pub fn schedule_retry(
        &mut self,
        policy: RetryPolicy,
        now: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, DomainError> {
        if self.status != ReminderStatus::Processing {
            return Err(invalid_transition(&self.status, "retry"));
        }
        if self.retry_count >= policy.max_retries {
            self.mark_failed()?;
            return Err(DomainError::MaxRetriesExceeded {
                attempts: self.retry_count,
            });
        }

        let retry_at = policy.retry_at(now, self.retry_count);
        self.retry_count += 1;
        self.retry_at = Some(retry_at);
        self.status = ReminderStatus::Retry {
            attempt: self.retry_count,
            retry_at,
        };
        Ok(retry_at)
    }

    pub fn mark_failed(&mut self) -> Result<(), DomainError> {
        if self.status.is_terminal() {
            return Err(invalid_transition(&self.status, "failed"));
        }

        self.status = ReminderStatus::Failed;
        self.retry_at = None;
        Ok(())
    }

    pub fn snooze(
        &mut self,
        now: DateTime<Utc>,
        minutes: i64,
    ) -> Result<DateTime<Utc>, DomainError> {
        if minutes <= 0 {
            return Err(DomainError::InvalidSnoozeMinutes(minutes));
        }

        let new_time = now + Duration::minutes(minutes);
        self.next_at = new_time;
        self.snooze_until = Some(new_time);
        self.retry_count = 0;
        self.retry_at = None;
        self.status = ReminderStatus::Active;
        Ok(new_time)
    }

    pub fn next_after_send(
        &mut self,
        now: DateTime<Utc>,
        prefs: &TimePreferences,
    ) -> Result<Option<DateTime<Utc>>, DomainError> {
        match self.schedule.next_after(now, prefs)? {
            Some(next) => {
                self.next_at = next;
                self.status = ReminderStatus::Active;
                self.retry_count = 0;
                self.retry_at = None;
                Ok(Some(next))
            }
            None => {
                self.status = ReminderStatus::Sent;
                self.retry_at = None;
                Ok(None)
            }
        }
    }
}

fn invalid_transition(status: &ReminderStatus, to: &'static str) -> DomainError {
    DomainError::InvalidStatusTransition {
        from: status.name(),
        to,
    }
}
