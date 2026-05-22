use application::{
    ApplicationError, ApplicationResult, ChannelSubscriptionRepository, DeliveryEventRepository,
    ExternalChannelSubscriptionRepository, PaymentRepository, PaymentTransactionRepository,
    ReferralRepository, ReminderRepository, SubscriptionRepository, TaskRepository,
    UserPreferencesRepository, UserRepository,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{
    bson::{
        self, doc,
        serde_helpers::{
            chrono_datetime_as_bson_datetime, chrono_datetime_as_bson_datetime_optional,
        },
        Bson,
    },
    options::{FindOneAndUpdateOptions, IndexOptions, ReturnDocument, UpdateOptions},
    Client, Collection, Database, IndexModel,
};
use serde::{Deserialize, Serialize};

use domain::{
    ChannelSubscription, ChatId, CommunicationPlatform, Currency, DayPosition, DeliveryChannel,
    DeliveryEvent, DeliveryEventId, DeliveryResult, ExternalChannelSubscription, FreeState,
    IntervalUnit, Language, Money, Months, NotificationPolicy, OffsetDirection, Payment, PaymentId,
    PaymentProvider, PaymentStatus, PaymentTransaction, Platform, PlatformIdentity,
    RecurrenceFilter, RecurrencePattern, RecurrenceRule, Referral, Reminder, ReminderId,
    ReminderStatus, Schedule, SnoozeDuration, SnoozePolicy, Subscription, SubscriptionId,
    SubscriptionPlan, SubscriptionPolicy, SubscriptionSource, Task, TaskId, TaskPriority,
    TaskStatus, TimeOfDay, TimePreferences, TimeSpec, TimeSpecType, TimeZone, User, UserId,
    UserPreferences, UserStatus, UtcOffset, Weekday,
};

const USERS_COLLECTION: &str = "users";
const TASKS_COLLECTION: &str = "tasks";
const REMINDERS_COLLECTION: &str = "reminders";
const DELIVERY_EVENTS_COLLECTION: &str = "delivery_events";
const SUBSCRIPTIONS_COLLECTION: &str = "subscriptions";
const PAYMENTS_COLLECTION: &str = "payments";
const REFERRALS_COLLECTION: &str = "referrals";
const EXTERNAL_CHANNEL_SUBSCRIPTIONS_COLLECTION: &str = "external_channel_subscriptions";

#[derive(Clone)]
pub struct MongoStore {
    db: Database,
}

impl MongoStore {
    pub async fn connect(uri: &str, db_name: &str) -> ApplicationResult<Self> {
        let client = Client::with_uri_str(uri).await.map_err(repo_err)?;
        let store = Self {
            db: client.database(db_name),
        };
        store.ensure_indexes().await?;
        Ok(store)
    }

    pub fn new(db: Database) -> Self {
        Self { db }
    }

    fn users(&self) -> Collection<UserDto> {
        self.db.collection(USERS_COLLECTION)
    }

    fn tasks(&self) -> Collection<TaskDto> {
        self.db.collection(TASKS_COLLECTION)
    }

    fn reminders(&self) -> Collection<ReminderDto> {
        self.db.collection(REMINDERS_COLLECTION)
    }

    fn delivery_events(&self) -> Collection<DeliveryEventDto> {
        self.db.collection(DELIVERY_EVENTS_COLLECTION)
    }

    fn subscriptions(&self) -> Collection<SubscriptionDto> {
        self.db.collection(SUBSCRIPTIONS_COLLECTION)
    }

    fn payments(&self) -> Collection<PaymentDto> {
        self.db.collection(PAYMENTS_COLLECTION)
    }

    fn referrals(&self) -> Collection<ReferralDto> {
        self.db.collection(REFERRALS_COLLECTION)
    }

    fn external_channel_subscriptions(&self) -> Collection<ExternalChannelSubscriptionDto> {
        self.db
            .collection(EXTERNAL_CHANNEL_SUBSCRIPTIONS_COLLECTION)
    }

    async fn ensure_indexes(&self) -> ApplicationResult<()> {
        self.users()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "identities.platform": 1, "identities.external_id": 1 })
                    .options(IndexOptions::builder().unique(true).sparse(true).build())
                    .build(),
                None,
            )
            .await
            .map_err(repo_err)?;
        self.users()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "identities.chat_id": 1 })
                    .options(IndexOptions::builder().sparse(true).build())
                    .build(),
                None,
            )
            .await
            .map_err(repo_err)?;

        self.tasks()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "user_id": 1, "status": 1 })
                    .build(),
                None,
            )
            .await
            .map_err(repo_err)?;
        self.tasks()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "due_at": 1, "status": 1 })
                    .build(),
                None,
            )
            .await
            .map_err(repo_err)?;

        self.reminders()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "status": 1, "next_at": 1 })
                    .build(),
                None,
            )
            .await
            .map_err(repo_err)?;
        self.reminders()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "status": 1, "retry_at": 1 })
                    .build(),
                None,
            )
            .await
            .map_err(repo_err)?;
        self.reminders()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "chat_id": 1, "status": 1 })
                    .build(),
                None,
            )
            .await
            .map_err(repo_err)?;

        self.delivery_events()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "reminder_id": 1, "planned_at": 1 })
                    .build(),
                None,
            )
            .await
            .map_err(repo_err)?;

        self.subscriptions()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "subject_type": 1, "subject_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
                None,
            )
            .await
            .map_err(repo_err)?;
        self.subscriptions()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "expires_at": 1, "active": 1 })
                    .build(),
                None,
            )
            .await
            .map_err(repo_err)?;

        self.payments()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "provider_payment_id": 1 })
                    .options(IndexOptions::builder().unique(true).sparse(true).build())
                    .build(),
                None,
            )
            .await
            .map_err(repo_err)?;
        self.payments()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "user_id": 1, "status": 1 })
                    .build(),
                None,
            )
            .await
            .map_err(repo_err)?;

        self.referrals()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "invited_user_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
                None,
            )
            .await
            .map_err(repo_err)?;
        self.referrals()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "referrer_user_id": 1, "invited_user_id": 1 })
                    .build(),
                None,
            )
            .await
            .map_err(repo_err)?;

        self.external_channel_subscriptions()
            .create_index(
                IndexModel::builder()
                    .keys(doc! {
                        "subject_type": 1,
                        "subject_id": 1,
                        "platform": 1,
                        "channel_id": 1,
                    })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
                None,
            )
            .await
            .map_err(repo_err)?;
        self.external_channel_subscriptions()
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "platform": 1, "channel_id": 1 })
                    .build(),
                None,
            )
            .await
            .map_err(repo_err)?;

        Ok(())
    }
}

#[async_trait]
impl UserRepository for MongoStore {
    async fn find_user(&self, id: UserId) -> ApplicationResult<Option<User>> {
        let dto = self
            .users()
            .find_one(doc! { "_id": id.value() }, None)
            .await
            .map_err(repo_err)?;
        dto.map(TryInto::try_into).transpose()
    }

    async fn save_user(&self, user: &User) -> ApplicationResult<()> {
        let dto = UserDto::from(user.clone());
        self.users()
            .replace_one(
                doc! { "_id": dto.id },
                dto,
                mongodb::options::ReplaceOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await
            .map_err(repo_err)?;
        Ok(())
    }
}

#[async_trait]
impl UserPreferencesRepository for MongoStore {
    async fn find_preferences(
        &self,
        user_id: UserId,
    ) -> ApplicationResult<Option<UserPreferences>> {
        let dto = self
            .users()
            .find_one(doc! { "_id": user_id.value() }, None)
            .await
            .map_err(repo_err)?;
        dto.map(|user| user.preferences.into_domain(user_id))
            .transpose()
    }

