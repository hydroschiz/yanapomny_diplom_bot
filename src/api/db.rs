use anyhow::Result;
use chrono::{DateTime, Datelike, Timelike, Utc};
use mongodb::{
    Client, Collection, Database, IndexModel,
    bson::{
        Document, doc,
        serde_helpers::{
            chrono_datetime_as_bson_datetime, chrono_datetime_as_bson_datetime_optional,
        },
    },
    options::{ClientOptions, FindOneAndUpdateOptions, IndexOptions, ReturnDocument},
};
use serde::{Deserialize, Serialize};

const DB_NAME: &str = "tgBot";
const USERS_COLLECTION: &str = "users";
const REMINDERS_COLLECTION: &str = "reminds";
const RECORDS_COLLECTION: &str = "records";
const TRANSACTIONS_COLLECTION: &str = "transactions";
const CHANNEL_SUBS_COLLECTION: &str = "channel_subscriptions";
const REFERRALS_COLLECTION: &str = "referrals";

/// Platform type for channel subscriptions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Twitch,
    Youtube,
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Platform::Twitch => write!(f, "Twitch"),
            Platform::Youtube => write!(f, "YouTube"),
        }
    }
}

/// Channel subscription for Twitch/YouTube notifications.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelSubscription {
    /// User's Telegram chat ID.
    #[serde(rename = "userId")]
    pub user_id: i64,
    /// Platform: twitch or youtube.
    pub platform: Platform,
    /// Channel ID on the platform (username for Twitch, channel ID for YouTube).
    #[serde(rename = "channelId")]
    pub channel_id: String,
    /// Channel display name.
    #[serde(rename = "channelName")]
    pub channel_name: String,
    /// Original URL provided by user.
    pub url: String,
    /// Subscription number for this user (1, 2, 3...).
    #[serde(rename = "subNum")]
    pub sub_num: i32,
    /// When the subscription was created.
    #[serde(rename = "createdAt", with = "chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    /// Last known stream/video ID to detect new content.
    #[serde(rename = "lastContentId", default)]
    pub last_content_id: Option<String>,
    /// Whether the channel is currently live (for Twitch).
    #[serde(rename = "isLive", default)]
    pub is_live: bool,
}

/// New payment transaction for YooKassa payments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentTransaction {
    #[serde(rename = "paymentId")]
    pub payment_id: String,
    #[serde(rename = "userId")]
    pub user_id: i64,
    pub amount: f64,
    pub currency: String,
    pub status: String,
    /// Duration in months (3, 6, 12).
    #[serde(default)]
    pub months: Option<i32>,
    #[serde(rename = "createdAt", with = "chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(
        rename = "updatedAt",
        with = "chrono_datetime_as_bson_datetime",
        default = "Utc::now"
    )]
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub fulfilled: Option<bool>,
    #[serde(
        rename = "fulfilledAt",
        default,
        with = "chrono_datetime_as_bson_datetime_optional"
    )]
    pub fulfilled_at: Option<DateTime<Utc>>,
    /// YooKassa idempotence key.
    #[serde(rename = "idempotenceKey", default)]
    pub idempotence_key: Option<String>,
    /// Payment provider (yookassa, etc).
    #[serde(default)]
    pub provider: Option<String>,
}

/// Referral record linking referrer to invited user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Referral {
    /// ID of user who shared the referral link.
    #[serde(rename = "referrerId")]
    pub referrer_id: i64,
    /// ID of user who was invited.
    #[serde(rename = "invitedId")]
    pub invited_id: i64,
    /// When the referral was recorded.
    #[serde(rename = "createdAt", with = "chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    /// When reward was given to referrer (None if not yet rewarded).
    #[serde(
        rename = "rewardedAt",
        default,
        with = "chrono_datetime_as_bson_datetime_optional"
    )]
    pub rewarded_at: Option<DateTime<Utc>>,
}

/// MongoDB wrapper that keeps all DB-specific logic in one place.
#[derive(Clone)]
pub struct Db {
    #[allow(dead_code)]
    client: Client,
    db: Database,
}

impl Db {
    /// Connects to MongoDB and prepares indexes. `db_name` falls back to the legacy value `tgBot`.
    pub async fn connect(uri: impl AsRef<str>, db_name: Option<&str>) -> Result<Self> {
        let mut options = ClientOptions::parse(uri.as_ref()).await?;
        options.app_name = Some("yanapomnyu_bot".into());

        let client = Client::with_options(options)?;
        let db = client.database(db_name.unwrap_or(DB_NAME));

        let instance = Self { client, db };
        instance.ensure_indexes().await?;
        instance.ensure_reminder_counter().await?;

        Ok(instance)
    }

    pub fn users(&self) -> Collection<User> {
        self.db.collection(USERS_COLLECTION)
    }

    pub fn reminders(&self) -> Collection<Reminder> {
        self.db.collection(REMINDERS_COLLECTION)
    }

