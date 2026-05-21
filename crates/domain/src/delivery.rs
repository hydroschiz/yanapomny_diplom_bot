use chrono::{DateTime, Utc};

use crate::{DeliveryEventId, ReminderId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryChannel {
    Vk,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryResult {
    Planned,
    Sent,
    TemporaryFailure { error_code: Option<String> },
    PermanentFailure { error_code: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryEvent {
    pub id: Option<DeliveryEventId>,
    pub reminder_id: ReminderId,
    pub channel: DeliveryChannel,
    pub planned_at: DateTime<Utc>,
    pub sent_at: Option<DateTime<Utc>>,
    pub result: DeliveryResult,
}

impl DeliveryEvent {
    pub fn planned(
        reminder_id: ReminderId,
        channel: DeliveryChannel,
        planned_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id: None,
            reminder_id,
            channel,
            planned_at,
            sent_at: None,
            result: DeliveryResult::Planned,
        }
    }

    pub fn assign_id(&mut self, id: DeliveryEventId) {
        self.id = Some(id);
    }

    pub fn mark_sent(&mut self, sent_at: DateTime<Utc>) {
        self.sent_at = Some(sent_at);
        self.result = DeliveryResult::Sent;
    }

    pub fn mark_temporary_failure(&mut self, error_code: Option<String>) {
        self.result = DeliveryResult::TemporaryFailure { error_code };
    }

    pub fn mark_permanent_failure(&mut self, error_code: Option<String>) {
        self.result = DeliveryResult::PermanentFailure { error_code };
    }
}
