//! YooKassa payment service with webhook handling.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Context;
use axum::{
    Json, Router,
    extract::State,
    response::IntoResponse,
    routing::post,
};
use chrono::Utc;
use serde_json::json;
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tracing::{debug, info, warn};
use yookassa::models::receipt::{Receipt, ReceiptCustomer, ReceiptItem};
use yookassa::{
    PaymentsApi, YooKassa,
    api::webhooks::{WebhookEvent, WebhookNotification},
    models::{CreatePaymentRequest, Payment, common::IdempotenceKey},
};

use crate::api::cache::{PendingCache, PendingPayment};
use crate::api::db::{Db, PaymentTransaction};

/// Tariff definition.
#[derive(Debug, Clone, Copy)]
pub struct Tariff {
    pub months: i32,
    pub price: i32,         // in rubles
    pub price_per_month: i32,
}

/// Available tariffs.
pub const TARIFFS: &[Tariff] = &[
    Tariff { months: 3, price: 195, price_per_month: 65 },
    Tariff { months: 6, price: 360, price_per_month: 60 },
    Tariff { months: 12, price: 660, price_per_month: 55 },
];

/// Get tariff by months.
pub fn get_tariff(months: i32) -> Option<&'static Tariff> {
    TARIFFS.iter().find(|t| t.months == months)
}

/// Payment service handling YooKassa integration.
pub struct PaymentService {
    pub yk_api: PaymentsApi,
    pub db: Db,
    pub cache: PendingCache,
}

/// Result of payment initialization.
pub struct InitializedPayment {
    pub id: String,
    pub confirmation_url: String,
}

impl PaymentService {
    /// Create service from environment variables.
    pub fn from_env(db: Db) -> anyhow::Result<Self> {
        let shop_id = std::env::var("YK_SHOP_ID").context("env YK_SHOP_ID is required")?;
        let secret = std::env::var("YK_SECRET_KEY").context("env YK_SECRET_KEY is required")?;

        let yk = YooKassa::with_credentials(shop_id, secret)?;
        let yk_api = yk.payments();
        let cache = PendingCache::from_env()?;

        Ok(Self { yk_api, db, cache })
    }

    /// Create a new YooKassa payment.
    pub async fn init_payment(
        &self,
        user_id: i64,
        months: i32,
    ) -> anyhow::Result<InitializedPayment> {
        let tariff = get_tariff(months).context("invalid tariff")?;
        let amount_str = format!("{}.00", tariff.price);
        let amount = yookassa::Amount {
            value: amount_str.clone(),
            currency: yookassa::Currency::Rub,
        };

        let confirmation = json!({
            "type": "redirect",
            "return_url": std::env::var("YK_RETURN_URL")
                .unwrap_or_else(|_| "https://t.me/yanapomnyu_bot".to_string())
        });

        let idempotence_key = IdempotenceKey::new();
        let idem_key_value = idempotence_key.0.clone();

        // Metadata for webhook
        let mut metadata = HashMap::new();
        metadata.insert(
            "user_id".to_string(),
            serde_json::Value::String(user_id.to_string()),
        );
        metadata.insert(
            "months".to_string(),
            serde_json::Value::Number(serde_json::Number::from(months)),
        );

        // Build receipt for fiscalization
        let description = format!("Подписка на {} мес.", months);
        let vat_code: u8 = std::env::var("YK_VAT_CODE")
            .ok()
            .and_then(|s| s.parse::<u8>().ok())
            .unwrap_or(1); // 1 = НДС 20%
        let payment_subject =
            std::env::var("YK_PAYMENT_SUBJECT").unwrap_or_else(|_| "service".to_string());
        let payment_mode =
            std::env::var("YK_PAYMENT_MODE").unwrap_or_else(|_| "full_payment".to_string());
        let tax_system_code: Option<u8> = std::env::var("YK_TAX_SYSTEM_CODE")
            .ok()
            .and_then(|s| s.parse::<u8>().ok());
        let email_suffix =
            std::env::var("YK_RECEIPT_EMAIL_SUFFIX").unwrap_or_else(|_| "yanapomnyu.ru".to_string());
        let customer_email = format!("tg-{}@{}", user_id, email_suffix);

        let mut item = ReceiptItem::new(description.clone(), amount.clone(), "1.0", vat_code);
        item.payment_subject = Some(payment_subject);
        item.payment_mode = Some(payment_mode);

        let mut receipt = Receipt::new(vec![item]);
        if let Some(ts) = tax_system_code {
            receipt.tax_system_code = Some(ts);
        }
        receipt.customer = Some(ReceiptCustomer {
            full_name: None,
            email: Some(customer_email),
            phone: None,
            inn: None,
        });

        let mut request = CreatePaymentRequest::new(amount.clone()).with_metadata(metadata);
        request.description = Some(description);
        request.confirmation = Some(confirmation);
        request.capture = Some(true);
        request.receipt = Some(serde_json::to_value(&receipt)?);

        let payment = self
            .yk_api
            .create_with_idempotency_key(request, idempotence_key)
            .await?;

        info!(
            payment_id = %payment.id,
            user_id = user_id,
            months = months,
            amount = %amount.value,
            "created YooKassa payment"
        );

        // Get confirmation URL
        let confirmation_url = match payment.confirmation {
            Some(yookassa::models::Confirmation::Redirect(ref redirect)) => {
                redirect.confirmation_url.clone()
            }
            _ => anyhow::bail!("no confirmation URL in payment response"),
        };

        // Save to Redis cache
        let pending = PendingPayment {
            payment_id: payment.id.clone(),
            user_id,
            confirmation_url: confirmation_url.clone(),
            idempotence_key: idem_key_value,
            amount: amount.value.clone(),
            currency: "RUB".to_string(),
            months: Some(months),
        };
        self.cache.put(&pending).await?;

        Ok(InitializedPayment {
            id: payment.id,
            confirmation_url,
        })
    }