    async fn save_preferences(&self, preferences: &UserPreferences) -> ApplicationResult<()> {
        let dto = UserPreferencesDto::from(preferences.clone());
        let preferences_bson = bson::to_bson(&dto).map_err(repo_err)?;
        self.users()
            .update_one(
                doc! { "_id": preferences.user_id.value() },
                doc! {
                    "$set": { "preferences": preferences_bson },
                    "$setOnInsert": {
                        "status": "active",
                        "identities": Bson::Array(Vec::new()),
                    },
                },
                UpdateOptions::builder().upsert(true).build(),
            )
            .await
            .map_err(repo_err)?;
        Ok(())
    }
}

#[async_trait]
impl TaskRepository for MongoStore {
    async fn create_task(&self, mut task: Task) -> ApplicationResult<Task> {
        if task.id.is_none() {
            task.assign_id(TaskId::new(generated_i64_id()));
        }
        let dto = TaskDto::from(task.clone());
        self.tasks().insert_one(dto, None).await.map_err(repo_err)?;
        Ok(task)
    }

    async fn find_task(&self, id: TaskId) -> ApplicationResult<Option<Task>> {
        let dto = self
            .tasks()
            .find_one(doc! { "_id": id.value() }, None)
            .await
            .map_err(repo_err)?;
        dto.map(TryInto::try_into).transpose()
    }

    async fn list_tasks(&self, user_id: UserId) -> ApplicationResult<Vec<Task>> {
        self.tasks()
            .find(doc! { "user_id": user_id.value() }, None)
            .await
            .map_err(repo_err)?
            .map_err(repo_err)
            .and_then(|dto| futures::future::ready(dto.try_into()))
            .try_collect()
            .await
    }

    async fn save_task(&self, task: &Task) -> ApplicationResult<()> {
        let id = task.id.ok_or_else(|| repo_message("task id is required"))?;
        let dto = TaskDto::from(task.clone());
        self.tasks()
            .replace_one(
                doc! { "_id": id.value() },
                dto,
                mongodb::options::ReplaceOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await
            .map_err(repo_err)?;
        Ok(())
    }
}

#[async_trait]
impl ReminderRepository for MongoStore {
    async fn create_reminder(&self, mut reminder: Reminder) -> ApplicationResult<Reminder> {
        if reminder.id.is_none() {
            reminder.assign_id(ReminderId::new(generated_i32_id()));
        }
        let dto = ReminderDto::from(reminder.clone());
        self.reminders()
            .insert_one(dto, None)
            .await
            .map_err(repo_err)?;
        Ok(reminder)
    }

    async fn find_reminder(&self, id: ReminderId) -> ApplicationResult<Option<Reminder>> {
        let dto = self
            .reminders()
            .find_one(doc! { "_id": id.value() }, None)
            .await
            .map_err(repo_err)?;
        dto.map(TryInto::try_into).transpose()
    }

    async fn save_reminder(&self, reminder: &Reminder) -> ApplicationResult<()> {
        let id = reminder
            .id
            .ok_or_else(|| repo_message("reminder id is required"))?;
        let dto = ReminderDto::from(reminder.clone());
        self.reminders()
            .replace_one(
                doc! { "_id": id.value() },
                dto,
                mongodb::options::ReplaceOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await
            .map_err(repo_err)?;
        Ok(())
    }

    async fn claim_due_reminders(
        &self,
        now: DateTime<Utc>,
        batch_size: usize,
    ) -> ApplicationResult<Vec<Reminder>> {
        let now_bson = mongodb::bson::DateTime::from_chrono(now);
        let filter = doc! {
            "$or": [
                { "status": "active", "next_at": { "$lte": now_bson } },
                { "status": "retry", "retry_at": { "$lte": now_bson } }
            ]
        };
        let update = doc! { "$set": { "status": "processing" } };
        let options = FindOneAndUpdateOptions::builder()
            .return_document(ReturnDocument::After)
            .build();
        let mut claimed = Vec::new();

        for _ in 0..batch_size {
            let Some(dto) = self
                .reminders()
                .find_one_and_update(filter.clone(), update.clone(), options.clone())
                .await
                .map_err(repo_err)?
            else {
                break;
            };
            claimed.push(dto.try_into()?);
        }

        Ok(claimed)
    }
}

#[async_trait]
impl DeliveryEventRepository for MongoStore {
    async fn create_delivery_event(
        &self,
        mut event: DeliveryEvent,
    ) -> ApplicationResult<DeliveryEvent> {
        if event.id.is_none() {
            event.assign_id(DeliveryEventId::new(generated_i64_id()));
        }
        let dto = DeliveryEventDto::from(event.clone());
        self.delivery_events()
            .insert_one(dto, None)
            .await
            .map_err(repo_err)?;
        Ok(event)
    }

    async fn save_delivery_event(&self, event: &DeliveryEvent) -> ApplicationResult<()> {
        let id = event
            .id
            .ok_or_else(|| repo_message("delivery event id is required"))?;
        let dto = DeliveryEventDto::from(event.clone());
        self.delivery_events()
            .replace_one(
                doc! { "_id": id.value() },
                dto,
                mongodb::options::ReplaceOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await
            .map_err(repo_err)?;
        Ok(())
    }

    async fn list_delivery_events(
        &self,
        reminder_id: ReminderId,
    ) -> ApplicationResult<Vec<DeliveryEvent>> {
        self.delivery_events()
            .find(doc! { "reminder_id": reminder_id.value() }, None)
            .await
            .map_err(repo_err)?
            .map_err(repo_err)
            .and_then(|dto| futures::future::ready(dto.try_into()))
            .try_collect()
            .await
    }
}

#[async_trait]
impl SubscriptionRepository for MongoStore {
    async fn find_subscription(&self, chat_id: ChatId) -> ApplicationResult<Option<Subscription>> {
        let dto = self
            .subscriptions()
            .find_one(
                doc! { "subject_type": "chat", "subject_id": chat_id.value() },
                None,
            )
            .await
            .map_err(repo_err)?;
        dto.map(TryInto::try_into).transpose()
    }

    async fn save_subscription(&self, subscription: &Subscription) -> ApplicationResult<()> {
        let dto = SubscriptionDto::from(subscription.clone());
        self.subscriptions()
            .replace_one(
                doc! { "subject_type": dto.subject_type.clone(), "subject_id": dto.subject_id },
                dto,
                mongodb::options::ReplaceOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await
            .map_err(repo_err)?;
        Ok(())
    }
}

#[async_trait]
impl PaymentRepository for MongoStore {
    async fn find_payment(&self, payment_id: &PaymentId) -> ApplicationResult<Option<Payment>> {
        let dto = self
            .payments()
            .find_one(doc! { "_id": payment_id.as_str() }, None)
            .await
            .map_err(repo_err)?;
        dto.map(PaymentDto::try_into_payment).transpose()
    }

