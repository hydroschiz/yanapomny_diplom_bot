//! Pure domain model and business rules.
//!
//! This crate intentionally avoids transport, database, HTTP, serialization, and
//! async runtime dependencies. Infrastructure crates are responsible for mapping
//! these types to BSON, JSON, VK, Telegram, or external API DTOs.

pub mod channels;
pub mod delivery;
pub mod error;
pub mod identity;
pub mod ids;
pub mod intent;
pub mod payments;
pub mod referral;
pub mod reminder;
pub mod scheduling;
pub mod subscription;
pub mod task;
pub mod time;
pub mod user;

pub use channels::{ChannelSubscription, ExternalChannelSubscription, Platform};
pub use delivery::{DeliveryChannel, DeliveryEvent, DeliveryResult};
pub use error::DomainError;
pub use identity::{CommunicationPlatform, PlatformIdentity};
pub use ids::{
    ChatId, DeliveryEventId, ExternalChannelSubscriptionId, IntentLogId, Months, PaymentId,
    ReminderId, SubscriptionId, TaskId, UserId,
};
pub use intent::IntentLog;
pub use payments::{
    Currency, Money, Payment, PaymentProvider, PaymentStatus, PaymentTransaction, Tariff, TARIFFS,
};
pub use referral::Referral;
pub use reminder::{Reminder, ReminderStatus, RetryPolicy};
pub use scheduling::{
    DayPosition, IntervalUnit, OffsetDirection, RecurrenceFilter, RecurrencePattern,
    RecurrenceRule, Schedule, TimeOfDay, TimeSpec, TimeSpecType, Weekday,
};
pub use subscription::{
    FreeState, Subscription, SubscriptionPlan, SubscriptionPolicy, SubscriptionSource,
    SubscriptionStatus,
};
pub use task::{Task, TaskPriority, TaskStatus};
pub use time::{TimePreferences, TimeZone, UtcOffset};
pub use user::{
    Language, NotificationPolicy, SnoozeDuration, SnoozePolicy, User, UserPreferences, UserStatus,
};
