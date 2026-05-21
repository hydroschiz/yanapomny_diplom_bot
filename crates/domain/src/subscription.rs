use chrono::{DateTime, Duration, Utc};

use crate::{scheduling::add_months, ChatId, Months, SubscriptionId, UserId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreeState {
    None,
    Trial,
    Paid,
    BonusWeek,
}

impl FreeState {
    pub const fn from_legacy_code(value: Option<i32>) -> Self {
        match value {
            Some(1) => Self::Trial,
            Some(2) => Self::Paid,
            Some(3) => Self::BonusWeek,
            _ => Self::None,
        }
    }

    pub const fn legacy_code(self) -> i32 {
        match self {
            Self::None => 0,
            Self::Trial => 1,
            Self::Paid => 2,
            Self::BonusWeek => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionStatus {
    Trial { until: DateTime<Utc> },
    Active { until: DateTime<Utc> },
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubscriptionPolicy {
    pub trial_days: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SubscriptionPlan {
    #[default]
    Basic,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SubscriptionSource {
    #[default]
    Trial,
    Payment,
    ReferralReward,
    AdminGrant,
}

impl Default for SubscriptionPolicy {
    fn default() -> Self {
        Self { trial_days: 7 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subscription {
    pub id: Option<SubscriptionId>,
    pub user_id: Option<UserId>,
    pub chat_id: ChatId,
    pub plan: SubscriptionPlan,
    pub source: SubscriptionSource,
    pub is_group: bool,
    pub group_name: String,
    pub owner_id: Option<UserId>,
    pub expires_at: DateTime<Utc>,
    pub active: bool,
    pub free_state: FreeState,
}

impl Subscription {
    pub fn new_trial(chat_id: ChatId, now: DateTime<Utc>, policy: SubscriptionPolicy) -> Self {
        Self {
            id: None,
            user_id: None,
            chat_id,
            plan: SubscriptionPlan::Basic,
            source: SubscriptionSource::Trial,
            is_group: false,
            group_name: String::new(),
            owner_id: None,
            expires_at: now + Duration::days(policy.trial_days),
            active: true,
            free_state: FreeState::Trial,
        }
    }

    pub fn mark_group(&mut self, name: impl Into<String>, owner_id: UserId) {
        self.is_group = true;
        self.group_name = name.into();
        self.owner_id = Some(owner_id);
    }

    pub fn assign_id(&mut self, id: SubscriptionId) {
        self.id = Some(id);
    }

    pub fn link_user(&mut self, user_id: UserId) {
        self.user_id = Some(user_id);
    }

    pub fn is_active(&self, now: DateTime<Utc>) -> bool {
        self.expires_at > now
    }

    pub fn status(&self, now: DateTime<Utc>) -> SubscriptionStatus {
        if !self.is_active(now) {
            return SubscriptionStatus::Expired;
        }

        match self.free_state {
            FreeState::Trial => SubscriptionStatus::Trial {
                until: self.expires_at,
            },
            _ => SubscriptionStatus::Active {
                until: self.expires_at,
            },
        }
    }

    pub fn extend(&mut self, months: Months, now: DateTime<Utc>) -> DateTime<Utc> {
        let base = if self.expires_at > now {
            self.expires_at
        } else {
            now
        };

        self.expires_at = add_months(base, months.value() as i32);
        self.active = true;
        self.free_state = FreeState::Paid;
        self.source = SubscriptionSource::Payment;
        self.expires_at
    }
}
