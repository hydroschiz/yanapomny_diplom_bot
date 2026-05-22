use std::{collections::HashMap, sync::Mutex};

use application::{
    ApplicationError, ApplicationResult, ChannelSubscriptionRepository, DialogState,
    DialogStateStore, NaturalLanguageReminderParser, PaymentCachePort, PaymentGatewayPort,
    PaymentTransactionRepository, ReferralRepository, ReminderPreferencesRepository,
    ReminderRepository, StreamPlatformGateway, SubscriptionRepository, UserRepository,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use domain::{
    ChannelSubscription, ChatId, PaymentId, PaymentTransaction, Referral, Reminder, ReminderId,
    Schedule, Subscription, TimePreferences, User, UserId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InMemoryPendingPayment {
    pub payment_id: PaymentId,
    pub user_id: UserId,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
struct InMemoryState {
    users: HashMap<UserId, User>,
    subscriptions: HashMap<ChatId, Subscription>,
    reminders: HashMap<ReminderId, Reminder>,
    channel_subscriptions: HashMap<UserId, Vec<ChannelSubscription>>,
    referrals: HashMap<(UserId, UserId), Referral>,
    payment_transactions: HashMap<PaymentId, PaymentTransaction>,
    pending_payments: HashMap<PaymentId, InMemoryPendingPayment>,
    dialog_states: HashMap<UserId, DialogState>,
    parsed_schedules: HashMap<(UserId, String), Schedule>,
    latest_content: HashMap<String, Option<String>>,
    payment_confirmation_urls: HashMap<PaymentId, String>,
}

#[derive(Debug, Default)]
pub struct InMemoryStore {
    state: Mutex<InMemoryState>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn user_count(&self) -> usize {
        self.state.lock().unwrap().users.len()
    }

    pub fn subscription_count(&self) -> usize {
        self.state.lock().unwrap().subscriptions.len()
    }

    pub fn pending_payment(&self, payment_id: &PaymentId) -> Option<InMemoryPendingPayment> {
        self.state
            .lock()
            .unwrap()
            .pending_payments
            .get(payment_id)
            .cloned()
    }

    pub fn set_parsed_schedule(
        &self,
        user_id: UserId,
        text: impl Into<String>,
        schedule: Schedule,
    ) {
        self.state
            .lock()
            .unwrap()
            .parsed_schedules
            .insert((user_id, text.into()), schedule);
    }

    pub fn set_latest_content(&self, channel_id: impl Into<String>, content_id: Option<String>) {
        self.state
            .lock()
            .unwrap()
            .latest_content
            .insert(channel_id.into(), content_id);
    }

    pub fn set_payment_confirmation_url(&self, payment_id: PaymentId, url: impl Into<String>) {
        self.state
            .lock()
            .unwrap()
            .payment_confirmation_urls
            .insert(payment_id, url.into());
    }
}

#[async_trait]
impl UserRepository for InMemoryStore {
    async fn find_user(&self, id: UserId) -> ApplicationResult<Option<User>> {
        Ok(self.state.lock().unwrap().users.get(&id).cloned())
    }

    async fn save_user(&self, user: &User) -> ApplicationResult<()> {
        self.state
            .lock()
            .unwrap()
            .users
            .insert(user.id, user.clone());
        Ok(())
    }
}

#[async_trait]
impl SubscriptionRepository for InMemoryStore {
    async fn find_subscription(&self, chat_id: ChatId) -> ApplicationResult<Option<Subscription>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .subscriptions
            .get(&chat_id)
            .cloned())
    }

    async fn save_subscription(&self, subscription: &Subscription) -> ApplicationResult<()> {
        self.state
            .lock()
            .unwrap()
            .subscriptions
            .insert(subscription.chat_id, subscription.clone());
        Ok(())
    }
}

#[async_trait]
impl ReminderRepository for InMemoryStore {
    async fn find_reminder(&self, id: ReminderId) -> ApplicationResult<Option<Reminder>> {
        Ok(self.state.lock().unwrap().reminders.get(&id).cloned())
    }

    async fn save_reminder(&self, reminder: &Reminder) -> ApplicationResult<()> {
        let id = reminder.id.ok_or_else(|| {
            ApplicationError::Repository("reminder id is required for InMemoryStore".to_string())
        })?;

        self.state
            .lock()
            .unwrap()
            .reminders
            .insert(id, reminder.clone());
        Ok(())
    }

    async fn list_reminders(&self, chat_id: ChatId) -> ApplicationResult<Vec<Reminder>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .reminders
            .values()
            .filter(|reminder| reminder.chat_id == chat_id)
            .cloned()
            .collect())
    }
}