    async fn save_payment(&self, payment: &Payment) -> ApplicationResult<()> {
        let dto = PaymentDto::from_payment(payment.clone());
        let mut set = doc! {
            "amount": dto.amount,
            "currency": dto.currency.clone(),
            "status": dto.status.clone(),
            "created_at": bson_datetime(dto.created_at),
        };
        if let Some(provider) = dto.provider.clone() {
            set.insert("provider", provider);
        }
        if let Some(subscription_id) = dto.subscription_id {
            set.insert("subscription_id", subscription_id);
        }
        if let Some(provider_payment_id) = dto.provider_payment_id.clone() {
            set.insert("provider_payment_id", provider_payment_id);
        }

        self.payments()
            .update_one(
                doc! { "_id": dto.payment_id.clone() },
                doc! {
                    "$set": set,
                    "$setOnInsert": { "fulfilled": false },
                },
                UpdateOptions::builder().upsert(true).build(),
            )
            .await
            .map_err(repo_err)?;
        Ok(())
    }
}

#[async_trait]
impl PaymentTransactionRepository for MongoStore {
    async fn find_payment_transaction(
        &self,
        payment_id: &PaymentId,
    ) -> ApplicationResult<Option<PaymentTransaction>> {
        let dto = self
            .payments()
            .find_one(doc! { "_id": payment_id.as_str() }, None)
            .await
            .map_err(repo_err)?;
        dto.map(PaymentDto::try_into_transaction).transpose()
    }

    async fn save_payment_transaction(
        &self,
        transaction: &PaymentTransaction,
    ) -> ApplicationResult<()> {
        let dto = PaymentDto::from_transaction(transaction.clone());
        let mut set = doc! {
            "amount": dto.amount,
            "currency": dto.currency.clone(),
            "status": dto.status.clone(),
            "fulfilled": dto.fulfilled,
            "created_at": bson_datetime(dto.created_at),
        };
        if let Some(user_id) = dto.user_id {
            set.insert("user_id", user_id);
        }
        if let Some(updated_at) = dto.updated_at {
            set.insert("updated_at", bson_datetime(updated_at));
        }
        if let Some(months) = dto.months {
            set.insert("months", months);
        }
        if let Some(fulfilled_at) = dto.fulfilled_at {
            set.insert("fulfilled_at", bson_datetime(fulfilled_at));
        }
        if let Some(idempotence_key) = dto.idempotence_key.clone() {
            set.insert("idempotence_key", idempotence_key);
        }
        if let Some(provider) = dto.provider.clone() {
            set.insert("provider", provider);
        }

        self.payments()
            .update_one(
                doc! { "_id": dto.payment_id.clone() },
                doc! { "$set": set },
                UpdateOptions::builder().upsert(true).build(),
            )
            .await
            .map_err(repo_err)?;
        Ok(())
    }
}

#[async_trait]
impl ReferralRepository for MongoStore {
    async fn find_referral(
        &self,
        referrer_id: UserId,
        invited_id: UserId,
    ) -> ApplicationResult<Option<Referral>> {
        let dto = self
            .referrals()
            .find_one(
                doc! { "referrer_user_id": referrer_id.value(), "invited_user_id": invited_id.value() },
                None,
            )
            .await
            .map_err(repo_err)?;
        dto.map(TryInto::try_into).transpose()
    }

    async fn save_referral(&self, referral: &Referral) -> ApplicationResult<()> {
        let dto = ReferralDto::from(referral.clone());
        self.referrals()
            .replace_one(
                doc! { "invited_user_id": dto.invited_id },
                dto,
                mongodb::options::ReplaceOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await
            .map_err(repo_err)?;
        Ok(())
    }
}

#[async_trait]
impl ChannelSubscriptionRepository for MongoStore {
    async fn list_channel_subscriptions(
        &self,
        user_id: UserId,
    ) -> ApplicationResult<Vec<ChannelSubscription>> {
        self.list_external_channel_subscriptions(user_id).await
    }

    async fn save_channel_subscription(
        &self,
        subscription: &ChannelSubscription,
    ) -> ApplicationResult<()> {
        self.save_external_channel_subscription(subscription).await
    }
}

#[async_trait]
impl ExternalChannelSubscriptionRepository for MongoStore {
    async fn list_external_channel_subscriptions(
        &self,
        user_id: UserId,
    ) -> ApplicationResult<Vec<ExternalChannelSubscription>> {
        self.external_channel_subscriptions()
            .find(
                doc! { "subject_type": "user", "subject_id": user_id.value() },
                None,
            )
            .await
            .map_err(repo_err)?
            .map_err(repo_err)
            .and_then(|dto| futures::future::ready(dto.try_into()))
            .try_collect::<Vec<ExternalChannelSubscription>>()
            .await
            .map(|mut subscriptions| {
                subscriptions.sort_by(|left, right| {
                    left.created_at
                        .cmp(&right.created_at)
                        .then_with(|| left.channel_id.cmp(&right.channel_id))
                });
                for (index, subscription) in subscriptions.iter_mut().enumerate() {
                    subscription.sub_num = (index + 1) as i32;
                }
                subscriptions
            })
    }

    async fn save_external_channel_subscription(
        &self,
        subscription: &ExternalChannelSubscription,
    ) -> ApplicationResult<()> {
        let dto = ExternalChannelSubscriptionDto::from(subscription.clone());
        self.external_channel_subscriptions()
            .replace_one(
                doc! {
                    "subject_type": dto.subject_type.clone(),
                    "subject_id": dto.subject_id,
                    "platform": dto.platform.clone(),
                    "channel_id": dto.channel_id.clone(),
                },
                dto,
                mongodb::options::ReplaceOptions::builder()
                    .upsert(true)
                    .build(),
            )
            .await
            .map_err(repo_err)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UserDto {
    #[serde(rename = "_id")]
    id: i64,
    status: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "chrono_datetime_as_bson_datetime_optional"
    )]
    created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    identities: Vec<PlatformIdentityDto>,
    #[serde(default)]
    preferences: UserPreferencesDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    payment_info: Option<String>,
}

impl From<User> for UserDto {
    fn from(value: User) -> Self {
        let preferences = value.preferences();
        Self {
            id: value.id.value(),
            status: user_status_to_str(value.status).to_string(),
            created_at: value.created_at,
            identities: value
                .identities
                .into_iter()
                .map(PlatformIdentityDto::from)
                .collect(),
            preferences: UserPreferencesDto::from(preferences),
            payment_info: value.payment_info,
        }
    }
}

impl TryFrom<UserDto> for User {
    type Error = ApplicationError;

