use anyhow::Result;
use chrono::{DateTime, Utc};
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
