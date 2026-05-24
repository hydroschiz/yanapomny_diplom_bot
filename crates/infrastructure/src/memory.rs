use std::{collections::HashMap, sync::Mutex};

use application::{
    ApplicationError, ApplicationResult, ChannelSubscriptionRepository, DialogState,
    DialogStateStore, ExternalChannelSubscriptionRepository, NaturalLanguageReminderParser,
    PaymentCachePort, PaymentGatewayPort, PaymentTransactionRepository, PendingPayment,
    ReferralRepository, ReminderPreferencesRepository, ReminderRepository,
    SchedulerDeduplicationPort, StreamPlatformGateway, SubscriptionMaintenanceRepository,
    SubscriptionRepository, UserRepository,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use domain::{
    ChannelSubscription, ChatId, PaymentId, PaymentStatus, PaymentTransaction, Referral, Reminder,
    ReminderId, Schedule, Subscription, TimePreferences, User, UserId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InMemoryPendingPayment {
    pub payment_id: PaymentId,
    pub user_id: UserId,
    pub months: Option<domain::Months>,
    pub confirmation_url: String,
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
    pending_payments_by_user: HashMap<UserId, PaymentId>,
    payment_notifications: HashMap<(PaymentId, String), DateTime<Utc>>,
    payment_locks: HashMap<PaymentId, DateTime<Utc>>,
    scheduler_once: HashMap<String, DateTime<Utc>>,
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
impl SubscriptionMaintenanceRepository for InMemoryStore {
    async fn list_expiring_subscriptions(
        &self,
        from: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> ApplicationResult<Vec<Subscription>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .subscriptions
            .values()
            .filter(|subscription| {
                subscription.active
                    && subscription.expires_at > from
                    && subscription.expires_at <= until
            })
            .cloned()
            .collect())
    }

    async fn list_expired_active_subscriptions(
        &self,
        now: DateTime<Utc>,
    ) -> ApplicationResult<Vec<Subscription>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .subscriptions
            .values()
            .filter(|subscription| subscription.active && subscription.expires_at <= now)
            .cloned()
            .collect())
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

    async fn find_snooze_buttons_for_chat(
        &self,
        chat_id: ChatId,
    ) -> ApplicationResult<Vec<domain::SnoozeDuration>> {
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
            .map(|user| user.snooze_buttons.clone())
            .unwrap_or_else(|| User::new(UserId::new(chat_id.value())).snooze_buttons))
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
impl ExternalChannelSubscriptionRepository for InMemoryStore {
    async fn list_external_channel_subscriptions(
        &self,
        user_id: UserId,
    ) -> ApplicationResult<Vec<ChannelSubscription>> {
        self.list_channel_subscriptions(user_id).await
    }

    async fn list_all_external_channel_subscriptions(
        &self,
    ) -> ApplicationResult<Vec<ChannelSubscription>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .channel_subscriptions
            .values()
            .flat_map(|subscriptions| subscriptions.iter().cloned())
            .collect())
    }

    async fn save_external_channel_subscription(
        &self,
        subscription: &ChannelSubscription,
    ) -> ApplicationResult<()> {
        self.save_channel_subscription(subscription).await
    }

    async fn delete_external_channel_subscription(
        &self,
        subscription: &ChannelSubscription,
    ) -> ApplicationResult<()> {
        let mut state = self.state.lock().unwrap();
        if let Some(subscriptions) = state.channel_subscriptions.get_mut(&subscription.user_id) {
            subscriptions.retain(|existing| {
                existing.platform != subscription.platform
                    || existing.channel_id != subscription.channel_id
            });
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

    async fn find_referral_by_invited(
        &self,
        invited_id: UserId,
    ) -> ApplicationResult<Option<Referral>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .referrals
            .values()
            .find(|referral| referral.invited_id == invited_id)
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

    async fn find_payment_transaction_by_provider_payment_id(
        &self,
        provider_payment_id: &str,
    ) -> ApplicationResult<Option<PaymentTransaction>> {
        Ok(self
            .state
            .lock()
            .unwrap()
            .payment_transactions
            .values()
            .find(|transaction| {
                transaction.provider_payment_id.as_deref() == Some(provider_payment_id)
            })
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
    async fn pending_payment_for_user(
        &self,
        user_id: UserId,
    ) -> ApplicationResult<Option<PendingPayment>> {
        let state = self.state.lock().unwrap();
        let Some(payment_id) = state.pending_payments_by_user.get(&user_id) else {
            return Ok(None);
        };
        Ok(state.pending_payments.get(payment_id).map(|payment| {
            PendingPayment::new(
                payment.payment_id.clone(),
                payment.user_id,
                payment.months,
                payment.confirmation_url.clone(),
                payment.expires_at,
            )
        }))
    }

    async fn remember_pending_payment(&self, payment: &PendingPayment) -> ApplicationResult<()> {
        let mut state = self.state.lock().unwrap();
        state
            .pending_payments_by_user
            .insert(payment.user_id, payment.payment_id.clone());
        state.pending_payments.insert(
            payment.payment_id.clone(),
            InMemoryPendingPayment {
                payment_id: payment.payment_id.clone(),
                user_id: payment.user_id,
                months: payment.months,
                confirmation_url: payment.confirmation_url.clone(),
                expires_at: payment.expires_at,
            },
        );
        Ok(())
    }

    async fn refresh_pending_payment(
        &self,
        payment: &PendingPayment,
        expires_at: DateTime<Utc>,
    ) -> ApplicationResult<()> {
        let mut payment = payment.clone();
        payment.expires_at = expires_at;
        self.remember_pending_payment(&payment).await
    }

    async fn delete_pending_payment(&self, payment_id: &PaymentId) -> ApplicationResult<()> {
        let mut state = self.state.lock().unwrap();
        if let Some(payment) = state.pending_payments.remove(payment_id) {
            state.pending_payments_by_user.remove(&payment.user_id);
        }
        Ok(())
    }

    async fn notify_once(
        &self,
        payment_id: &PaymentId,
        event: &str,
        expires_at: DateTime<Utc>,
    ) -> ApplicationResult<bool> {
        let mut state = self.state.lock().unwrap();
        let key = (payment_id.clone(), event.to_string());
        if state.payment_notifications.contains_key(&key) {
            return Ok(false);
        }
        state.payment_notifications.insert(key, expires_at);
        Ok(true)
    }

    async fn try_acquire_fulfill_lock(
        &self,
        payment_id: &PaymentId,
        expires_at: DateTime<Utc>,
    ) -> ApplicationResult<bool> {
        let mut state = self.state.lock().unwrap();
        if state.payment_locks.contains_key(payment_id) {
            return Ok(false);
        }
        state.payment_locks.insert(payment_id.clone(), expires_at);
        Ok(true)
    }

    async fn release_fulfill_lock(&self, payment_id: &PaymentId) -> ApplicationResult<()> {
        self.state.lock().unwrap().payment_locks.remove(payment_id);
        Ok(())
    }
}

#[async_trait]
impl SchedulerDeduplicationPort for InMemoryStore {
    async fn once(&self, key: &str, expires_at: DateTime<Utc>) -> ApplicationResult<bool> {
        let mut state = self.state.lock().unwrap();
        if state.scheduler_once.contains_key(key) {
            return Ok(false);
        }
        state.scheduler_once.insert(key.to_string(), expires_at);
        Ok(true)
    }
}

#[async_trait]
impl PaymentGatewayPort for InMemoryStore {
    async fn create_payment(
        &self,
        transaction: &PaymentTransaction,
    ) -> ApplicationResult<(String, String)> {
        let mut state = self.state.lock().unwrap();
        let url = state
            .payment_confirmation_urls
            .entry(transaction.payment_id.clone())
            .or_insert_with(|| format!("https://pay.example/{}", transaction.payment_id));
        Ok((transaction.payment_id.as_str().to_string(), url.clone()))
    }

    async fn get_payment_status(&self, payment_id: &PaymentId) -> ApplicationResult<PaymentStatus> {
        let _ = payment_id;
        Ok(PaymentStatus::Pending)
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
