use async_trait::async_trait;
use chrono::{DateTime, Utc};
use domain::{
    ChannelSubscription, ChatId, DeliveryEvent, ExternalChannelSubscription, Months, Payment,
    PaymentId, PaymentStatus, PaymentTransaction, Referral, Reminder, ReminderId, Schedule,
    SnoozeDuration, Subscription, Task, TaskId, TimePreferences, User, UserId, UserPreferences,
};

use crate::ApplicationResult;

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn find_user(&self, id: UserId) -> ApplicationResult<Option<User>>;
    async fn save_user(&self, user: &User) -> ApplicationResult<()>;
}

#[async_trait]
pub trait UserPreferencesRepository: Send + Sync {
    async fn find_preferences(&self, user_id: UserId)
        -> ApplicationResult<Option<UserPreferences>>;
    async fn save_preferences(&self, preferences: &UserPreferences) -> ApplicationResult<()>;
}

#[async_trait]
pub trait TaskRepository: Send + Sync {
    async fn create_task(&self, task: Task) -> ApplicationResult<Task>;
    async fn find_task(&self, id: TaskId) -> ApplicationResult<Option<Task>>;
    async fn list_tasks(&self, user_id: UserId) -> ApplicationResult<Vec<Task>>;
    async fn save_task(&self, task: &Task) -> ApplicationResult<()>;
}

#[async_trait]
pub trait SubscriptionRepository: Send + Sync {
    async fn find_subscription(&self, chat_id: ChatId) -> ApplicationResult<Option<Subscription>>;
    async fn save_subscription(&self, subscription: &Subscription) -> ApplicationResult<()>;
}

#[async_trait]
pub trait SubscriptionMaintenanceRepository: Send + Sync {
    async fn list_expiring_subscriptions(
        &self,
        from: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> ApplicationResult<Vec<Subscription>>;

    async fn list_expired_active_subscriptions(
        &self,
        now: DateTime<Utc>,
    ) -> ApplicationResult<Vec<Subscription>>;
}

#[async_trait]
pub trait ReminderRepository: Send + Sync {
    async fn create_reminder(&self, reminder: Reminder) -> ApplicationResult<Reminder> {
        self.save_reminder(&reminder).await?;
        Ok(reminder)
    }

    async fn find_reminder(&self, id: ReminderId) -> ApplicationResult<Option<Reminder>>;
    async fn save_reminder(&self, reminder: &Reminder) -> ApplicationResult<()>;
    async fn list_reminders(&self, _chat_id: ChatId) -> ApplicationResult<Vec<Reminder>> {
        Ok(Vec::new())
    }

    async fn claim_due_reminders(
        &self,
        _now: DateTime<Utc>,
        _batch_size: usize,
    ) -> ApplicationResult<Vec<Reminder>> {
        Ok(Vec::new())
    }
}

#[async_trait]
pub trait DeliveryEventRepository: Send + Sync {
    async fn create_delivery_event(&self, event: DeliveryEvent)
        -> ApplicationResult<DeliveryEvent>;
    async fn save_delivery_event(&self, event: &DeliveryEvent) -> ApplicationResult<()>;
    async fn list_delivery_events(
        &self,
        reminder_id: ReminderId,
    ) -> ApplicationResult<Vec<DeliveryEvent>>;
}

#[async_trait]
pub trait ReminderPreferencesRepository: Send + Sync {
    async fn find_time_preferences_for_chat(
        &self,
        chat_id: ChatId,
    ) -> ApplicationResult<TimePreferences>;

