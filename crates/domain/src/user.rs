use chrono::{DateTime, Duration, Utc};

use crate::{PlatformIdentity, TimePreferences, UserId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserStatus {
    Active,
    Blocked,
    Deleted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SnoozeDuration {
    minutes: u32,
}

impl SnoozeDuration {
    pub const FIVE_MINUTES: Self = Self { minutes: 5 };
    pub const FIFTEEN_MINUTES: Self = Self { minutes: 15 };
    pub const ONE_HOUR: Self = Self { minutes: 60 };
    pub const THREE_HOURS: Self = Self { minutes: 180 };
    pub const ONE_DAY: Self = Self { minutes: 24 * 60 };

    pub const fn from_minutes(minutes: u32) -> Self {
        Self { minutes }
    }

    pub const fn minutes(self) -> u32 {
        self.minutes
    }

    pub fn duration(self) -> Duration {
        Duration::minutes(self.minutes as i64)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    pub id: UserId,
    pub status: UserStatus,
    pub created_at: Option<DateTime<Utc>>,
    pub identities: Vec<PlatformIdentity>,
    pub time_preferences: TimePreferences,
    pub snooze_buttons: Vec<SnoozeDuration>,
    pub auto_snooze: SnoozeDuration,
    pub payment_info: Option<String>,
}

impl User {
    pub fn new(id: UserId) -> Self {
        Self {
            id,
            status: UserStatus::Active,
            created_at: None,
            identities: Vec::new(),
            time_preferences: TimePreferences::default(),
            snooze_buttons: vec![
                SnoozeDuration::ONE_HOUR,
                SnoozeDuration::THREE_HOURS,
                SnoozeDuration::ONE_DAY,
            ],
            auto_snooze: SnoozeDuration::FIFTEEN_MINUTES,
            payment_info: None,
        }
    }

    pub fn registered(id: UserId, created_at: DateTime<Utc>) -> Self {
        Self {
            created_at: Some(created_at),
            ..Self::new(id)
        }
    }

    pub fn with_time_preferences(mut self, preferences: TimePreferences) -> Self {
        self.time_preferences = preferences;
        self
    }

    pub fn set_snooze_buttons(&mut self, buttons: Vec<SnoozeDuration>) {
        self.snooze_buttons = buttons;
    }

    pub fn set_auto_snooze(&mut self, value: SnoozeDuration) {
        self.auto_snooze = value;
    }

    pub fn add_identity(&mut self, identity: PlatformIdentity) {
        self.identities.retain(|existing| {
            existing.platform != identity.platform || existing.external_id != identity.external_id
        });
        self.identities.push(identity);
    }

    pub fn preferences(&self) -> UserPreferences {
        UserPreferences {
            user_id: self.id,
            time_preferences: self.time_preferences.clone(),
            language: Language::Russian,
            snooze_policy: SnoozePolicy {
                buttons: self.snooze_buttons.clone(),
                auto_snooze: self.auto_snooze,
            },
            notification_policy: NotificationPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Russian,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnoozePolicy {
    pub buttons: Vec<SnoozeDuration>,
    pub auto_snooze: SnoozeDuration,
}

impl Default for SnoozePolicy {
    fn default() -> Self {
        Self {
            buttons: vec![
                SnoozeDuration::ONE_HOUR,
                SnoozeDuration::THREE_HOURS,
                SnoozeDuration::ONE_DAY,
            ],
            auto_snooze: SnoozeDuration::FIFTEEN_MINUTES,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationPolicy {
    pub enabled: bool,
}

impl Default for NotificationPolicy {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserPreferences {
    pub user_id: UserId,
    pub time_preferences: TimePreferences,
    pub language: Language,
    pub snooze_policy: SnoozePolicy,
    pub notification_policy: NotificationPolicy,
}

impl UserPreferences {
    pub fn new(user_id: UserId) -> Self {
        Self {
            user_id,
            time_preferences: TimePreferences::default(),
            language: Language::Russian,
            snooze_policy: SnoozePolicy::default(),
            notification_policy: NotificationPolicy::default(),
        }
    }
}
