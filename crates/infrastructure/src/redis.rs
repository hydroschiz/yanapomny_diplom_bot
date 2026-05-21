use application::{ApplicationError, ApplicationResult, PaymentCachePort};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use domain::{PaymentId, UserId};
use redis::AsyncCommands;

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
}

#[async_trait]
impl PaymentCachePort for RedisPaymentCache {
    async fn remember_pending_payment(
        &self,
        payment_id: &PaymentId,
        user_id: UserId,
        expires_at: DateTime<Utc>,
    ) -> ApplicationResult<()> {
        let mut connection = self
            .client
            .get_multiplexed_async_connection()
            .await
            .map_err(|err| ApplicationError::Repository(err.to_string()))?;
        let ttl = (expires_at - Utc::now()).num_seconds().max(1) as u64;
        let value = serde_json::json!({
            "payment_id": payment_id.as_str(),
            "user_id": user_id.value(),
            "expires_at": expires_at.to_rfc3339(),
        })
        .to_string();
        let _: () = connection
            .set_ex(self.pending_key(payment_id), value, ttl)
            .await
            .map_err(|err| ApplicationError::Repository(err.to_string()))?;
        Ok(())
    }
}