    pub fn reminder_docs(&self) -> Collection<Document> {
        self.db.collection(REMINDERS_COLLECTION)
    }

    pub fn records(&self) -> Collection<UserRecord> {
        self.db.collection(RECORDS_COLLECTION)
    }

    pub fn transactions(&self) -> Collection<Transaction> {
        self.db.collection(TRANSACTIONS_COLLECTION)
    }

    pub fn channel_subscriptions(&self) -> Collection<ChannelSubscription> {
        self.db.collection(CHANNEL_SUBS_COLLECTION)
    }

    pub fn referrals(&self) -> Collection<Referral> {
        self.db.collection(REFERRALS_COLLECTION)
    }

    pub async fn ensure_user(&self, id: i64) -> Result<User> {
        if let Some(user) = self.find_user(id).await? {
            return Ok(user);
        }
        self.create_user(id).await
    }

    /// Ensures indexes on chat identifiers to mirror the Go implementation.
    async fn ensure_indexes(&self) -> Result<()> {
        self.users()
            .create_index(
                IndexModel::builder()
                    .keys(doc! {"id": 1})
                    .options(IndexOptions::builder().build())
                    .build(),
                None,
            )
            .await?;

        self.records()
            .create_index(
                IndexModel::builder()
                    .keys(doc! {"id": 1})
                    .options(IndexOptions::builder().build())
                    .build(),
                None,
            )
            .await?;

        // Reminders are identified by remID in the legacy bot.
        self.reminders()
            .create_index(
                IndexModel::builder()
                    .keys(doc! {"remID": 1})
                    .options(IndexOptions::builder().build())
                    .build(),
                None,
            )
            .await?;

        // Channel subscriptions index on userId for fast lookups.
        self.channel_subscriptions()
            .create_index(
                IndexModel::builder()
                    .keys(doc! {"userId": 1})
                    .options(IndexOptions::builder().build())
                    .build(),
                None,
            )
            .await?;

        // Compound index for unique channel per user.
        self.channel_subscriptions()
            .create_index(
                IndexModel::builder()
                    .keys(doc! {"userId": 1, "platform": 1, "channelId": 1})
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
                None,
            )
            .await?;

        Ok(())
    }

    /// Legacy bot stored a counter document with fields `number: 1, num: <int>` in the reminders collection.
    async fn ensure_reminder_counter(&self) -> Result<()> {
        let filter = doc! {"number": 1};
        if self
            .reminder_docs()
            .find_one(filter.clone(), None)
            .await?
            .is_none()
        {
            self.reminder_docs()
                .insert_one(doc! {"number": 1, "num": 1}, None)
                .await?;
        }
        Ok(())
    }

    /// Atomically increments and returns the next `remID` value.
    pub async fn next_reminder_id(&self) -> Result<i32> {
        let filter = doc! {"number": 1};
        let update = doc! {"$inc": {"num": 1}};
        let opts = FindOneAndUpdateOptions::builder()
            .return_document(ReturnDocument::After)
            .upsert(true)
            .build();

        let doc = self
            .reminder_docs()
            .find_one_and_update(filter, update, opts)
            .await?
            .unwrap_or_else(|| doc! {"num": 1});

        let next = doc.get_i32("num").unwrap_or(1);
        Ok(next)
    }

    pub async fn find_user(&self, id: i64) -> Result<Option<User>> {
        let filter = doc! {"id": id};
        Ok(self.users().find_one(filter, None).await?)
    }

    pub async fn create_user(&self, id: i64) -> Result<User> {
        let user = User::new(id);
        self.users().insert_one(&user, None).await?;
        Ok(user)
    }

    pub async fn update_timezone(&self, id: i64, timezone: &str) -> Result<()> {
        let filter = doc! {"id": id};
        let update = doc! {"$set": {"timezone": timezone}};
        self.users().update_one(filter, update, None).await?;
        Ok(())
    }

    pub async fn update_utc_and_clear_timezone(&self, id: i64, utc: &str) -> Result<()> {
        let filter = doc! {"id": id};
        let update = doc! {"$set": {"utc": utc, "timezone": ""}};
        self.users().update_one(filter, update, None).await?;
        Ok(())
    }

    pub async fn insert_reminder(&self, mut reminder: Reminder) -> Result<Reminder> {
        reminder.rem_id = Some(self.next_reminder_id().await?);
        self.reminders().insert_one(&reminder, None).await?;
        Ok(reminder)
    }

    pub async fn update_user_state(&self, id: i64, state: &str) -> Result<()> {
        let filter = doc! {"id": id};
        let update = doc! {"$set": {"state": state}};
        self.users().update_one(filter, update, None).await?;
        Ok(())
    }

    pub async fn update_snooze_buttons(&self, id: i64, buttons: Vec<String>) -> Result<()> {
        let filter = doc! {"id": id};
        let update = doc! {"$set": {"delay": buttons}};
        self.users().update_one(filter, update, None).await?;
        Ok(())
    }

