//! Application layer.
//!
//! Use cases, ports, and command/query DTOs. This crate depends on `domain`, but
//! does not know about MongoDB, Redis, VK, Telegram, HTTP clients, or axum.

pub mod error;
pub mod ports;
pub mod use_cases;

pub use error::{ApplicationError, ApplicationResult};
pub use ports::{
    ChannelSubscriptionRepository, Clock, DialogState, DialogStateStore, IdGenerator,
    NaturalLanguageReminderParser, Notification, Notifier, PaymentCachePort, PaymentGatewayPort,
    PaymentTransactionRepository, ProfileNotification, ReferralRepository, ReminderRepository,
    StreamPlatformGateway, SubscriptionRepository, UserRepository,
};
pub use use_cases::{
    EnsureSubscriptionUseCase, EnsureUserUseCase, GetProfileUseCase, ProfileView,
    SetAutoSnoozeUseCase, SetSnoozeButtonsUseCase, SetUserTimezoneUseCase,
};
