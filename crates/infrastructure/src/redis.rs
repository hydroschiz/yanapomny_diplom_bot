use application::{
    ApplicationError, ApplicationResult, PaymentCachePort, PendingPayment,
    SchedulerDeduplicationPort,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use domain::{Months, PaymentId, UserId};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct RedisPaymentCache {
    client: redis::Client,
    key_prefix: String,
}

impl RedisPaymentCache {
    pub fn new(redis_url: &str) -> ApplicationResult<Self> {
        let client = redis::Client::open(redis_url)
            .map_err(|err| ApplicationError::Repository(err.to_string()))?;
        Ok(Self {
            client,
            key_prefix: "payment".to_string(),
        })
    }

    pub fn with_prefix(mut self, key_prefix: impl Into<String>) -> Self {
        self.key_prefix = key_prefix.into();
        self
    }

    fn pending_key(&self, payment_id: &PaymentId) -> String {
        format!("{}:pending:{}", self.key_prefix, payment_id)
    }

    fn pending_user_key(&self, user_id: UserId) -> String {
        format!("{}:pending:user:{}", self.key_prefix, user_id.value())
    }

    fn notify_key(&self, payment_id: &PaymentId, event: &str) -> String {
        format!("{}:notify:{}:{}", self.key_prefix, event, payment_id)
    }

    fn fulfill_lock_key(&self, payment_id: &PaymentId) -> String {
        format!("{}:lock:fulfill:{}", self.key_prefix, payment_id)
    }

    fn once_key(&self, key: &str) -> String {
        format!("{}:once:{}", self.key_prefix, key)
    }

    fn ttl_until(expires_at: DateTime<Utc>) -> u64 {
        (expires_at - Utc::now()).num_seconds().max(1) as u64
    }
}

#[async_trait]
impl SchedulerDeduplicationPort for RedisPaymentCache {
    async fn once(&self, key: &str, expires_at: DateTime<Utc>) -> ApplicationResult<bool> {
        let mut connection = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|err| ApplicationError::Repository(err.to_string()))?;
        let response: Option<String> = redis::cmd("SET")
            .arg(self.once_key(key))
            .arg("1")
            .arg("NX")
            .arg("EX")
            .arg(Self::ttl_until(expires_at))
            .query_async(&mut connection)
            .await
            .map_err(|err| ApplicationError::Repository(err.to_string()))?;
        Ok(response.is_some())
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct PendingPaymentJson {
    payment_id: String,
    user_id: i64,
    months: Option<i32>,
    confirmation_url: String,
    expires_at: String,
}

impl From<&PendingPayment> for PendingPaymentJson {
    fn from(value: &PendingPayment) -> Self {
        Self {
            payment_id: value.payment_id.as_str().to_string(),
            user_id: value.user_id.value(),
            months: value.months.map(|months| months.value() as i32),
            confirmation_url: value.confirmation_url.clone(),
            expires_at: value.expires_at.to_rfc3339(),
        }
    }
}

impl TryFrom<PendingPaymentJson> for PendingPayment {
    type Error = ApplicationError;

    fn try_from(value: PendingPaymentJson) -> Result<Self, Self::Error> {
        let expires_at = DateTime::parse_from_rfc3339(&value.expires_at)
            .map_err(|err| ApplicationError::Repository(err.to_string()))?
            .with_timezone(&Utc);
        Ok(Self::new(
            PaymentId::new(value.payment_id),
            UserId::new(value.user_id),
            value.months.map(Months::new).transpose()?,
            value.confirmation_url,
            expires_at,
        ))
    }
}

#[async_trait]
impl PaymentCachePort for RedisPaymentCache {
    async fn pending_payment_for_user(
        &self,
        user_id: UserId,
    ) -> ApplicationResult<Option<PendingPayment>> {
        let mut connection = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|err| ApplicationError::Repository(err.to_string()))?;
        let payment_id: Option<String> = connection
            .get(self.pending_user_key(user_id))
            .await
            .map_err(|err| ApplicationError::Repository(err.to_string()))?;
        let Some(payment_id) = payment_id else {
            return Ok(None);
        };
        let value: Option<String> = connection
            .get(self.pending_key(&PaymentId::new(payment_id)))
            .await
            .map_err(|err| ApplicationError::Repository(err.to_string()))?;
        value
            .map(|value| {
                serde_json::from_str::<PendingPaymentJson>(&value)
                    .map_err(|err| ApplicationError::Repository(err.to_string()))?
                    .try_into()
            })
            .transpose()
    }

    async fn remember_pending_payment(&self, payment: &PendingPayment) -> ApplicationResult<()> {
        let mut connection = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|err| ApplicationError::Repository(err.to_string()))?;
        let ttl = Self::ttl_until(payment.expires_at);
        let value = serde_json::to_string(&PendingPaymentJson::from(payment))
            .map_err(|err| ApplicationError::Repository(err.to_string()))?;
        let _: () = connection
            .set_ex(self.pending_key(&payment.payment_id), value, ttl)
            .await
            .map_err(|err| ApplicationError::Repository(err.to_string()))?;
        let _: () = connection
            .set_ex(
                self.pending_user_key(payment.user_id),
                payment.payment_id.as_str(),
                ttl,
            )
            .await
            .map_err(|err| ApplicationError::Repository(err.to_string()))?;
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
        let mut connection = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|err| ApplicationError::Repository(err.to_string()))?;
        let value: Option<String> = connection
            .get(self.pending_key(payment_id))
            .await
            .map_err(|err| ApplicationError::Repository(err.to_string()))?;
        if let Some(value) = value {
            if let Ok(payment) = serde_json::from_str::<PendingPaymentJson>(&value) {
                let _: () = connection
                    .del(self.pending_user_key(UserId::new(payment.user_id)))
                    .await
                    .map_err(|err| ApplicationError::Repository(err.to_string()))?;
            }
        }
        let _: () = connection
            .del(self.pending_key(payment_id))
            .await
            .map_err(|err| ApplicationError::Repository(err.to_string()))?;
        Ok(())
    }

    async fn notify_once(
        &self,
        payment_id: &PaymentId,
        event: &str,
        expires_at: DateTime<Utc>,
    ) -> ApplicationResult<bool> {
        let mut connection = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|err| ApplicationError::Repository(err.to_string()))?;
        let response: Option<String> = redis::cmd("SET")
            .arg(self.notify_key(payment_id, event))
            .arg("1")
            .arg("NX")
            .arg("EX")
            .arg(Self::ttl_until(expires_at))
            .query_async(&mut connection)
            .await
            .map_err(|err| ApplicationError::Repository(err.to_string()))?;
        Ok(response.is_some())
    }

    async fn try_acquire_fulfill_lock(
        &self,
        payment_id: &PaymentId,
        expires_at: DateTime<Utc>,
    ) -> ApplicationResult<bool> {
        let mut connection = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|err| ApplicationError::Repository(err.to_string()))?;
        let response: Option<String> = redis::cmd("SET")
            .arg(self.fulfill_lock_key(payment_id))
            .arg("1")
            .arg("NX")
            .arg("EX")
            .arg(Self::ttl_until(expires_at))
            .query_async(&mut connection)
            .await
            .map_err(|err| ApplicationError::Repository(err.to_string()))?;
        Ok(response.is_some())
    }

    async fn release_fulfill_lock(&self, payment_id: &PaymentId) -> ApplicationResult<()> {
        let mut connection = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|err| ApplicationError::Repository(err.to_string()))?;
        let _: () = connection
            .del(self.fulfill_lock_key(payment_id))
            .await
            .map_err(|err| ApplicationError::Repository(err.to_string()))?;
        Ok(())
    }
}
