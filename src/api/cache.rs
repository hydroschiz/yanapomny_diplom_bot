//! Redis cache for pending YooKassa payments.
//! Provides deduplication, idempotency keys storage, and TTL management.

use anyhow::Result;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};

/// Pending payment stored in Redis while awaiting webhook confirmation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingPayment {
    pub payment_id: String,
    pub user_id: i64,
    pub confirmation_url: String,
    pub idempotence_key: String,
    pub amount: String,
    pub currency: String,
    /// Duration in months (3, 6, 12).
    #[serde(default)]
    pub months: Option<i32>,
}

/// Redis cache wrapper for pending payments.
#[derive(Clone)]
pub struct PendingCache {
    client: redis::Client,
    ttl_secs: usize,
    notify_ttl_secs: usize,
    lock_ttl_secs: usize,
}

impl PendingCache {
    /// Create cache from environment variables.
    pub fn from_env() -> Result<Self> {
        let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".into());
        let client = redis::Client::open(url)?;
        let ttl_secs = std::env::var("PENDING_TTL_SECS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(15 * 60); // 15 minutes
        let notify_ttl_secs = std::env::var("WEBHOOK_NOTIFY_TTL_SECS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(60 * 60); // 1 hour dedup for notifications
        let lock_ttl_secs = std::env::var("FULFILL_LOCK_TTL_SECS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(60); // 60s lock

        Ok(Self {
            client,
            ttl_secs,
            notify_ttl_secs,
            lock_ttl_secs,
        })
    }

    fn key_user(user_id: i64) -> String {
        format!("yk:pending:user:{user_id}")
    }

    fn key_pay(payment_id: &str) -> String {
        format!("yk:pending:pay:{payment_id}")
    }

    fn key_notify(event: &str, payment_id: &str) -> String {
        format!("yk:notify:{event}:{payment_id}")
    }

    fn key_lock_fulfill(payment_id: &str) -> String {
        format!("yk:lock:fulfill:{payment_id}")
    }

    /// Store a pending payment with TTL.
    pub async fn put(&self, p: &PendingPayment) -> Result<()> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let json = serde_json::to_string(p)?;
        let ku = Self::key_user(p.user_id);
        let kp = Self::key_pay(&p.payment_id);
        let ttl = self.ttl_secs;

        let _: () = redis::pipe()
            .cmd("SET")
            .arg(&ku)
            .arg(&json)
            .ignore()
            .cmd("EXPIRE")
            .arg(&ku)
            .arg(ttl)
            .ignore()
            .cmd("SET")
            .arg(&kp)
            .arg(p.user_id)
            .ignore()
            .cmd("EXPIRE")
            .arg(&kp)
            .arg(ttl)
            .ignore()
            .query_async(&mut conn)
            .await?;

        Ok(())
    }

    /// Get pending payment by user ID.
    pub async fn get_by_user(&self, user_id: i64) -> Result<Option<PendingPayment>> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let ku = Self::key_user(user_id);
        let data: Option<String> = conn.get(&ku).await?;
        Ok(match data {
            Some(s) => serde_json::from_str(&s).ok(),
            None => None,
        })
    }

    /// Refresh TTL for user's pending payment.
    pub async fn refresh_user_ttl(&self, user_id: i64) -> Result<()> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let ku = Self::key_user(user_id);
        let _: () = conn
            .expire(&ku, i64::try_from(self.ttl_secs).unwrap_or(900))
            .await?;
        Ok(())
    }

    /// Get user ID by payment ID.
    pub async fn user_by_payment(&self, payment_id: &str) -> Result<Option<i64>> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let kp = Self::key_pay(payment_id);
        let v: Option<i64> = conn.get(kp).await?;
        Ok(v)
    }

    /// Delete pending payment by payment ID.
    pub async fn delete_by_payment(&self, payment_id: &str) -> Result<()> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let kp = Self::key_pay(payment_id);
        if let Some(uid) = self.user_by_payment(payment_id).await? {
            let ku = Self::key_user(uid);
            let _: () = redis::pipe().del(kp).del(ku).query_async(&mut conn).await?;
        } else {
            let _: () = redis::pipe().del(kp).query_async(&mut conn).await?;
        }
        Ok(())
    }

    /// Returns true if this is the first notification for this event+payment.
    /// Used to deduplicate webhook notifications.
    pub async fn notify_once(&self, payment_id: &str, event: &str) -> Result<bool> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let key = Self::key_notify(event, payment_id);
        let ttl = i64::try_from(self.notify_ttl_secs).unwrap_or(3600);
        // SET key 1 NX EX ttl - returns OK only if key didn't exist
        let res: Option<String> = redis::cmd("SET")
            .arg(&key)
            .arg("1")
            .arg("NX")
            .arg("EX")
            .arg(ttl)
            .query_async(&mut conn)
            .await?;
        Ok(res.is_some())
    }

    /// Try to acquire a lock for fulfilling a payment.
    /// Returns true if lock was acquired.
    pub async fn try_acquire_fulfill_lock(&self, payment_id: &str) -> Result<bool> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let key = Self::key_lock_fulfill(payment_id);
        let ttl = i64::try_from(self.lock_ttl_secs).unwrap_or(60);
        let res: Option<String> = redis::cmd("SET")
            .arg(&key)
            .arg("1")
            .arg("NX")
            .arg("EX")
            .arg(ttl)
            .query_async(&mut conn)
            .await?;
        Ok(res.is_some())
    }

    /// Release the fulfill lock for a payment.
    pub async fn release_fulfill_lock(&self, payment_id: &str) -> Result<()> {
        let mut conn = self.client.get_multiplexed_async_connection().await?;
        let key = Self::key_lock_fulfill(payment_id);
        let _: () = redis::cmd("DEL").arg(&key).query_async(&mut conn).await?;
        Ok(())
    }
}