    pub async fn update_auto_delay(&self, id: i64, auto: String) -> Result<()> {
        let filter = doc! {"id": id};
        let update = doc! {"$set": {"autodelay": auto}};
        self.users().update_one(filter, update, None).await?;
        Ok(())
    }

    // ===== Subscription / Records methods =====

    /// Find user's subscription record.
    pub async fn find_record(&self, id: i64) -> Result<Option<UserRecord>> {
        let filter = doc! {"id": id};
        Ok(self.records().find_one(filter, None).await?)
    }

    /// Ensure a subscription record exists. Creates one with 7-day trial if not found.
    pub async fn ensure_record(&self, id: i64) -> Result<UserRecord> {
        if let Some(record) = self.find_record(id).await? {
            return Ok(record);
        }
        self.create_record(id).await
    }

    /// Create a new subscription record with 7-day trial.
    pub async fn create_record(&self, id: i64) -> Result<UserRecord> {
        let record = UserRecord::new_trial(id);
        self.records().insert_one(&record, None).await?;
        Ok(record)
    }

    /// Check if subscription is active (nextPaymentDate > now).
    pub async fn is_subscription_active(&self, id: i64) -> Result<bool> {
        if let Some(record) = self.find_record(id).await? {
            return Ok(record.is_active());
        }
        Ok(false)
    }

    /// Extend subscription by N months from current expiry or now (whichever is later).
    pub async fn extend_subscription(&self, id: i64, months: i32) -> Result<DateTime<Utc>> {
        let record = self.ensure_record(id).await?;
        let now = Utc::now();

        // Start from current expiry if still active, otherwise from now
        let base = if record.next_payment_date > now {
            record.next_payment_date
        } else {
            now
        };

        // Add months
        let new_expiry = add_months(base, months);

        let filter = doc! {"id": id};
        let update = doc! {
            "$set": {
                "nextPaymentDate": mongodb::bson::DateTime::from_chrono(new_expiry),
                "active": true,
                "freestate": 2  // Mark as paid user
            }
        };
        self.records().update_one(filter, update, None).await?;

        Ok(new_expiry)
    }

    /// Get subscription expiry date.
    pub async fn get_subscription_expiry(&self, id: i64) -> Result<Option<DateTime<Utc>>> {
        if let Some(record) = self.find_record(id).await? {
            return Ok(Some(record.next_payment_date));
        }
        Ok(None)
    }

    // ===== Transaction methods =====

    /// Save a payment transaction.
    pub async fn save_transaction(&self, tx: &PaymentTransaction) -> Result<()> {
        self.db
            .collection::<PaymentTransaction>("transactions")
            .insert_one(tx, None)
            .await?;
        Ok(())
    }

    /// Find transaction by payment ID.
    pub async fn find_transaction_by_payment_id(
        &self,
        payment_id: &str,
    ) -> Result<Option<PaymentTransaction>> {
        let filter = doc! {"paymentId": payment_id};
        Ok(self
            .db
            .collection::<PaymentTransaction>("transactions")
            .find_one(filter, None)
            .await?)
    }

    /// Update transaction status.
    pub async fn update_transaction_status(
        &self,
        payment_id: &str,
        status: &str,
    ) -> Result<()> {
        let filter = doc! {"paymentId": payment_id};
        let update = doc! {
            "$set": {
                "status": status,
                "updatedAt": mongodb::bson::DateTime::from_chrono(Utc::now())
            }
        };
        self.db
            .collection::<Document>("transactions")
            .update_one(filter, update, None)
            .await?;
        Ok(())
    }

    /// Mark transaction as fulfilled.
    pub async fn mark_transaction_fulfilled(&self, payment_id: &str) -> Result<()> {
        let filter = doc! {"paymentId": payment_id};
        let update = doc! {
            "$set": {
                "fulfilled": true,
                "fulfilledAt": mongodb::bson::DateTime::from_chrono(Utc::now())
            }
        };
        self.db
            .collection::<Document>("transactions")
            .update_one(filter, update, None)
            .await?;
        Ok(())
    }

    /// Check if transaction was already fulfilled.
    pub async fn is_transaction_fulfilled(&self, payment_id: &str) -> Result<bool> {
        if let Some(tx) = self.find_transaction_by_payment_id(payment_id).await? {
            return Ok(tx.fulfilled.unwrap_or(false));
        }
        Ok(false)
    }

    // ===== Reminder methods =====

    /// Get all active reminders for a user, sorted by time.
    pub async fn get_user_reminders(&self, user_id: i64) -> Result<Vec<Reminder>> {
        use futures::TryStreamExt;
        use mongodb::options::FindOptions;

        let filter = doc! {
            "id": user_id,
            "status": { "$ne": "sent" }
        };
        let options = FindOptions::builder()
            .sort(doc! { "time": 1 })
            .build();

        let cursor = self.reminders().find(filter, options).await?;
        let reminders: Vec<Reminder> = cursor.try_collect().await?;
        Ok(reminders)
    }

