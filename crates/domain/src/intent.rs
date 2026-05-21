use chrono::{DateTime, Utc};

use crate::{IntentLogId, UserId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntentLog {
    pub id: Option<IntentLogId>,
    pub user_id: UserId,
    pub raw_text: String,
    pub interpreted_as: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl IntentLog {
    pub fn new(user_id: UserId, raw_text: impl Into<String>, created_at: DateTime<Utc>) -> Self {
        Self {
            id: None,
            user_id,
            raw_text: raw_text.into(),
            interpreted_as: None,
            created_at,
        }
    }

    pub fn assign_id(&mut self, id: IntentLogId) {
        self.id = Some(id);
    }

    pub fn mark_interpreted(&mut self, intent: impl Into<String>) {
        self.interpreted_as = Some(intent.into());
    }
}