#[async_trait]
impl ReminderPreferencesRepository for InMemoryStore {
    async fn find_time_preferences_for_chat(
        &self,
        chat_id: ChatId,
    ) -> ApplicationResult<TimePreferences> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .users
            .values()
            .find(|user| {
                user.id.value() == chat_id.value()
                    || user
                        .identities
                        .iter()
                        .any(|identity| identity.chat_id == Some(chat_id))
            })
            .map(|user| user.time_preferences.clone())
            .unwrap_or_default())
    }
}

#[async_trait]
impl ChannelSubscriptionRepository for InMemoryStore {
    async fn list_channel_subscriptions(
        &self,
        user_id: UserId,
    ) -> ApplicationResult<Vec<ChannelSubscription>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .channel_subscriptions
            .get(&user_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn save_channel_subscription(
        &self,
        subscription: &ChannelSubscription,
    ) -> ApplicationResult<()> {
        let mut state = self.state.lock().unwrap();
        let subscriptions = state
            .channel_subscriptions
            .entry(subscription.user_id)
            .or_default();

        if let Some(existing) = subscriptions
            .iter_mut()
            .find(|existing| existing.sub_num == subscription.sub_num)
        {
            *existing = subscription.clone();
        } else {
            subscriptions.push(subscription.clone());
        }

        Ok(())
    }
}

#[async_trait]
impl ReferralRepository for InMemoryStore {
    async fn find_referral(
        &self,
        referrer_id: UserId,
        invited_id: UserId,
    ) -> ApplicationResult<Option<Referral>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .referrals
            .get(&(referrer_id, invited_id))
            .cloned())
    }

    async fn save_referral(&self, referral: &Referral) -> ApplicationResult<()> {
        self.state.lock().unwrap().referrals.insert(
            (referral.referrer_id, referral.invited_id),
            referral.clone(),
        );
        Ok(())
    }
}

#[async_trait]
impl PaymentTransactionRepository for InMemoryStore {
    async fn find_payment_transaction(
        &self,
        payment_id: &PaymentId,
    ) -> ApplicationResult<Option<PaymentTransaction>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .payment_transactions
            .get(payment_id)
            .cloned())
    }

    async fn save_payment_transaction(
        &self,
        transaction: &PaymentTransaction,
    ) -> ApplicationResult<()> {
        self.state
            .lock()
            .unwrap()
            .payment_transactions
            .insert(transaction.payment_id.clone(), transaction.clone());
        Ok(())
    }
}

#[async_trait]
impl PaymentCachePort for InMemoryStore {
    async fn remember_pending_payment(
        &self,
        payment_id: &PaymentId,
        user_id: UserId,
        expires_at: DateTime<Utc>,
    ) -> ApplicationResult<()> {
        self.state.lock().unwrap().pending_payments.insert(
            payment_id.clone(),
            InMemoryPendingPayment {
                payment_id: payment_id.clone(),
                user_id,
                expires_at,
            },
        );
        Ok(())
    }
}

#[async_trait]
impl PaymentGatewayPort for InMemoryStore {
    async fn create_payment(&self, transaction: &PaymentTransaction) -> ApplicationResult<String> {
        let mut state = self.state.lock().unwrap();
        let url = state
            .payment_confirmation_urls
            .entry(transaction.payment_id.clone())
            .or_insert_with(|| format!("https://pay.example/{}", transaction.payment_id));
        Ok(url.clone())
    }
}

#[async_trait]
impl NaturalLanguageReminderParser for InMemoryStore {
    async fn parse_reminder(&self, text: &str, user: &User) -> ApplicationResult<Schedule> {
        self.state
            .lock()
            .unwrap()
            .parsed_schedules
            .get(&(user.id, text.to_string()))
            .cloned()
            .ok_or_else(|| ApplicationError::ExternalService("missing parsed schedule".to_string()))
    }
}

#[async_trait]
impl StreamPlatformGateway for InMemoryStore {
    async fn latest_content_id(
        &self,
        subscription: &ChannelSubscription,
    ) -> ApplicationResult<Option<String>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .latest_content
            .get(&subscription.channel_id)
            .cloned()
            .unwrap_or_default())
    }
}

#[async_trait]
impl DialogStateStore for InMemoryStore {
    async fn get_state(&self, user_id: UserId) -> ApplicationResult<DialogState> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .dialog_states
            .get(&user_id)
            .cloned()
            .unwrap_or(DialogState::Idle))
    }

    async fn set_state(&self, user_id: UserId, state: DialogState) -> ApplicationResult<()> {
        self.state
            .lock()
            .unwrap()
            .dialog_states
            .insert(user_id, state);
        Ok(())
    }
}