    /// Atomically claim a batch of due reminders for processing.
    /// Sets status to "processing" and returns the claimed reminders.
    /// This prevents race conditions when multiple scheduler cycles overlap.
    pub async fn claim_due_reminders(&self, batch_size: i64) -> Result<Vec<Reminder>> {
        use mongodb::options::FindOneAndUpdateOptions;
        use mongodb::options::ReturnDocument;

        let now = Utc::now();
        let now_bson = mongodb::bson::DateTime::from_chrono(now);

        // Filter: active OR (retry AND retry_at <= now)
        let filter = doc! {
            "$or": [
                {
                    "status": "active",
                    "time": { "$lte": now_bson.clone() }
                },
                {
                    "status": "retry",
                    "retryAt": { "$lte": now_bson.clone() }
                }
            ]
        };

        let update = doc! {
            "$set": { "status": "processing" }
        };

        let options = FindOneAndUpdateOptions::builder()
            .return_document(ReturnDocument::After)
            .build();

        let mut claimed = Vec::new();
        
        // Claim up to batch_size reminders atomically
        for _ in 0..batch_size {
            match self.reminders().find_one_and_update(
                filter.clone(),
                update.clone(),
                options.clone()
            ).await? {
                Some(reminder) => claimed.push(reminder),
                None => break, // No more due reminders
            }
        }

        Ok(claimed)
    }

    /// Update reminder time and reset to active (for recurring reminders).
    pub async fn update_reminder_time(&self, rem_id: i32, new_time: DateTime<Utc>) -> Result<()> {
        let filter = doc! { "remID": rem_id };
        let update = doc! {
            "$set": {
                "time": mongodb::bson::DateTime::from_chrono(new_time),
                "status": "active",
                "retryCount": 0,
                "retryAt": mongodb::bson::Bson::Null
            }
        };
        self.reminders().update_one(filter, update, None).await?;
        Ok(())
    }

    /// Mark reminder as sent.
    pub async fn mark_reminder_sent(&self, rem_id: i32) -> Result<()> {
        let filter = doc! { "remID": rem_id };
        let update = doc! { "$set": { "status": "sent" } };
        self.reminders().update_one(filter, update, None).await?;
        Ok(())
    }

    /// Schedule reminder for retry with exponential backoff.
    pub async fn schedule_retry(&self, rem_id: i32, retry_count: i32) -> Result<()> {
        // Exponential backoff: 30s, 60s, 120s
        let delay_secs = 30 * (1 << retry_count.min(3));
        let retry_at = Utc::now() + chrono::Duration::seconds(delay_secs);

        let filter = doc! { "remID": rem_id };
        let update = doc! {
            "$set": {
                "status": "retry",
                "retryCount": retry_count + 1,
                "retryAt": mongodb::bson::DateTime::from_chrono(retry_at)
            }
        };
        self.reminders().update_one(filter, update, None).await?;
        Ok(())
    }

    /// Mark reminder as permanently failed (after max retries).
    pub async fn mark_reminder_failed(&self, rem_id: i32) -> Result<()> {
        let filter = doc! { "remID": rem_id };
        let update = doc! { "$set": { "status": "failed" } };
        self.reminders().update_one(filter, update, None).await?;
        Ok(())
    }

    /// Snooze a reminder by a given number of minutes.
    /// Calculates new time and updates the reminder status to "snoozed".
    pub async fn snooze_reminder(&self, rem_id: i32, minutes: i64) -> Result<DateTime<Utc>> {
        let new_time = Utc::now() + chrono::Duration::minutes(minutes);
        let filter = doc! { "remID": rem_id };
        let update = doc! {
            "$set": {
                "time": mongodb::bson::DateTime::from_chrono(new_time),
                "status": "snoozed",
                "snoozeTime": mongodb::bson::DateTime::from_chrono(new_time),
                "retryCount": 0,
                "retryAt": mongodb::bson::Bson::Null
            }
        };
        self.reminders().update_one(filter, update, None).await?;
        Ok(new_time)
    }

    /// Complete (delete) a reminder by rem_id.
    pub async fn complete_reminder(&self, rem_id: i32) -> Result<bool> {
        let filter = doc! { "remID": rem_id };
        let result = self.reminders().delete_one(filter, None).await?;
        Ok(result.deleted_count > 0)
    }

    /// Recover stuck reminders (processing for too long).
    pub async fn recover_stuck_reminders(&self, stuck_threshold_secs: i64) -> Result<i64> {
        // For now, just reset any "processing" reminders back to "active"
        // In production, you'd track processing_started_at
        let filter = doc! { "status": "processing" };
        let update = doc! { "$set": { "status": "active" } };
        let result = self.reminders().update_many(filter, update, None).await?;
        Ok(result.modified_count as i64)
    }

