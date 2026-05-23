use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use anyhow::{Context, Result};
use application::{
    ApplicationError, ApplicationResult, Notification, Notifier,
    ProcessSubscriptionPaymentWebhookUseCase,
};
use async_trait::async_trait;
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use domain::{PaymentId, PaymentStatus};
use infrastructure::{MongoStore, RedisPaymentCache, SystemClock};
use serde::Deserialize;
use serde_json::Value;
use tokio::net::TcpListener;
use tracing::{error, info};
use transport_core::BotTransport;
use transport_vk::VkTransport;

#[derive(Clone)]
struct WebhookState {
    store: MongoStore,
    payment_cache: RedisPaymentCache,
    notifier: TransportNotifier<VkTransport>,
    clock: SystemClock,
}

#[derive(Clone)]
struct TransportNotifier<T> {
    transport: T,
}

#[async_trait]
impl<T> Notifier for TransportNotifier<T>
where
    T: BotTransport,
{
    async fn notify(&self, notification: Notification) -> ApplicationResult<()> {
        match notification {
            Notification::Text { chat_id, text } => self
                .transport
                .send_text(chat_id.value(), &text)
                .await
                .map_err(|error| ApplicationError::ExternalService(error.to_string())),
            Notification::Profile(_) => Ok(()),
        }
    }
}

#[derive(Debug, Deserialize)]
struct YooKassaWebhook {
    event: String,
    object: YooKassaPaymentObject,
}

#[derive(Debug, Deserialize)]
struct YooKassaPaymentObject {
    id: String,
    status: String,
    #[serde(default)]
    metadata: HashMap<String, Value>,
}

pub async fn spawn_yookassa_webhook_server(
    bind_ip: &str,
    port: u16,
    store: MongoStore,
    payment_cache: RedisPaymentCache,
    transport: VkTransport,
) -> Result<()> {
    let state = Arc::new(WebhookState {
        store,
        payment_cache,
        notifier: TransportNotifier { transport },
        clock: SystemClock,
    });
    let app = Router::new()
        .route("/yookassa", post(yookassa_webhook))
        .route("/yookassa/webhook", post(yookassa_webhook))
        .with_state(state);
    let addr: SocketAddr = format!("{bind_ip}:{port}")
        .parse()
        .context("invalid webhook bind address")?;
    let listener = TcpListener::bind(addr).await?;

    info!(%addr, "starting YooKassa webhook server");
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app).await {
            error!(%error, "YooKassa webhook server failed");
        }
    });

    Ok(())
}

async fn yookassa_webhook(
    State(state): State<Arc<WebhookState>>,
    Json(payload): Json<YooKassaWebhook>,
) -> StatusCode {
    let payment_id = metadata_string(&payload.object.metadata, "payment_id")
        .unwrap_or(payload.object.id.clone());
    let status = yookassa_status(&payload.object.status, &payload.event);
    let payment_id = PaymentId::new(payment_id);
    let use_case = ProcessSubscriptionPaymentWebhookUseCase::new(
        &state.store,
        &state.store,
        &state.payment_cache,
        &state.store,
        &state.notifier,
        &state.clock,
    );

    match use_case
        .execute_with_provider_payment_id(&payment_id, Some(&payload.object.id), status)
        .await
    {
        Ok(payment) => {
            info!(payment_id = %payment.transaction.payment_id, "processed YooKassa webhook");
            StatusCode::OK
        }
        Err(error) => {
            error!(%error, "failed to process YooKassa webhook");
            StatusCode::OK
        }
    }
}

fn metadata_string(metadata: &HashMap<String, Value>, key: &str) -> Option<String> {
    metadata.get(key).and_then(|value| {
        value
            .as_str()
            .map(ToString::to_string)
            .or_else(|| value.as_i64().map(|value| value.to_string()))
    })
}

fn yookassa_status(status: &str, event: &str) -> PaymentStatus {
    match status {
        "pending" => PaymentStatus::Pending,
        "waiting_for_capture" => PaymentStatus::WaitingForCapture,
        "succeeded" => PaymentStatus::Succeeded,
        "canceled" => PaymentStatus::Canceled,
        _ if event == "payment.canceled" => PaymentStatus::Canceled,
        other => PaymentStatus::Unknown(other.to_string()),
    }
}