    /// Get existing pending payment or create new one.
    pub async fn init_or_get_last(
        &self,
        user_id: i64,
        months: i32,
    ) -> anyhow::Result<InitializedPayment> {
        // Check for existing unpaid payment in cache
        if let Some(p) = self.cache.get_by_user(user_id).await? {
            if p.months == Some(months) {
                // Refresh TTL and return existing
                let _ = self.cache.refresh_user_ttl(user_id).await;
                return Ok(InitializedPayment {
                    id: p.payment_id,
                    confirmation_url: p.confirmation_url,
                });
            }
            // Different tariff - clear old and create new
            let _ = self.cache.delete_by_payment(&p.payment_id).await;
        }

        self.init_payment(user_id, months).await
    }

    /// Handle webhook from YooKassa.
    pub async fn handle_webhook(
        &self,
        bot: &Bot,
        notification: WebhookNotification,
    ) -> anyhow::Result<()> {
        let event = notification.event;
        let payment: Payment = serde_json::from_value(notification.object.clone())
            .context("failed to parse payment from notification.object")?;

        debug!(
            ?event,
            payment_id = %payment.id,
            status = ?payment.status,
            paid = payment.paid,
            "received webhook"
        );

        // Extract user_id from metadata
        let user_id = payment
            .metadata
            .as_ref()
            .and_then(|m| m.get("user_id"))
            .and_then(|v| {
                v.as_i64()
                    .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
            });

        // Extract months from metadata
        let months = payment
            .metadata
            .as_ref()
            .and_then(|m| m.get("months"))
            .and_then(|v| {
                v.as_i64()
                    .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
            })
            .map(|m| m as i32);

        let _status_str = serde_json::to_value(&payment.status)?
            .as_str()
            .unwrap_or("")
            .to_string();

        match event {
            WebhookEvent::PaymentSucceeded => {
                if let Some(uid) = user_id {
                    // Deduplicate notifications
                    if self.cache.notify_once(&payment.id, "succeeded").await.unwrap_or(true) {
                        self.fulfill_payment(bot, &payment.id, uid, months.unwrap_or(3)).await?;
                    }
                } else {
                    warn!(payment_id = %payment.id, "user_id not found in metadata");
                }
                // Clear cache
                let _ = self.cache.delete_by_payment(&payment.id).await;
            }
            WebhookEvent::PaymentCanceled => {
                if let Some(uid) = user_id {
                    if self.cache.notify_once(&payment.id, "canceled").await.unwrap_or(true) {
                        let _ = bot
                            .send_message(ChatId(uid), "❌ Оплата отменена.")
                            .await;
                    }
                }
                let _ = self.cache.delete_by_payment(&payment.id).await;
            }
            WebhookEvent::PaymentWaitingForCapture => {
                if let Some(uid) = user_id {
                    if self.cache.notify_once(&payment.id, "waiting").await.unwrap_or(true) {
                        let _ = bot
                            .send_message(ChatId(uid), "⏳ Платёж ожидает подтверждения...")
                            .await;
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Fulfill payment: extend subscription and notify user.
    pub async fn fulfill_payment(
        &self,
        bot: &Bot,
        payment_id: &str,
        user_id: i64,
        months: i32,
    ) -> anyhow::Result<()> {
        // Try to acquire lock to prevent double-fulfillment
        if !self.cache.try_acquire_fulfill_lock(payment_id).await.unwrap_or(true) {
            return Ok(());
        }

        // Check if already fulfilled
        if self.db.is_transaction_fulfilled(payment_id).await? {
            let _ = self.cache.release_fulfill_lock(payment_id).await;
            return Ok(());
        }

        // Extend subscription
        let new_expiry = self.db.extend_subscription(user_id, months).await?;

        // Save transaction
        let tariff = get_tariff(months);
        let tx = PaymentTransaction {
            payment_id: payment_id.to_string(),
            user_id,
            amount: tariff.map(|t| t.price as f64).unwrap_or(0.0),
            currency: "RUB".to_string(),
            status: "succeeded".to_string(),
            months: Some(months),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            fulfilled: Some(true),
            fulfilled_at: Some(Utc::now()),
            idempotence_key: None,
            provider: Some("yookassa".to_string()),
        };
        let _ = self.db.save_transaction(&tx).await;

        // Release lock
        let _ = self.cache.release_fulfill_lock(payment_id).await;

        // Notify user
        let expiry_str = new_expiry.format("%d.%m.%Y").to_string();
        let message = format!(
            "✅ <b>Спасибо за покупку подписки!</b>\n\n\
             <b>Статус:</b> активна\n\
             <b>Действует до:</b> {}\n\n\
             Узнай больше о Яне через команду /yan.",
            expiry_str
        );

        let _ = bot
            .send_message(ChatId(user_id), message)
            .parse_mode(ParseMode::Html)
            .await;

        info!(payment_id = %payment_id, user_id = user_id, months = months, "payment fulfilled");

        Ok(())
    }

    /// Manual payment check (for "Проверить оплату" button).
    pub async fn manual_check(
        &self,
        bot: &Bot,
        user_id: i64,
        payment_id: &str,
    ) -> anyhow::Result<String> {
        // Get payment status from YooKassa
        let payment = self.yk_api.get(payment_id).await?;

        let status_str = serde_json::to_value(&payment.status)
            .ok()
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_default();

        let months = payment
            .metadata
            .as_ref()
            .and_then(|m| m.get("months"))
            .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
            .map(|m| m as i32)
            .unwrap_or(3);

        match status_str.as_str() {
            "succeeded" => {
                // Fulfill if not yet
                self.fulfill_payment(bot, payment_id, user_id, months).await?;
                let _ = self.cache.delete_by_payment(payment_id).await;
                Ok("✅ Платёж найден и подтверждён! Подписка активирована.".to_string())
            }
            "canceled" => {
                let _ = self.cache.delete_by_payment(payment_id).await;
                Ok("❌ Платёж отменён. Оформите новый платёж.".to_string())
            }
            "pending" => {
                Ok("⏳ Платёж в обработке. Подождите или попробуйте позже.".to_string())
            }
            other => {
                Ok(format!("Статус платежа: {}. Подождите или попробуйте позже.", other))
            }
        }
    }

    /// Get pending payment for user.
    pub async fn get_pending_payment(&self, user_id: i64) -> anyhow::Result<Option<PendingPayment>> {
        self.cache.get_by_user(user_id).await
    }

    /// Build Axum router for webhooks.
    pub fn router(self: Arc<Self>, bot: Bot) -> Router {
        let state = WebhookState {
            payment_svc: self,
            bot,
        };

        Router::new()
            .route("/yookassa/webhook", post(yookassa_webhook))
            .with_state(state)
    }
}

#[derive(Clone)]
struct WebhookState {
    payment_svc: Arc<PaymentService>,
    bot: Bot,
}

async fn yookassa_webhook(
    State(state): State<WebhookState>,
    Json(notification): Json<WebhookNotification>,
) -> impl IntoResponse {
    info!(event = ?notification.event, "YooKassa webhook received");

    if let Err(e) = state.payment_svc.handle_webhook(&state.bot, notification).await {
        warn!(%e, "webhook handling error");
    }

    // Always return 200 OK to YooKassa
    axum::http::StatusCode::OK
}