    /// Delete a reminder by rem_id for a specific user.
    pub async fn delete_reminder(&self, user_id: i64, rem_id: i32) -> Result<bool> {
        let filter = doc! {
            "id": user_id,
            "remID": rem_id
        };
        let result = self.reminders().delete_one(filter, None).await?;
        Ok(result.deleted_count > 0)
    }

    /// Find a reminder by rem_id.
    pub async fn find_reminder(&self, rem_id: i32) -> Result<Option<Reminder>> {
        let filter = doc! { "remID": rem_id };
        Ok(self.reminders().find_one(filter, None).await?)
    }

    // ===== Profile statistics methods =====

    /// Count active reminders for a user.
    pub async fn count_active_reminders(&self, user_id: i64) -> Result<i64> {
        let filter = doc! {
            "id": user_id,
            "status": { "$nin": ["sent", "failed"] }
        };
        let count = self.reminders().count_documents(filter, None).await?;
        Ok(count as i64)
    }

    /// Count reminders created this month for a user.
    pub async fn count_reminders_this_month(&self, user_id: i64) -> Result<i64> {
        let now = Utc::now();
        let start_of_month = now
            .with_day(1)
            .unwrap_or(now)
            .with_hour(0)
            .unwrap_or(now)
            .with_minute(0)
            .unwrap_or(now)
            .with_second(0)
            .unwrap_or(now);

        let filter = doc! {
            "id": user_id,
            "time": { "$gte": mongodb::bson::DateTime::from_chrono(start_of_month) }
        };
        let count = self.reminders().count_documents(filter, None).await?;
        Ok(count as i64)
    }

    /// Count reminders created last month for a user.
    pub async fn count_reminders_last_month(&self, user_id: i64) -> Result<i64> {
        let now = Utc::now();
        let start_of_this_month = now
            .with_day(1)
            .unwrap_or(now)
            .with_hour(0)
            .unwrap_or(now)
            .with_minute(0)
            .unwrap_or(now)
            .with_second(0)
            .unwrap_or(now);
        
        let start_of_last_month = start_of_this_month - chrono::Duration::days(
            start_of_this_month.day() as i64
        );
        let start_of_last_month = start_of_last_month.with_day(1).unwrap_or(start_of_last_month);

        let filter = doc! {
            "id": user_id,
            "time": {
                "$gte": mongodb::bson::DateTime::from_chrono(start_of_last_month),
                "$lt": mongodb::bson::DateTime::from_chrono(start_of_this_month)
            }
        };
        let count = self.reminders().count_documents(filter, None).await?;
        Ok(count as i64)
    }

    /// Get the last (most recent) reminder for a user.
    pub async fn get_last_reminder(&self, user_id: i64) -> Result<Option<Reminder>> {
        use mongodb::options::FindOneOptions;

        let filter = doc! {
            "id": user_id,
            "status": { "$nin": ["sent", "failed"] }
        };
        let options = FindOneOptions::builder()
            .sort(doc! { "time": 1 })  // Get the next upcoming reminder
            .build();

        Ok(self.reminders().find_one(filter, options).await?)
    }

    // ===== Subscription expiry methods =====

    /// Get users whose subscriptions expire within N days.
    /// Returns users where nextPaymentDate is between now and now + days.
    pub async fn get_users_with_expiring_subscriptions(&self, days_before: i32) -> Result<Vec<UserRecord>> {
        use futures::TryStreamExt;

        let now = Utc::now();
        let expiry_check_date = now + chrono::Duration::days(days_before as i64);

        let filter = doc! {
            "nextPaymentDate": {
                "$gt": mongodb::bson::DateTime::from_chrono(now),
                "$lte": mongodb::bson::DateTime::from_chrono(expiry_check_date)
            },
            // Only get users who haven't been warned yet
            "expiryWarned": { "$ne": true }
        };

        let cursor = self.records().find(filter, None).await?;
        let records: Vec<UserRecord> = cursor.try_collect().await?;
        Ok(records)
    }

    /// Mark that user has been warned about subscription expiry.
    pub async fn mark_subscription_warning_sent(&self, id: i64) -> Result<()> {
        let filter = doc! { "id": id };
        let update = doc! {
            "$set": {
                "expiryWarned": true,
                "expiryWarnedAt": mongodb::bson::DateTime::from_chrono(Utc::now())
            }
        };
        self.records().update_one(filter, update, None).await?;
        Ok(())
    }

    /// Get users with expired subscriptions (nextPaymentDate < now).
    pub async fn get_expired_subscriptions(&self) -> Result<Vec<UserRecord>> {
        use futures::TryStreamExt;

        let now = Utc::now();

        let filter = doc! {
            "nextPaymentDate": { "$lt": mongodb::bson::DateTime::from_chrono(now) },
            // Only get users whose reminders haven't been deleted yet
            "remindersDeleted": { "$ne": true }
        };

        let cursor = self.records().find(filter, None).await?;
        let records: Vec<UserRecord> = cursor.try_collect().await?;
        Ok(records)
    }