    async fn find_snooze_buttons_for_chat(
        &self,
        _chat_id: ChatId,
    ) -> ApplicationResult<Vec<SnoozeDuration>> {
        Ok(vec![
            SnoozeDuration::ONE_HOUR,
            SnoozeDuration::THREE_HOURS,
            SnoozeDuration::ONE_DAY,
        ])
    }
}

#[async_trait]
pub trait ChannelSubscriptionRepository: Send + Sync {
    async fn list_channel_subscriptions(
        &self,
        user_id: UserId,
    ) -> ApplicationResult<Vec<ChannelSubscription>>;
    async fn save_channel_subscription(
        &self,
        subscription: &ChannelSubscription,
    ) -> ApplicationResult<()>;
}

#[async_trait]
pub trait ExternalChannelSubscriptionRepository: Send + Sync {
    async fn list_external_channel_subscriptions(
        &self,
        user_id: UserId,
    ) -> ApplicationResult<Vec<ExternalChannelSubscription>>;
    async fn list_all_external_channel_subscriptions(
        &self,
    ) -> ApplicationResult<Vec<ExternalChannelSubscription>>;
    async fn save_external_channel_subscription(
        &self,
        subscription: &ExternalChannelSubscription,
    ) -> ApplicationResult<()>;
    async fn delete_external_channel_subscription(
        &self,
        subscription: &ExternalChannelSubscription,
    ) -> ApplicationResult<()>;
}

#[async_trait]
pub trait ReferralRepository: Send + Sync {
    async fn find_referral(
        &self,
        referrer_id: UserId,
        invited_id: UserId,
    ) -> ApplicationResult<Option<Referral>>;
    async fn find_referral_by_invited(
        &self,
        invited_id: UserId,
    ) -> ApplicationResult<Option<Referral>>;
    async fn save_referral(&self, referral: &Referral) -> ApplicationResult<()>;
}

#[async_trait]
pub trait PaymentTransactionRepository: Send + Sync {
    async fn find_payment_transaction(
        &self,
        payment_id: &PaymentId,
    ) -> ApplicationResult<Option<PaymentTransaction>>;
    async fn find_payment_transaction_by_provider_payment_id(
        &self,
        provider_payment_id: &str,
    ) -> ApplicationResult<Option<PaymentTransaction>>;
    async fn save_payment_transaction(
        &self,
        transaction: &PaymentTransaction,
    ) -> ApplicationResult<()>;
}

#[async_trait]
pub trait PaymentRepository: Send + Sync {
    async fn find_payment(&self, payment_id: &PaymentId) -> ApplicationResult<Option<Payment>>;
    async fn save_payment(&self, payment: &Payment) -> ApplicationResult<()>;
}

#[async_trait]
pub trait PaymentCachePort: Send + Sync {
    async fn pending_payment_for_user(
        &self,
        user_id: UserId,
    ) -> ApplicationResult<Option<PendingPayment>>;
    async fn remember_pending_payment(&self, payment: &PendingPayment) -> ApplicationResult<()>;
    async fn refresh_pending_payment(
        &self,
        payment: &PendingPayment,
        expires_at: DateTime<Utc>,
    ) -> ApplicationResult<()>;
    async fn delete_pending_payment(&self, payment_id: &PaymentId) -> ApplicationResult<()>;
    async fn notify_once(
        &self,
        payment_id: &PaymentId,
        event: &str,
        expires_at: DateTime<Utc>,
    ) -> ApplicationResult<bool>;
    async fn try_acquire_fulfill_lock(
        &self,
        payment_id: &PaymentId,
        expires_at: DateTime<Utc>,
    ) -> ApplicationResult<bool>;
    async fn release_fulfill_lock(&self, payment_id: &PaymentId) -> ApplicationResult<()>;
}

#[async_trait]
pub trait SchedulerDeduplicationPort: Send + Sync {
    async fn once(&self, key: &str, expires_at: DateTime<Utc>) -> ApplicationResult<bool>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPayment {
    pub payment_id: PaymentId,
    pub user_id: UserId,
    pub months: Option<Months>,
    pub confirmation_url: String,
    pub expires_at: DateTime<Utc>,
}

impl PendingPayment {
    pub fn new(
        payment_id: PaymentId,
        user_id: UserId,
        months: Option<Months>,
        confirmation_url: impl Into<String>,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            payment_id,
            user_id,
            months,
            confirmation_url: confirmation_url.into(),
            expires_at,
        }
    }
}

#[async_trait]
pub trait PaymentGatewayPort: Send + Sync {
    async fn create_payment(
        &self,
        transaction: &PaymentTransaction,
    ) -> ApplicationResult<(String, String)>;
    async fn get_payment_status(&self, payment_id: &PaymentId) -> ApplicationResult<PaymentStatus>;
}

#[async_trait]
pub trait NaturalLanguageReminderParser: Send + Sync {
    async fn parse_reminder(&self, text: &str, user: &User) -> ApplicationResult<domain::Schedule>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterpretedTask {
    pub title: String,
    pub description: Option<String>,
    pub schedule: Schedule,
    pub trigger_at: DateTime<Utc>,
}

#[async_trait]
pub trait NaturalLanguageInterpreter: Send + Sync {
    async fn interpret_task(&self, text: &str, user: &User) -> ApplicationResult<InterpretedTask>;
}

#[async_trait]
pub trait StreamPlatformGateway: Send + Sync {
    async fn latest_content_id(
        &self,
        subscription: &ChannelSubscription,
    ) -> ApplicationResult<Option<String>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogState {
    Idle,
    AwaitingUtc,
    AwaitingSnoozeButtons,
    AwaitingAutoSnooze,
    AwaitingTextConfirmation {
        text: String,
    },
    AwaitingReminderConfirmation {
        original_text: String,
        interpreted: InterpretedTask,
    },
    AwaitingReminderEdit {
        original_text: String,
        interpreted: InterpretedTask,
    },
    AwaitingExistingReminderEditSelection,
    AwaitingExistingReminderText {
        reminder_id: ReminderId,
    },
    AwaitingReminderDeletion,
    AwaitingChannelSubscriptionDeletion,
}

#[async_trait]
pub trait DialogStateStore: Send + Sync {
    async fn get_state(&self, user_id: UserId) -> ApplicationResult<DialogState>;
    async fn set_state(&self, user_id: UserId, state: DialogState) -> ApplicationResult<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notification {
    Profile(ProfileNotification),
    Text {
        chat_id: ChatId,
        text: String,
    },
    ReminderDue {
        chat_id: ChatId,
        reminder_id: ReminderId,
        text: String,
        snooze_buttons: Vec<SnoozeDuration>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileNotification {
    pub chat_id: ChatId,
    pub user_id: UserId,
}

#[async_trait]
pub trait Notifier: Send + Sync {
    async fn notify(&self, notification: Notification) -> ApplicationResult<()>;
}

pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub trait IdGenerator: Send + Sync {
    fn new_payment_id(&self) -> PaymentId;
}
