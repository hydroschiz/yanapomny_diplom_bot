use async_trait::async_trait;
use chrono::{DateTime, Utc};
use domain::{
    ChannelSubscription, ChatId, PaymentId, PaymentTransaction, Referral, Reminder, ReminderId,
    Subscription, User, UserId,
};

use crate::ApplicationResult;

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn find_user(&self, id: UserId) -> ApplicationResult<Option<User>>;
    async fn save_user(&self, user: &User) -> ApplicationResult<()>;
}

#[async_trait]
pub trait SubscriptionRepository: Send + Sync {
    async fn find_subscription(&self, chat_id: ChatId) -> ApplicationResult<Option<Subscription>>;
    async fn save_subscription(&self, subscription: &Subscription) -> ApplicationResult<()>;
}

#[async_trait]
pub trait ReminderRepository: Send + Sync {
    async fn find_reminder(&self, id: ReminderId) -> ApplicationResult<Option<Reminder>>;
    async fn save_reminder(&self, reminder: &Reminder) -> ApplicationResult<()>;
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
pub trait ReferralRepository: Send + Sync {
    async fn find_referral(
        &self,
        referrer_id: UserId,
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
    async fn save_payment_transaction(
        &self,
        transaction: &PaymentTransaction,
    ) -> ApplicationResult<()>;
}

#[async_trait]
pub trait PaymentCachePort: Send + Sync {
    async fn remember_pending_payment(
        &self,
        payment_id: &PaymentId,
        user_id: UserId,
        expires_at: DateTime<Utc>,
    ) -> ApplicationResult<()>;
}

#[async_trait]
pub trait PaymentGatewayPort: Send + Sync {
    async fn create_payment(&self, transaction: &PaymentTransaction) -> ApplicationResult<String>;
}

#[async_trait]
pub trait NaturalLanguageReminderParser: Send + Sync {
    async fn parse_reminder(&self, text: &str, user: &User) -> ApplicationResult<domain::Schedule>;
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
}

#[async_trait]
pub trait DialogStateStore: Send + Sync {
    async fn get_state(&self, user_id: UserId) -> ApplicationResult<DialogState>;
    async fn set_state(&self, user_id: UserId, state: DialogState) -> ApplicationResult<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notification {
    Profile(ProfileNotification),
    Text { chat_id: ChatId, text: String },
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
