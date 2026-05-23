use anyhow::{Context, Result};
use infrastructure::{MongoStore, RedisPaymentCache};
use tracing::{error, info};
use transport_vk::VkTransport;
use webhook::spawn_yookassa_webhook_server;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();

    let config = WebhookConfig::from_env()?;
    let store = MongoStore::connect(&config.mongo_uri, &config.mongo_db).await?;
    let payment_cache = RedisPaymentCache::new(&config.redis_url)?;
    let transport = VkTransport::new(config.vk_access_token.clone())?;

    spawn_yookassa_webhook_server(
        &config.bind_ip,
        config.port,
        store,
        payment_cache,
        transport,
    )
    .await?;
    shutdown_signal().await;

    Ok(())
}

#[derive(Debug, Clone)]
struct WebhookConfig {
    mongo_uri: String,
    mongo_db: String,
    redis_url: String,
    vk_access_token: String,
    bind_ip: String,
    port: u16,
}

impl WebhookConfig {
    fn from_env() -> Result<Self> {
        Ok(Self {
            mongo_uri: required_env("MONGO_URI")?,
            mongo_db: env_or("MONGO_DB", "tgBot"),
            redis_url: required_env("REDIS_URL")?,
            vk_access_token: required_env("VK_ACCESS_TOKEN")?,
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
