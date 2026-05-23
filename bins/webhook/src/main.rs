use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use anyhow::{Context, Result};
use application::ProcessSubscriptionPaymentWebhookUseCase;
use axum::{extract::State, http::StatusCode, routing::post, Json, Router};
use domain::{PaymentId, PaymentStatus};
use infrastructure::{MongoStore, SystemClock};
use serde::Deserialize;
use serde_json::Value;
use tokio::net::TcpListener;
use tracing::{error, info};

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();

    let config = WebhookConfig::from_env()?;
    let store = MongoStore::connect(&config.mongo_uri, &config.mongo_db).await?;
    let state = Arc::new(AppState {
        store,
        clock: SystemClock,
    });
    let app = Router::new()
        .route("/yookassa", post(yookassa_webhook))
        .with_state(state);
    let addr: SocketAddr = format!("{}:{}", config.bind_ip, config.port)
        .parse()
        .context("invalid webhook bind address")?;
    let listener = TcpListener::bind(addr).await?;

    info!(%addr, "starting YooKassa webhook service");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

#[derive(Clone)]
struct AppState {
    store: MongoStore,
    clock: SystemClock,
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

async fn yookassa_webhook(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<YooKassaWebhook>,
) -> StatusCode {
    let payment_id =
        metadata_string(&payload.object.metadata, "payment_id").unwrap_or(payload.object.id);
    let status = yookassa_status(&payload.object.status, &payload.event);
    let payment_id = PaymentId::new(payment_id);
    let use_case =
        ProcessSubscriptionPaymentWebhookUseCase::new(&state.store, &state.store, &state.clock);

    match use_case.execute(&payment_id, status).await {
        Ok(payment) => {
            info!(payment_id = %payment.transaction.payment_id, "processed YooKassa webhook");
            StatusCode::OK
        }
        Err(error) => {
            error!(%error, "failed to process YooKassa webhook");
            StatusCode::INTERNAL_SERVER_ERROR
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

#[derive(Debug, Clone)]
struct WebhookConfig {
    mongo_uri: String,
    mongo_db: String,
    bind_ip: String,
    port: u16,
}

impl WebhookConfig {
    fn from_env() -> Result<Self> {
        Ok(Self {
            mongo_uri: required_env("MONGO_URI")?,
            mongo_db: env_or("MONGO_DB", "tgBot"),
            bind_ip: env_or("IP", "0.0.0.0"),
            port: env_parse("PORT", 3001)?,
        })
    }
}

fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        error!(%error, "failed to listen for shutdown signal");
    }
    info!("shutdown signal received");
}

fn required_env(name: &str) -> Result<String> {
    std::env::var(name).with_context(|| format!("{name} must be set"))
}

fn optional_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn env_or(name: &str, default: &str) -> String {
    optional_env(name).unwrap_or_else(|| default.to_string())
}

fn env_parse<T>(name: &str, default: T) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match optional_env(name) {
        Some(value) => value
            .parse()
            .map_err(|error| anyhow::anyhow!("{name} has invalid value `{value}`: {error}")),
        None => Ok(default),
    }
}