    fn try_from(value: UserDto) -> Result<Self, Self::Error> {
        let user_id = UserId::new(value.id);
        Ok(Self {
            id: user_id,
            status: user_status_from_str(&value.status),
            created_at: value.created_at,
            identities: value
                .identities
                .into_iter()
                .map(|identity| identity.into_domain(user_id))
                .collect::<ApplicationResult<_>>()?,
            time_preferences: value.preferences.time_preferences.clone().try_into()?,
            snooze_buttons: value
                .preferences
                .snooze_policy
                .buttons
                .iter()
                .copied()
                .map(SnoozeDuration::from_minutes)
                .collect(),
            auto_snooze: SnoozeDuration::from_minutes(value.preferences.snooze_policy.auto_snooze),
            payment_info: value.payment_info,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PlatformIdentityDto {
    platform: String,
    external_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    chat_id: Option<i64>,
    #[serde(with = "chrono_datetime_as_bson_datetime")]
    connected_at: DateTime<Utc>,
}

impl From<domain::PlatformIdentity> for PlatformIdentityDto {
    fn from(value: domain::PlatformIdentity) -> Self {
        Self {
            platform: "vk".to_string(),
            external_id: value.external_id,
            chat_id: value.chat_id.map(ChatId::value),
            connected_at: value.connected_at,
        }
    }
}

impl PlatformIdentityDto {
    fn into_domain(self, user_id: UserId) -> ApplicationResult<PlatformIdentity> {
        Ok(PlatformIdentity::new(
            user_id,
            communication_platform_from_str(&self.platform)?,
            self.external_id,
            self.chat_id.map(ChatId::new),
            self.connected_at,
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UserPreferencesDto {
    time_preferences: TimePreferencesDto,
    language: String,
    snooze_policy: SnoozePolicyDto,
    notification_policy: NotificationPolicyDto,
}

impl Default for UserPreferencesDto {
    fn default() -> Self {
        Self::from(User::new(UserId::new(0)).preferences())
    }
}

impl From<UserPreferences> for UserPreferencesDto {
    fn from(value: UserPreferences) -> Self {
        Self {
            time_preferences: TimePreferencesDto::from(value.time_preferences),
            language: language_to_str(value.language).to_string(),
            snooze_policy: SnoozePolicyDto::from(value.snooze_policy),
            notification_policy: NotificationPolicyDto::from(value.notification_policy),
        }
    }
}

impl UserPreferencesDto {
    fn into_domain(self, user_id: UserId) -> ApplicationResult<UserPreferences> {
        Ok(UserPreferences {
            user_id,
            time_preferences: self.time_preferences.try_into()?,
            language: language_from_str(&self.language),
            snooze_policy: self.snooze_policy.into(),
            notification_policy: self.notification_policy.into(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TimePreferencesDto {
    morning: String,
    afternoon: String,
    evening: String,
    utc_offset_seconds: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    time_zone: Option<String>,
}

impl Default for TimePreferencesDto {
    fn default() -> Self {
        Self::from(TimePreferences::default())
    }
}

impl From<TimePreferences> for TimePreferencesDto {
    fn from(value: TimePreferences) -> Self {
        Self {
            morning: value.morning.format("%H:%M").to_string(),
            afternoon: value.afternoon.format("%H:%M").to_string(),
            evening: value.evening.format("%H:%M").to_string(),
            utc_offset_seconds: value.utc_offset.seconds(),
            time_zone: match value.time_zone {
                TimeZone::Utc => Some("UTC".to_string()),
                TimeZone::Fixed(offset) => Some(offset.to_string()),
                TimeZone::Iana(name) => Some(name),
            },
        }
    }
}

impl TryFrom<TimePreferencesDto> for TimePreferences {
    type Error = ApplicationError;

    fn try_from(value: TimePreferencesDto) -> Result<Self, Self::Error> {
        let offset = UtcOffset::from_seconds(value.utc_offset_seconds)?;
        Ok(TimePreferences::from_fixed_offset_strings(
            &value.morning,
            &value.afternoon,
            &value.evening,
            &offset.to_string(),
        )?)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SnoozePolicyDto {
    buttons: Vec<u32>,
    auto_snooze: u32,
}

impl Default for SnoozePolicyDto {
    fn default() -> Self {
        Self::from(SnoozePolicy::default())
    }
}

impl From<SnoozePolicy> for SnoozePolicyDto {
    fn from(value: SnoozePolicy) -> Self {
        Self {
            buttons: value
                .buttons
                .into_iter()
                .map(SnoozeDuration::minutes)
                .collect(),
            auto_snooze: value.auto_snooze.minutes(),
        }
    }
}

impl From<SnoozePolicyDto> for SnoozePolicy {
    fn from(value: SnoozePolicyDto) -> Self {
        Self {
            buttons: value
                .buttons
                .into_iter()
                .map(SnoozeDuration::from_minutes)
                .collect(),
            auto_snooze: SnoozeDuration::from_minutes(value.auto_snooze),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NotificationPolicyDto {
    enabled: bool,
}

impl Default for NotificationPolicyDto {
    fn default() -> Self {
        Self::from(NotificationPolicy::default())
    }
}

impl From<NotificationPolicy> for NotificationPolicyDto {
    fn from(value: NotificationPolicy) -> Self {
        Self {
            enabled: value.enabled,
        }
    }
}

impl From<NotificationPolicyDto> for NotificationPolicy {
    fn from(value: NotificationPolicyDto) -> Self {
        Self {
            enabled: value.enabled,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TaskDto {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    id: Option<i64>,
    user_id: i64,
    title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    status: String,
    priority: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "chrono_datetime_as_bson_datetime_optional"
    )]
    due_at: Option<DateTime<Utc>>,
    #[serde(with = "chrono_datetime_as_bson_datetime")]
    created_at: DateTime<Utc>,
    #[serde(with = "chrono_datetime_as_bson_datetime")]
    updated_at: DateTime<Utc>,
}

impl From<Task> for TaskDto {
    fn from(value: Task) -> Self {
        Self {
            id: value.id.map(TaskId::value),
            user_id: value.user_id.value(),
            title: value.title,
            description: value.description,
            status: task_status_to_str(value.status).to_string(),
            priority: task_priority_to_str(value.priority).to_string(),
            due_at: value.due_at,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

impl TryFrom<TaskDto> for Task {
    type Error = ApplicationError;

    fn try_from(value: TaskDto) -> Result<Self, Self::Error> {
        let mut task = Task::new(UserId::new(value.user_id), value.title, value.created_at);
        if let Some(id) = value.id {
            task.assign_id(TaskId::new(id));
        }
        task.description = value.description;
        task.status = task_status_from_str(&value.status);
        task.priority = task_priority_from_str(&value.priority);
        task.due_at = value.due_at;
        task.updated_at = value.updated_at;
        Ok(task)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReminderDto {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    id: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    task_id: Option<i64>,
    chat_id: i64,
    text: String,
    schedule: ScheduleDto,
    #[serde(with = "chrono_datetime_as_bson_datetime")]
    next_at: DateTime<Utc>,
    status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message_id: Option<i32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "chrono_datetime_as_bson_datetime_optional"
    )]
    snooze_until: Option<DateTime<Utc>>,
    retry_count: u32,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "chrono_datetime_as_bson_datetime_optional"
    )]
    retry_at: Option<DateTime<Utc>>,
}

impl From<Reminder> for ReminderDto {
    fn from(value: Reminder) -> Self {
        Self {
            id: value.id.map(ReminderId::value),
            task_id: value.task_id.map(TaskId::value),
            chat_id: value.chat_id.value(),
            text: value.text,
            schedule: ScheduleDto::from(value.schedule),
            next_at: value.next_at,
            status: reminder_status_to_str(&value.status).to_string(),
            message_id: value.message_id,
            snooze_until: value.snooze_until,
            retry_count: value.retry_count,
            retry_at: value.retry_at,
        }
    }
}

impl TryFrom<ReminderDto> for Reminder {
    type Error = ApplicationError;

    fn try_from(value: ReminderDto) -> Result<Self, Self::Error> {
        let mut reminder = Reminder::new(
            ChatId::new(value.chat_id),
            value.text,
            value.schedule.try_into()?,
            value.next_at,
        );
        if let Some(id) = value.id {
            reminder.assign_id(ReminderId::new(id));
        }
        if let Some(task_id) = value.task_id {
            reminder.attach_task(TaskId::new(task_id));
        }
        reminder.status =
            reminder_status_from_parts(&value.status, value.retry_count, value.retry_at);
        reminder.message_id = value.message_id;
        reminder.snooze_until = value.snooze_until;
        reminder.retry_count = value.retry_count;
        reminder.retry_at = value.retry_at;
        Ok(reminder)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScheduleDto {
    kind: String,
    time: TimeSpecDto,
    recurrence: Option<RecurrenceRuleDto>,
}

impl From<Schedule> for ScheduleDto {
    fn from(value: Schedule) -> Self {
        match value {
            Schedule::OneTime(time) => Self {
                kind: "one_time".to_string(),
                time: TimeSpecDto::from(time),
                recurrence: None,
            },
            Schedule::Recurring { time, recurrence } => Self {
                kind: "recurring".to_string(),
                time: TimeSpecDto::from(time),
                recurrence: Some(RecurrenceRuleDto::from(recurrence)),
            },
        }
    }
}

impl TryFrom<ScheduleDto> for Schedule {
    type Error = ApplicationError;

    fn try_from(value: ScheduleDto) -> Result<Self, Self::Error> {
        if value.kind == "recurring" {
            Ok(Self::Recurring {
                time: value.time.into(),
                recurrence: value.recurrence.unwrap_or_default().into(),
            })
        } else {
            Ok(Self::OneTime(value.time.into()))
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct TimeSpecDto {
    #[serde(rename = "type")]
    spec_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    anchor: Option<String>,
    offset_minutes: i32,
    offset_hours: i32,
    offset_days: i32,
    offset_weeks: i32,
    offset_months: i32,
    offset_years: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    offset_direction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    weekday: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    date: Option<String>,
    day_of_month: i32,
    week_of_month: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    day_position: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    time_of_day: Option<String>,
}

impl From<TimeSpec> for TimeSpecDto {
    fn from(value: TimeSpec) -> Self {
        Self {
            spec_type: time_spec_type_to_str(value.spec_type).to_string(),
            anchor: value.anchor,
            offset_minutes: value.offset_minutes,
            offset_hours: value.offset_hours,
            offset_days: value.offset_days,
            offset_weeks: value.offset_weeks,
            offset_months: value.offset_months,
            offset_years: value.offset_years,
            offset_direction: value
                .offset_direction
                .map(offset_direction_to_str)
                .map(str::to_string),
            weekday: value.weekday.map(weekday_to_str).map(str::to_string),
            date: value.date,
            day_of_month: value.day_of_month,
            week_of_month: value.week_of_month,
            day_position: value
                .day_position
                .map(day_position_to_str)
                .map(str::to_string),
            time: value.time,
            time_of_day: value
                .time_of_day
                .map(time_of_day_to_str)
                .map(str::to_string),
        }
    }
}

impl From<TimeSpecDto> for TimeSpec {
    fn from(value: TimeSpecDto) -> Self {
        Self {
            spec_type: time_spec_type_from_str(&value.spec_type),
            anchor: value.anchor,
            offset_minutes: value.offset_minutes,
            offset_hours: value.offset_hours,
            offset_days: value.offset_days,
            offset_weeks: value.offset_weeks,
            offset_months: value.offset_months,
            offset_years: value.offset_years,
            offset_direction: value
                .offset_direction
                .as_deref()
                .map(offset_direction_from_str),
            weekday: value.weekday.as_deref().map(weekday_from_str),
            date: value.date,
            day_of_month: value.day_of_month,
            week_of_month: value.week_of_month,
            day_position: value.day_position.as_deref().map(day_position_from_str),
            time: value.time,
            time_of_day: value.time_of_day.as_deref().map(time_of_day_from_str),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RecurrenceRuleDto {
    pattern: String,
    interval: i32,
    filters: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    interval_unit: Option<String>,
    week_of_month: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    day_position: Option<String>,
}

impl From<RecurrenceRule> for RecurrenceRuleDto {
    fn from(value: RecurrenceRule) -> Self {
        Self {
            pattern: recurrence_pattern_to_str(value.pattern).to_string(),
            interval: value.interval,
            filters: value
                .filters
                .into_iter()
                .map(recurrence_filter_to_str)
                .map(str::to_string)
                .collect(),
            interval_unit: value
                .interval_unit
                .map(interval_unit_to_str)
                .map(str::to_string),
            week_of_month: value.week_of_month,
            day_position: value
                .day_position
                .map(day_position_to_str)
                .map(str::to_string),
        }
    }
}

impl From<RecurrenceRuleDto> for RecurrenceRule {
    fn from(value: RecurrenceRuleDto) -> Self {
        Self {
            pattern: recurrence_pattern_from_str(&value.pattern),
            interval: value.interval,
            filters: value
                .filters
                .iter()
                .map(String::as_str)
                .map(recurrence_filter_from_str)
                .collect(),
            interval_unit: value.interval_unit.as_deref().map(interval_unit_from_str),
            week_of_month: value.week_of_month,
            day_position: value.day_position.as_deref().map(day_position_from_str),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DeliveryEventDto {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    id: Option<i64>,
    reminder_id: i32,
    channel: String,
    #[serde(with = "chrono_datetime_as_bson_datetime")]
    planned_at: DateTime<Utc>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "chrono_datetime_as_bson_datetime_optional"
    )]
    sent_at: Option<DateTime<Utc>>,
    result: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
}

impl From<DeliveryEvent> for DeliveryEventDto {
    fn from(value: DeliveryEvent) -> Self {
        let (result, error_code) = delivery_result_to_parts(value.result);
        Self {
            id: value.id.map(DeliveryEventId::value),
            reminder_id: value.reminder_id.value(),
            channel: delivery_channel_to_str(value.channel).to_string(),
            planned_at: value.planned_at,
            sent_at: value.sent_at,
            result: result.to_string(),
            error_code,
        }
    }
}

impl TryFrom<DeliveryEventDto> for DeliveryEvent {
    type Error = ApplicationError;

    fn try_from(value: DeliveryEventDto) -> Result<Self, Self::Error> {
        let mut event = DeliveryEvent::planned(
            ReminderId::new(value.reminder_id),
            delivery_channel_from_str(&value.channel),
            value.planned_at,
        );
        if let Some(id) = value.id {
            event.assign_id(DeliveryEventId::new(id));
        }
        event.sent_at = value.sent_at;
        event.result = delivery_result_from_parts(&value.result, value.error_code);
        Ok(event)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SubscriptionDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subscription_id: Option<i64>,
    subject_type: String,
    subject_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user_id: Option<i64>,
    plan: String,
    source: String,
    is_group: bool,
    group_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    owner_user_id: Option<i64>,
    #[serde(with = "chrono_datetime_as_bson_datetime")]
    expires_at: DateTime<Utc>,
    active: bool,
    free_state: String,
}

impl From<Subscription> for SubscriptionDto {
    fn from(value: Subscription) -> Self {
        Self {
            subscription_id: value.id.map(SubscriptionId::value),
            subject_type: "chat".to_string(),
            subject_id: value.chat_id.value(),
            user_id: value.user_id.map(UserId::value),
            plan: subscription_plan_to_str(value.plan).to_string(),
            source: subscription_source_to_str(value.source).to_string(),
            is_group: value.is_group,
            group_name: value.group_name,
            owner_user_id: value.owner_id.map(UserId::value),
            expires_at: value.expires_at,
            active: value.active,
            free_state: free_state_to_str(value.free_state).to_string(),
        }
    }
}

impl TryFrom<SubscriptionDto> for Subscription {
    type Error = ApplicationError;

    fn try_from(value: SubscriptionDto) -> Result<Self, Self::Error> {
        let mut subscription = Subscription::new_trial(
            ChatId::new(value.subject_id),
            value.expires_at,
            SubscriptionPolicy { trial_days: 0 },
        );
        if value.subject_type != "chat" {
            return Err(repo_message(format!(
                "unsupported subscription subject type: {}",
                value.subject_type
            )));
        }
        if let Some(id) = value.subscription_id {
            subscription.assign_id(SubscriptionId::new(id));
        }
        if let Some(user_id) = value.user_id {
            subscription.link_user(UserId::new(user_id));
        }
        subscription.plan = subscription_plan_from_str(&value.plan);
        subscription.source = subscription_source_from_str(&value.source);
        subscription.is_group = value.is_group;
        subscription.group_name = value.group_name;
        subscription.owner_id = value.owner_user_id.map(UserId::new);
        subscription.expires_at = value.expires_at;
        subscription.active = value.active;
        subscription.free_state = free_state_from_str(&value.free_state);
        Ok(subscription)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PaymentDto {
    #[serde(rename = "_id")]
    payment_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subscription_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider_payment_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user_id: Option<i64>,
    amount: i64,
    currency: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    months: Option<i32>,
    status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    confirmation_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    idempotence_key: Option<String>,
    #[serde(default)]
    fulfilled: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "chrono_datetime_as_bson_datetime_optional"
    )]
    fulfilled_at: Option<DateTime<Utc>>,
    #[serde(with = "chrono_datetime_as_bson_datetime")]
    created_at: DateTime<Utc>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "chrono_datetime_as_bson_datetime_optional"
    )]
    updated_at: Option<DateTime<Utc>>,
}

impl PaymentDto {
    fn from_payment(value: Payment) -> Self {
        Self {
            payment_id: value.id.into_string(),
            subscription_id: value.subscription_id.map(SubscriptionId::value),
            provider: Some(value.provider.to_string()),
            provider_payment_id: value.provider_payment_id,
            user_id: None,
            amount: value.amount.amount,
            currency: value.amount.currency.to_string(),
            months: None,
            status: payment_status_to_str(&value.status).to_string(),
            confirmation_url: None,
            idempotence_key: None,
            fulfilled: false,
            fulfilled_at: None,
            created_at: value.created_at,
            updated_at: None,
        }
    }

    fn from_transaction(value: PaymentTransaction) -> Self {
        Self {
            payment_id: value.payment_id.into_string(),
            subscription_id: None,
            provider: value.provider,
            provider_payment_id: None,
            user_id: Some(value.user_id.value()),
            amount: value.amount.amount,
            currency: value.amount.currency.to_string(),
            months: value.months.map(|months| months.value() as i32),
            status: payment_status_to_str(&value.status).to_string(),
            confirmation_url: None,
            idempotence_key: value.idempotence_key,
            fulfilled: value.fulfilled,
            fulfilled_at: value.fulfilled_at,
            created_at: value.created_at,
            updated_at: Some(value.updated_at),
        }
    }

    fn try_into_payment(self) -> ApplicationResult<Payment> {
        let mut payment = Payment::new(
            PaymentId::new(self.payment_id),
            payment_provider_from_str(self.provider.as_deref().unwrap_or("yookassa")),
            Money::new(self.amount, currency_from_str(&self.currency))?,
            self.created_at,
        );
        if let Some(subscription_id) = self.subscription_id {
            payment.link_subscription(SubscriptionId::new(subscription_id));
        }
        if let Some(provider_payment_id) = self.provider_payment_id {
            payment.set_provider_payment_id(provider_payment_id);
        }
        payment.update_status(payment_status_from_str(&self.status));
        Ok(payment)
    }

    fn try_into_transaction(self) -> ApplicationResult<PaymentTransaction> {
        let user_id = self
            .user_id
            .ok_or_else(|| repo_message("payment transaction user_id is required"))?;
        let months = self.months.map(Months::new).transpose()?;
        let mut transaction = PaymentTransaction::new(
            PaymentId::new(self.payment_id),
            UserId::new(user_id),
            Money::new(self.amount, currency_from_str(&self.currency))?,
            months,
            self.created_at,
        );
        transaction.status = payment_status_from_str(&self.status);
        transaction.updated_at = self.updated_at.unwrap_or(self.created_at);
        transaction.fulfilled = self.fulfilled;
        transaction.fulfilled_at = self.fulfilled_at;
        transaction.idempotence_key = self.idempotence_key;
        transaction.provider = self.provider;
        Ok(transaction)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ReferralDto {
    referrer_id: i64,
    invited_id: i64,
    #[serde(with = "chrono_datetime_as_bson_datetime")]
    created_at: DateTime<Utc>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "chrono_datetime_as_bson_datetime_optional"
    )]
    rewarded_at: Option<DateTime<Utc>>,
}

impl From<Referral> for ReferralDto {
    fn from(value: Referral) -> Self {
        Self {
            referrer_id: value.referrer_id.value(),
            invited_id: value.invited_id.value(),
            created_at: value.created_at,
            rewarded_at: value.rewarded_at,
        }
    }
}

impl TryFrom<ReferralDto> for Referral {
    type Error = ApplicationError;

    fn try_from(value: ReferralDto) -> Result<Self, Self::Error> {
        let mut referral = Referral::new(
            UserId::new(value.referrer_id),
            UserId::new(value.invited_id),
            value.created_at,
        );
        if let Some(rewarded_at) = value.rewarded_at {
            referral.mark_rewarded(rewarded_at);
        }
        Ok(referral)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExternalChannelSubscriptionDto {
    subject_type: String,
    subject_id: i64,
    platform: String,
    channel_id: String,
    channel_name: String,
    url: String,
    #[serde(with = "chrono_datetime_as_bson_datetime")]
    created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_content_id: Option<String>,
    is_live: bool,
}

impl From<ExternalChannelSubscription> for ExternalChannelSubscriptionDto {
    fn from(value: ExternalChannelSubscription) -> Self {
        Self {
            subject_type: "user".to_string(),
            subject_id: value.user_id.value(),
            platform: platform_to_str(value.platform).to_string(),
            channel_id: value.channel_id,
            channel_name: value.channel_name,
            url: value.url,
            created_at: value.created_at,
            last_content_id: value.last_content_id,
            is_live: value.is_live,
        }
    }
}

impl TryFrom<ExternalChannelSubscriptionDto> for ExternalChannelSubscription {
    type Error = ApplicationError;

    fn try_from(value: ExternalChannelSubscriptionDto) -> Result<Self, Self::Error> {
        if value.subject_type != "user" {
            return Err(repo_message(format!(
                "unsupported external channel subscription subject type: {}",
                value.subject_type
            )));
        }
        let mut subscription = ExternalChannelSubscription::new(
            UserId::new(value.subject_id),
            platform_from_str(&value.platform),
            value.channel_id,
            value.channel_name,
            value.url,
            0,
            value.created_at,
        );
        subscription.last_content_id = value.last_content_id;
        subscription.is_live = value.is_live;
        Ok(subscription)
    }
}

fn repo_err(error: impl std::fmt::Display) -> ApplicationError {
    ApplicationError::Repository(error.to_string())
}

fn repo_message(message: impl Into<String>) -> ApplicationError {
    ApplicationError::Repository(message.into())
}

fn bson_datetime(value: DateTime<Utc>) -> mongodb::bson::DateTime {
    mongodb::bson::DateTime::from_chrono(value)
}

fn generated_i64_id() -> i64 {
    Utc::now().timestamp_micros()
}

fn generated_i32_id() -> i32 {
    (Utc::now().timestamp_micros() & i32::MAX as i64) as i32
}

fn user_status_to_str(value: UserStatus) -> &'static str {
    match value {
        UserStatus::Active => "active",
        UserStatus::Blocked => "blocked",
        UserStatus::Deleted => "deleted",
    }
}

fn user_status_from_str(value: &str) -> UserStatus {
    match value {
        "blocked" => UserStatus::Blocked,
        "deleted" => UserStatus::Deleted,
        _ => UserStatus::Active,
    }
}

fn communication_platform_from_str(value: &str) -> ApplicationResult<CommunicationPlatform> {
    match value {
        "vk" => Ok(CommunicationPlatform::Vk),
        other => Err(repo_message(format!(
            "unsupported communication platform: {other}"
        ))),
    }
}

fn language_to_str(value: Language) -> &'static str {
    match value {
        Language::Russian => "ru",
    }
}

fn language_from_str(_value: &str) -> Language {
    Language::Russian
}

fn task_status_to_str(value: TaskStatus) -> &'static str {
    value.name()
}

fn task_status_from_str(value: &str) -> TaskStatus {
    match value {
        "completed" => TaskStatus::Completed,
        "deleted" => TaskStatus::Deleted,
        _ => TaskStatus::Active,
    }
}

fn task_priority_to_str(value: TaskPriority) -> &'static str {
    match value {
        TaskPriority::Low => "low",
        TaskPriority::Normal => "normal",
        TaskPriority::High => "high",
    }
}

fn task_priority_from_str(value: &str) -> TaskPriority {
    match value {
        "low" => TaskPriority::Low,
        "high" => TaskPriority::High,
        _ => TaskPriority::Normal,
    }
}

fn reminder_status_to_str(value: &ReminderStatus) -> &'static str {
    value.name()
}

fn reminder_status_from_parts(
    status: &str,
    retry_count: u32,
    retry_at: Option<DateTime<Utc>>,
) -> ReminderStatus {
    match status {
        "processing" => ReminderStatus::Processing,
        "retry" => ReminderStatus::Retry {
            attempt: retry_count,
            retry_at: retry_at.unwrap_or_else(Utc::now),
        },
        "sent" => ReminderStatus::Sent,
        "failed" => ReminderStatus::Failed,
        _ => ReminderStatus::Active,
    }
}

fn time_spec_type_to_str(value: TimeSpecType) -> &'static str {
    match value {
        TimeSpecType::Relative => "relative",
        TimeSpecType::Weekday => "weekday",
        TimeSpecType::Absolute => "absolute",
        TimeSpecType::Monthly => "monthly",
        TimeSpecType::Yearly => "yearly",
        TimeSpecType::Daily => "daily",
    }
}

fn time_spec_type_from_str(value: &str) -> TimeSpecType {
    match value {
        "weekday" => TimeSpecType::Weekday,
        "absolute" => TimeSpecType::Absolute,
        "monthly" => TimeSpecType::Monthly,
        "yearly" => TimeSpecType::Yearly,
        "daily" => TimeSpecType::Daily,
        _ => TimeSpecType::Relative,
    }
}

fn offset_direction_to_str(value: OffsetDirection) -> &'static str {
    match value {
        OffsetDirection::After => "after",
        OffsetDirection::Before => "before",
    }
}

fn offset_direction_from_str(value: &str) -> OffsetDirection {
    match value {
        "before" => OffsetDirection::Before,
        _ => OffsetDirection::After,
    }
}

fn weekday_to_str(value: Weekday) -> &'static str {
    match value {
        Weekday::Monday => "monday",
        Weekday::Tuesday => "tuesday",
        Weekday::Wednesday => "wednesday",
        Weekday::Thursday => "thursday",
        Weekday::Friday => "friday",
        Weekday::Saturday => "saturday",
        Weekday::Sunday => "sunday",
    }
}

fn weekday_from_str(value: &str) -> Weekday {
    match value {
        "tuesday" => Weekday::Tuesday,
        "wednesday" => Weekday::Wednesday,
        "thursday" => Weekday::Thursday,
        "friday" => Weekday::Friday,
        "saturday" => Weekday::Saturday,
        "sunday" => Weekday::Sunday,
        _ => Weekday::Monday,
    }
}

fn day_position_to_str(value: DayPosition) -> &'static str {
    match value {
        DayPosition::First => "first",
        DayPosition::Second => "second",
        DayPosition::Third => "third",
        DayPosition::Fourth => "fourth",
        DayPosition::Last => "last",
    }
}

fn day_position_from_str(value: &str) -> DayPosition {
    match value {
        "second" => DayPosition::Second,
        "third" => DayPosition::Third,
        "fourth" => DayPosition::Fourth,
        "last" => DayPosition::Last,
        _ => DayPosition::First,
    }
}

fn time_of_day_to_str(value: TimeOfDay) -> &'static str {
    match value {
        TimeOfDay::Morning => "morning",
        TimeOfDay::Afternoon => "afternoon",
        TimeOfDay::Evening => "evening",
    }
}

fn time_of_day_from_str(value: &str) -> TimeOfDay {
    match value {
        "afternoon" => TimeOfDay::Afternoon,
        "evening" => TimeOfDay::Evening,
        _ => TimeOfDay::Morning,
    }
}

fn recurrence_pattern_to_str(value: RecurrencePattern) -> &'static str {
    match value {
        RecurrencePattern::Daily => "daily",
        RecurrencePattern::Weekly => "weekly",
        RecurrencePattern::Monthly => "monthly",
        RecurrencePattern::Yearly => "yearly",
        RecurrencePattern::Custom => "custom",
    }
}

fn recurrence_pattern_from_str(value: &str) -> RecurrencePattern {
    match value {
        "weekly" => RecurrencePattern::Weekly,
        "monthly" => RecurrencePattern::Monthly,
        "yearly" => RecurrencePattern::Yearly,
        "custom" => RecurrencePattern::Custom,
        _ => RecurrencePattern::Daily,
    }
}

fn recurrence_filter_to_str(value: RecurrenceFilter) -> &'static str {
    match value {
        RecurrenceFilter::Weekdays => "weekdays",
        RecurrenceFilter::Weekends => "weekends",
    }
}

fn recurrence_filter_from_str(value: &str) -> RecurrenceFilter {
    match value {
        "weekends" => RecurrenceFilter::Weekends,
        _ => RecurrenceFilter::Weekdays,
    }
}

fn interval_unit_to_str(value: IntervalUnit) -> &'static str {
    match value {
        IntervalUnit::Days => "days",
        IntervalUnit::Weeks => "weeks",
        IntervalUnit::Months => "months",
        IntervalUnit::Years => "years",
    }
}

fn interval_unit_from_str(value: &str) -> IntervalUnit {
    match value {
        "weeks" => IntervalUnit::Weeks,
        "months" => IntervalUnit::Months,
        "years" => IntervalUnit::Years,
        _ => IntervalUnit::Days,
    }
}

fn delivery_channel_to_str(value: DeliveryChannel) -> &'static str {
    match value {
        DeliveryChannel::Vk => "vk",
    }
}

fn delivery_channel_from_str(_value: &str) -> DeliveryChannel {
    DeliveryChannel::Vk
}

fn delivery_result_to_parts(value: DeliveryResult) -> (&'static str, Option<String>) {
    match value {
        DeliveryResult::Planned => ("planned", None),
        DeliveryResult::Sent => ("sent", None),
        DeliveryResult::TemporaryFailure { error_code } => ("temporary_failure", error_code),
        DeliveryResult::PermanentFailure { error_code } => ("permanent_failure", error_code),
    }
}

fn delivery_result_from_parts(value: &str, error_code: Option<String>) -> DeliveryResult {
    match value {
        "sent" => DeliveryResult::Sent,
        "temporary_failure" => DeliveryResult::TemporaryFailure { error_code },
        "permanent_failure" => DeliveryResult::PermanentFailure { error_code },
        _ => DeliveryResult::Planned,
    }
}

fn subscription_plan_to_str(value: SubscriptionPlan) -> &'static str {
    match value {
        SubscriptionPlan::Basic => "basic",
    }
}

fn subscription_plan_from_str(_value: &str) -> SubscriptionPlan {
    SubscriptionPlan::Basic
}

fn subscription_source_to_str(value: SubscriptionSource) -> &'static str {
    match value {
        SubscriptionSource::Trial => "trial",
        SubscriptionSource::Payment => "payment",
        SubscriptionSource::ReferralReward => "referral_reward",
        SubscriptionSource::AdminGrant => "admin_grant",
    }
}

fn subscription_source_from_str(value: &str) -> SubscriptionSource {
    match value {
        "payment" => SubscriptionSource::Payment,
        "referral_reward" => SubscriptionSource::ReferralReward,
        "admin_grant" => SubscriptionSource::AdminGrant,
        _ => SubscriptionSource::Trial,
    }
}

fn free_state_to_str(value: FreeState) -> &'static str {
    match value {
        FreeState::None => "none",
        FreeState::Trial => "trial",
        FreeState::Paid => "paid",
        FreeState::BonusWeek => "bonus_week",
    }
}

fn free_state_from_str(value: &str) -> FreeState {
    match value {
        "trial" => FreeState::Trial,
        "paid" => FreeState::Paid,
        "bonus_week" => FreeState::BonusWeek,
        _ => FreeState::None,
    }
}

fn payment_provider_from_str(_value: &str) -> PaymentProvider {
    PaymentProvider::YooKassa
}

fn payment_status_to_str(value: &PaymentStatus) -> &str {
    match value {
        PaymentStatus::Pending => "pending",
        PaymentStatus::WaitingForCapture => "waiting_for_capture",
        PaymentStatus::Succeeded => "succeeded",
        PaymentStatus::Canceled => "canceled",
        PaymentStatus::Failed => "failed",
        PaymentStatus::Unknown(value) => value.as_str(),
    }
}

fn payment_status_from_str(value: &str) -> PaymentStatus {
    match value {
        "pending" => PaymentStatus::Pending,
        "waiting_for_capture" => PaymentStatus::WaitingForCapture,
        "succeeded" => PaymentStatus::Succeeded,
        "canceled" => PaymentStatus::Canceled,
        "failed" => PaymentStatus::Failed,
        other => PaymentStatus::Unknown(other.to_string()),
    }
}

fn currency_from_str(_value: &str) -> Currency {
    Currency::Rub
}

fn platform_to_str(value: Platform) -> &'static str {
    match value {
        Platform::Twitch => "twitch",
        Platform::Youtube => "youtube",
    }
}

fn platform_from_str(value: &str) -> Platform {
    match value {
        "youtube" => Platform::Youtube,
        _ => Platform::Twitch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_dto_roundtrips_without_legacy_field_names() {
        let now = Utc::now();
        let mut task = Task::new(UserId::new(7), "title", now);
        task.assign_id(TaskId::new(1));
        task.set_priority(TaskPriority::High, now);

        let dto = TaskDto::from(task.clone());
        let restored: Task = dto.try_into().unwrap();

        assert_eq!(restored.id, task.id);
        assert_eq!(restored.priority, TaskPriority::High);
    }

    #[test]
    fn reminder_dto_keeps_task_separate_from_trigger() {
        let now = Utc::now();
        let mut reminder = Reminder::new(
            ChatId::new(7),
            "text",
            Schedule::OneTime(TimeSpec::default()),
            now,
        );
        reminder.assign_id(ReminderId::new(1));
        reminder.attach_task(TaskId::new(2));

        let dto = ReminderDto::from(reminder.clone());
        let restored: Reminder = dto.try_into().unwrap();

        assert_eq!(restored.id, reminder.id);
        assert_eq!(restored.task_id, Some(TaskId::new(2)));
        assert_eq!(restored.next_at, now);
    }

    #[test]
    fn user_dto_identity_roundtrips_with_user_id() {
        let now = Utc::now();
        let user_id = UserId::new(42);
        let mut user = User::registered(user_id, now);
        user.add_identity(PlatformIdentity::new(
            user_id,
            CommunicationPlatform::Vk,
            "vk-42",
            Some(ChatId::new(7)),
            now,
        ));

        let dto = UserDto::from(user.clone());
        let restored: User = dto.try_into().unwrap();

        assert_eq!(restored.id, user_id);
        assert_eq!(restored.identities.len(), 1);
        assert_eq!(restored.identities[0].user_id, user_id);
        assert_eq!(restored.identities[0].external_id, "vk-42");
    }

    #[test]
    fn user_preferences_are_embedded_in_user_document() {
        let user = User::new(UserId::new(42));

        let document = bson::to_document(&UserDto::from(user)).unwrap();

        assert!(document.contains_key("_id"));
        assert!(document.contains_key("preferences"));
        assert!(!document.contains_key("timePreferences"));
        assert!(!document.contains_key("snoozeButtons"));
        assert!(!document.contains_key("autoSnooze"));
    }

    #[test]
    fn payment_transaction_uses_unified_payment_document() {
        let now = Utc::now();
        let mut transaction = PaymentTransaction::new(
            PaymentId::new("payment-1"),
            UserId::new(7),
            Money::rub(195),
            Some(Months::THREE),
            now,
        );
        transaction.idempotence_key = Some("idem-1".to_string());
        transaction.provider = Some("yookassa".to_string());
        transaction.mark_fulfilled(now);

        let dto = PaymentDto::from_transaction(transaction.clone());
        let document = bson::to_document(&dto).unwrap();
        let restored = dto.try_into_transaction().unwrap();

        assert_eq!(document.get_str("_id").unwrap(), "payment-1");
        assert_eq!(document.get_i64("user_id").unwrap(), 7);
        assert_eq!(document.get_i32("months").unwrap(), 3);
        assert!(!document.contains_key("id"));
        assert_eq!(restored.payment_id, transaction.payment_id);
        assert_eq!(restored.user_id, transaction.user_id);
        assert_eq!(restored.months, transaction.months);
        assert!(restored.fulfilled);
    }

    #[test]
    fn external_channel_subscription_does_not_persist_display_number() {
        let subscription = ExternalChannelSubscription::new(
            UserId::new(7),
            Platform::Twitch,
            "channel",
            "Channel",
            "https://twitch.tv/channel",
            9,
            Utc::now(),
        );

        let document = bson::to_document(&ExternalChannelSubscriptionDto::from(subscription))
            .expect("external channel subscription should serialize");

        assert_eq!(document.get_str("subject_type").unwrap(), "user");
        assert_eq!(document.get_i64("subject_id").unwrap(), 7);
        assert!(!document.contains_key("sub_num"));
        assert!(!document.contains_key("subNum"));
    }
}