    /// Delete all reminders for a user and mark as deleted in their record.
    pub async fn delete_all_user_reminders(&self, user_id: i64) -> Result<i64> {
        // Delete all reminders (active, retry status - not sent)
        let filter = doc! {
            "id": user_id,
            "status": { "$ne": "sent" }
        };
        let result = self.reminders().delete_many(filter, None).await?;

        // Mark in record that reminders were deleted
        let record_filter = doc! { "id": user_id };
        let record_update = doc! {
            "$set": {
                "remindersDeleted": true,
                "remindersDeletedAt": mongodb::bson::DateTime::from_chrono(Utc::now())
            }
        };
        self.records().update_one(record_filter, record_update, None).await?;

        Ok(result.deleted_count as i64)
    }

    /// Reset expiry warning flags when subscription is renewed.
    /// Should be called after successful payment.
    pub async fn reset_expiry_flags(&self, id: i64) -> Result<()> {
        let filter = doc! { "id": id };
        let update = doc! {
            "$set": {
                "expiryWarned": false,
                "remindersDeleted": false
            },
            "$unset": {
                "expiryWarnedAt": "",
                "remindersDeletedAt": ""
            }
        };
        self.records().update_one(filter, update, None).await?;
        Ok(())
    }

    /// Count active reminders for a user.
    pub async fn count_user_reminders(&self, user_id: i64) -> Result<i64> {
        let filter = doc! {
            "id": user_id,
            "status": { "$ne": "sent" }
        };
        let count = self.reminders().count_documents(filter, None).await?;
        Ok(count as i64)
    }

    // ===== Channel subscriptions methods =====

    /// Get all channel subscriptions for a user.
    pub async fn get_user_channel_subs(&self, user_id: i64) -> Result<Vec<ChannelSubscription>> {
        use futures::TryStreamExt;

        let filter = doc! { "userId": user_id };
        let cursor = self.channel_subscriptions().find(filter, None).await?;
        let subs: Vec<ChannelSubscription> = cursor.try_collect().await?;
        Ok(subs)
    }

    /// Get the next subscription number for a user.
    async fn next_sub_num(&self, user_id: i64) -> Result<i32> {
        let subs = self.get_user_channel_subs(user_id).await?;
        let max_num = subs.iter().map(|s| s.sub_num).max().unwrap_or(0);
        Ok(max_num + 1)
    }

    /// Add a new channel subscription.
    pub async fn add_channel_sub(
        &self,
        user_id: i64,
        platform: Platform,
        channel_id: String,
        channel_name: String,
        url: String,
    ) -> Result<ChannelSubscription> {
        let sub_num = self.next_sub_num(user_id).await?;

        let sub = ChannelSubscription {
            user_id,
            platform,
            channel_id,
            channel_name,
            url,
            sub_num,
            created_at: Utc::now(),
            last_content_id: None,
            is_live: false,
        };

        self.channel_subscriptions().insert_one(&sub, None).await?;
        Ok(sub)
    }

    /// Check if user already subscribed to this channel.
    pub async fn is_channel_subscribed(
        &self,
        user_id: i64,
        platform: Platform,
        channel_id: &str,
    ) -> Result<bool> {
        let filter = doc! {
            "userId": user_id,
            "platform": mongodb::bson::to_bson(&platform)?,
            "channelId": channel_id
        };
        let count = self.channel_subscriptions().count_documents(filter, None).await?;
        Ok(count > 0)
    }

    /// Delete a channel subscription by user_id and sub_num.
    pub async fn delete_channel_sub(&self, user_id: i64, sub_num: i32) -> Result<bool> {
        let filter = doc! {
            "userId": user_id,
            "subNum": sub_num
        };
        let result = self.channel_subscriptions().delete_one(filter, None).await?;
        
        // Renumber remaining subscriptions
        if result.deleted_count > 0 {
            self.renumber_channel_subs(user_id).await?;
        }
        
        Ok(result.deleted_count > 0)
    }

    /// Renumber subscriptions after deletion to maintain sequential order.
    async fn renumber_channel_subs(&self, user_id: i64) -> Result<()> {
        use futures::TryStreamExt;
        use mongodb::options::FindOptions;

        let filter = doc! { "userId": user_id };
        let options = FindOptions::builder().sort(doc! { "subNum": 1 }).build();
        let cursor = self.channel_subscriptions().find(filter, options).await?;
        let subs: Vec<ChannelSubscription> = cursor.try_collect().await?;

        for (idx, sub) in subs.iter().enumerate() {
            let new_num = (idx + 1) as i32;
            if sub.sub_num != new_num {
                let filter = doc! {
                    "userId": user_id,
                    "platform": mongodb::bson::to_bson(&sub.platform)?,
                    "channelId": &sub.channel_id
                };
                let update = doc! { "$set": { "subNum": new_num } };
                self.channel_subscriptions().update_one(filter, update, None).await?;
            }
        }
        Ok(())
    }

