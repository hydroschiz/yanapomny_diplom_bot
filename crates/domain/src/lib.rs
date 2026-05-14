//! Pure domain model and business rules.
//!
//! This crate intentionally avoids transport, database, HTTP, serialization, and
//! async runtime dependencies. Infrastructure crates are responsible for mapping
//! these types to BSON, JSON, VK, Telegram, or external API DTOs.

pub mod channels;
pub mod error;
pub mod ids;
pub mod payments;
pub mod referral;
pub mod reminder;
pub mod scheduling;
pub mod subscription;
pub mod time;
pub mod user;

pub use channels::{ChannelSubscription, Platform};
pub use error::DomainError;
pub use ids::{ChatId, Months, PaymentId, ReminderId, UserId};
pub use payments::{Currency, Money, PaymentStatus, PaymentTransaction, Tariff, TARIFFS};
pub use referral::Referral;
pub use reminder::{Reminder, ReminderStatus, RetryPolicy};
pub use scheduling::{
    DayPosition, IntervalUnit, OffsetDirection, RecurrenceFilter, RecurrencePattern,
    RecurrenceRule, Schedule, TimeOfDay, TimeSpec, TimeSpecType, Weekday,
};
pub use subscription::{FreeState, Subscription, SubscriptionPolicy, SubscriptionStatus};
pub use time::{TimePreferences, TimeZone, UtcOffset};
pub use user::{SnoozeDuration, User};
