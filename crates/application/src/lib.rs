//! Application layer.
//!
//! Use cases, ports, and command/query DTOs. This crate depends on `domain`, but
//! does not know about MongoDB, Redis, VK, Telegram, HTTP clients, or axum.

pub mod error;
pub mod ports;
pub mod use_cases;

pub use error::{ApplicationError, ApplicationResult};
pub use ports::{
    ChannelSubscriptionRepository, Clock, DeliveryEventRepository, DialogState, DialogStateStore,
    ExternalChannelSubscriptionRepository, IdGenerator, InterpretedTask,
    NaturalLanguageInterpreter, NaturalLanguageReminderParser, Notification, Notifier,
    PaymentCachePort, PaymentGateway, PaymentGatewayPort, PaymentRepository,
    PaymentTransactionRepository, ProfileNotification, ReferralRepository,
    ReminderPreferencesRepository, ReminderRepository, StreamPlatformGateway,
    SubscriptionRepository, TaskRepository, UserPreferencesRepository, UserRepository,
};
pub use use_cases::{
    active_reminders, active_tasks, CancelReminderUseCase, CheckTwitchStreamsUseCase,
    CompleteReminderUseCase, CompleteTaskUseCase, ConsumeReferralRewardUseCase,
    CreatePaymentCommand, CreatePaymentUseCase, CreateReferralUseCase, CreateReminderCommand,
    CreateReminderFromTextCommand, CreateReminderFromTextUseCase, CreateReminderUseCase,
    CreateTaskCommand, CreateTaskFromTextUseCase, CreateTaskUseCase, CreatedPayment,
    CreatedReminderFromText, DeleteTaskUseCase, DeliverDueRemindersUseCase, DeliveryReport,
    EnsureSubscriptionUseCase, EnsureUserUseCase, GetProfileUseCase, ListActiveRemindersUseCase,
    ListTasksUseCase, ProcessYooKassaWebhookUseCase, ProfileView, ReminderActionCommand,
    SaveExternalChannelSubscriptionCommand, SaveExternalChannelSubscriptionUseCase,
    SetAutoSnoozeUseCase, SetSnoozeButtonsUseCase, SetUserTimezoneUseCase, SnoozeReminderUseCase,
    UpdatePreferencesUseCase,
};