    /// Get all unique channel subscriptions grouped by channel for the scheduler.
    pub async fn get_all_channel_subs(&self) -> Result<Vec<ChannelSubscription>> {
        use futures::TryStreamExt;

        let cursor = self.channel_subscriptions().find(doc! {}, None).await?;
        let subs: Vec<ChannelSubscription> = cursor.try_collect().await?;
        Ok(subs)
    }

    /// Update the last content ID and live status for a channel.
    pub async fn update_channel_content(
        &self,
        platform: Platform,
        channel_id: &str,
        last_content_id: Option<String>,
        is_live: bool,
    ) -> Result<()> {
        let filter = doc! {
            "platform": mongodb::bson::to_bson(&platform)?,
            "channelId": channel_id
        };
        let update = doc! {
            "$set": {
                "lastContentId": last_content_id,
                "isLive": is_live
            }
        };
        self.channel_subscriptions().update_many(filter, update, None).await?;
        Ok(())
    }

    /// Get all user IDs subscribed to a specific channel.
    pub async fn get_channel_subscribers(
        &self,
        platform: Platform,
        channel_id: &str,
    ) -> Result<Vec<i64>> {
        use futures::TryStreamExt;

        let filter = doc! {
            "platform": mongodb::bson::to_bson(&platform)?,
            "channelId": channel_id
        };
        let cursor = self.channel_subscriptions().find(filter, None).await?;
        let subs: Vec<ChannelSubscription> = cursor.try_collect().await?;
        Ok(subs.into_iter().map(|s| s.user_id).collect())
    }

    /// Count user's channel subscriptions.
    pub async fn count_user_channel_subs(&self, user_id: i64) -> Result<i64> {
        let filter = doc! { "userId": user_id };
        let count = self.channel_subscriptions().count_documents(filter, None).await?;
        Ok(count as i64)
    }

    // ===== Referral methods =====

    /// Record a referral when a new user is invited.
    /// Returns true if the referral was recorded, false if already exists or self-referral.
    pub async fn record_referral(&self, referrer_id: i64, invited_id: i64) -> Result<bool> {
        // Don't allow self-referral
        if referrer_id == invited_id {
            return Ok(false);
        }

        // Check if already recorded
        let filter = doc! { "invitedId": invited_id };
        if self.referrals().find_one(filter, None).await?.is_some() {
            return Ok(false);
        }

        // Insert referral record
        let referral = Referral {
            referrer_id,
            invited_id,
            created_at: Utc::now(),
            rewarded_at: None,
        };
        self.referrals().insert_one(referral, None).await?;
        Ok(true)
    }

    /// Get the referrer ID for a given invited user.
    pub async fn get_referrer_of(&self, invited_id: i64) -> Result<Option<i64>> {
        let filter = doc! { "invitedId": invited_id };
        Ok(self.referrals().find_one(filter, None).await?.map(|r| r.referrer_id))
    }

    /// Count how many users were invited by a referrer.
    pub async fn count_referrals_by_referrer(&self, referrer_id: i64) -> Result<i64> {
        let filter = doc! { "referrerId": referrer_id };
        let count = self.referrals().count_documents(filter, None).await?;
        Ok(count as i64)
    }

    /// Mark a referral as rewarded and return the referrer ID if eligible.
    /// Returns Some(referrer_id) if reward should be granted, None otherwise.
    pub async fn consume_referral_reward(&self, invited_id: i64) -> Result<Option<i64>> {
        let filter = doc! { "invitedId": invited_id };
        
        if let Some(referral) = self.referrals().find_one(filter.clone(), None).await? {
            // Already rewarded
            if referral.rewarded_at.is_some() {
                return Ok(None);
            }

            // Mark as rewarded
            let update = doc! {
                "$set": {
                    "rewardedAt": mongodb::bson::DateTime::from_chrono(Utc::now())
                }
            };
            self.referrals().update_one(filter, update, None).await?;
            return Ok(Some(referral.referrer_id));
        }

        Ok(None)
    }
}

