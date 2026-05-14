use chrono::Duration;

use crate::{TimePreferences, UserId};

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
    pub time_preferences: TimePreferences,
    pub snooze_buttons: Vec<SnoozeDuration>,
    pub auto_snooze: SnoozeDuration,
    pub payment_info: Option<String>,
}

impl User {
    pub fn new(id: UserId) -> Self {
        Self {
            id,
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
}
