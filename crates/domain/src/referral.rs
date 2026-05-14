use chrono::{DateTime, Utc};

use crate::UserId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Referral {
    pub referrer_id: UserId,
    pub invited_id: UserId,
    pub created_at: DateTime<Utc>,
    pub rewarded_at: Option<DateTime<Utc>>,
}

impl Referral {
    pub fn new(referrer_id: UserId, invited_id: UserId, created_at: DateTime<Utc>) -> Self {
        Self {
            referrer_id,
            invited_id,
            created_at,
            rewarded_at: None,
        }
    }

    pub fn is_rewarded(&self) -> bool {
        self.rewarded_at.is_some()
    }

    pub fn mark_rewarded(&mut self, rewarded_at: DateTime<Utc>) {
        self.rewarded_at = Some(rewarded_at);
    }
}