/// Helper to add months to a DateTime.
fn add_months(dt: DateTime<Utc>, months: i32) -> DateTime<Utc> {
    use chrono::{Datelike, NaiveDate};

    let date = dt.date_naive();
    let mut year = date.year();
    let mut month = date.month() as i32 + months;

    while month > 12 {
        month -= 12;
        year += 1;
    }
    while month < 1 {
        month += 12;
        year -= 1;
    }

    // Handle day overflow (e.g., Jan 31 + 1 month = Feb 28/29)
    let day = date.day().min(days_in_month(year, month as u32));
    let new_date = NaiveDate::from_ymd_opt(year, month as u32, day).unwrap_or(date);
    let new_dt = new_date.and_time(dt.time());

    DateTime::<Utc>::from_naive_utc_and_offset(new_dt, Utc)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    use chrono::NaiveDate;
    if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    }
    .unwrap()
    .signed_duration_since(NaiveDate::from_ymd_opt(year, month, 1).unwrap())
    .num_days() as u32
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    #[serde(rename = "id")]
    pub id: i64,
    pub utc: String,
    #[serde(rename = "timezone")]
    pub time_zone: String,
    #[serde(rename = "delay")]
    pub snooze_buttons: Vec<String>,
    #[serde(rename = "autodelay")]
    pub auto_snooze: String,
    pub morning: String,
    pub afternoon: String,
    pub evening: String,
    pub state: String,
    #[serde(rename = "paymentInfo", default)]
    pub payment_info: Option<String>,
}

impl User {
    pub fn new(id: i64) -> Self {
        Self {
            id,
            utc: "nil".to_string(),
            time_zone: String::new(),
            snooze_buttons: vec![
                "1hourSnooze".into(),
                "3hourSnooze".into(),
                "1daySnooze".into(),
            ],
            auto_snooze: "15minutAutoSnooze".into(),
            morning: "8:00".into(),
            afternoon: "14:00".into(),
            evening: "19:00".into(),
            state: "waiting_for_message".into(),
            payment_info: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reminder {
    #[serde(rename = "id")]
    pub chat_id: i64,
    pub text: String,
    /// Empty string means "no repeat" in the legacy data set.
    #[serde(default)]
    pub delay: String,
    #[serde(with = "chrono_datetime_as_bson_datetime")]
    pub time: DateTime<Utc>,
    pub status: String,
    #[serde(rename = "remID", default)]
    pub rem_id: Option<i32>,
    #[serde(default)]
    pub messageID: Option<i32>,
    #[serde(
        rename = "snoozeTime",
        default,
        with = "chrono_datetime_as_bson_datetime_optional"
    )]
    pub snooze_time: Option<DateTime<Utc>>,
    /// Retry count for failed sends
    #[serde(rename = "retryCount", default)]
    pub retry_count: i32,
    /// Time for next retry attempt
    #[serde(
        rename = "retryAt",
        default,
        with = "chrono_datetime_as_bson_datetime_optional"
    )]
    pub retry_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRecord {
    #[serde(rename = "id")]
    pub chat_id: i64,
    /// Legacy balance field - kept for compatibility, not used in new logic.
    #[serde(default)]
    pub balance: i32,
    #[serde(rename = "isGroup", default)]
    pub is_group: bool,
    #[serde(rename = "groupName", default)]
    pub group_name: String,
    /// Subscription expiry date. Named "nextPaymentDate" for legacy compatibility.
    #[serde(rename = "nextPaymentDate", with = "chrono_datetime_as_bson_datetime")]
    pub next_payment_date: DateTime<Utc>,
    #[serde(default)]
    pub active: bool,
    /// 0 = no trial, 1 = new user (7 days trial), 2 = paid user, 3 = got bonus week
    #[serde(rename = "freestate", default)]
    pub free_state: Option<i32>,
}

impl UserRecord {
    /// Create a new trial subscription (7 days).
    pub fn new_trial(id: i64) -> Self {
        Self {
            chat_id: id,
            balance: 0,
            is_group: false,
            group_name: String::new(),
            next_payment_date: Utc::now() + chrono::Duration::days(7),
            active: true,
            free_state: Some(1), // New user with trial
        }
    }

    /// Check if subscription is currently active.
    pub fn is_active(&self) -> bool {
        self.next_payment_date > Utc::now()
    }

    /// Format expiry date for display.
    pub fn expiry_formatted(&self) -> String {
        self.next_payment_date.format("%d.%m.%Y").to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transaction {
    #[serde(rename = "transactionID")]
    pub transaction_id: String,
    #[serde(rename = "id")]
    pub chat_id: i64,
    pub amount: i32,
    pub currency: String,
    pub status: String,
    #[serde(rename = "createdAt", with = "chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(rename = "updatedAt", with = "chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(rename = "preCheckoutQueryID")]
    pub pre_checkout_query_id: String,
    #[serde(rename = "invoicePayload")]
    pub invoice_payload: String,
    #[serde(rename = "paymentMethod")]
    pub payment_method: String,
    #[serde(rename = "processingResult")]
    pub processing_result: String,
    #[serde(rename = "telegramPaymentChargeID")]
    pub telegram_payment_charge_id: String,
    #[serde(rename = "providerPaymentChargeID")]
    pub provider_payment_charge_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InfoForParse {
    pub utc: String,
    #[serde(rename = "timezone")]
    pub time_zone: String,
    pub morning: String,
    pub afternoon: String,
    pub evening: String,
    pub state: String,
}
