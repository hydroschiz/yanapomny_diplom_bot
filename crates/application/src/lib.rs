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
    PaymentCachePort, PaymentGatewayPort, PaymentRepository, PaymentTransactionRepository,
    PendingPayment, ProfileNotification, ReferralRepository, ReminderPreferencesRepository,
    ReminderRepository, SchedulerDeduplicationPort, StreamPlatformGateway,
    SubscriptionMaintenanceRepository, SubscriptionRepository, TaskRepository,
    UserPreferencesRepository, UserRepository,
};
pub use use_cases::{
    active_reminders, active_tasks, CancelReminderUseCase, CheckAllTwitchStreamsUseCase,
    CheckSubscriptionPaymentUseCase, CheckTwitchStreamsUseCase, CompleteReminderUseCase,
    CompleteTaskUseCase, ConsumeReferralRewardUseCase, CreatePaymentCommand, CreatePaymentUseCase,
    CreateReferralUseCase, CreateReminderCommand, CreateReminderFromPreviewCommand,
    CreateReminderFromPreviewUseCase, CreateReminderFromTextCommand, CreateReminderFromTextUseCase,
    CreateReminderUseCase, CreateSubscriptionPaymentCommand, CreateSubscriptionPaymentUseCase,
    CreateTaskCommand, CreateTaskFromTextUseCase, CreateTaskUseCase, CreatedPayment,
    CreatedReminderFromText, CreatedSubscriptionPayment, DeleteExternalChannelSubscriptionCommand,
    DeleteExternalChannelSubscriptionUseCase, DeleteTaskUseCase, DeliverDueRemindersUseCase,
    DeliveryReport, EnsureSubscriptionUseCase, EnsureUserUseCase, GetProfileUseCase,
    ListActiveRemindersUseCase, ListExternalChannelSubscriptionsUseCase, ListTasksUseCase,
    PreviewReminderFromTextCommand, PreviewReminderFromTextUseCase, PreviewedReminderFromText,
    ProcessSubscriptionPaymentWebhookUseCase, ProcessYooKassaWebhookUseCase, ProfileView,
    PurgeExpiredSubscriptionsUseCase, ReminderActionCommand,
    SaveExternalChannelSubscriptionCommand, SaveExternalChannelSubscriptionUseCase,
    SetAutoSnoozeUseCase, SetSnoozeButtonsUseCase, SetUserTimezoneUseCase, SnoozeReminderUseCase,
    SubscriptionMaintenanceReport, SubscriptionPaymentStatus, UpdatePreferencesUseCase,
    WarnExpiringSubscriptionsUseCase,
};
