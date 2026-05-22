use std::time::Duration;

use anyhow::{Context, Result};
use application::{
    ApplicationError, ApplicationResult, DeliverDueRemindersUseCase, Notification, Notifier,
};
use async_trait::async_trait;
use domain::{DeliveryChannel, RetryPolicy};
use infrastructure::{MongoStore, SystemClock, TwitchGateway};
use tracing::{error, info, warn};
use transport_core::BotTransport;
use transport_vk::VkTransport;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();

    let config = SchedulerConfig::from_env()?;
    let store = MongoStore::connect(&config.mongo_uri, &config.mongo_db).await?;
    let transport = VkTransport::new(config.vk_access_token.clone())?;
    let notifier = TransportNotifier { transport };
    let clock = SystemClock;

    let twitch_gateway = config.twitch_gateway();
    if twitch_gateway.is_some() {
        warn!("Twitch gateway is configured, but all-subscription polling awaits an application listing port; Twitch loop is disabled");
    } else {
        warn!("Twitch credentials are not configured; Twitch polling loop is disabled in this service binary");
    }
    warn!("Subscription warning/purge loops await application maintenance ports; subscription maintenance is disabled");

    info!(
        interval_secs = config.interval_secs,
        batch_size = config.batch_size,
        "starting scheduler service"
    );

    let mut interval = tokio::time::interval(Duration::from_secs(config.interval_secs));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                let use_case = DeliverDueRemindersUseCase::new(
                    &store,
                    &store,
                    &store,
                    &notifier,
                    &clock,
                    RetryPolicy::default(),
                    DeliveryChannel::Vk,
                );
                match use_case.execute(config.batch_size).await {
                    Ok(report) if report.claimed > 0 => info!(
                        claimed = report.claimed,
                        delivered = report.delivered,
                        failed = report.failed,
                        "processed due reminders"
                    ),
                    Ok(_) => {}
                    Err(error) => error!(%error, "failed to process due reminders"),
                }
            }
            _ = shutdown_signal() => break,
        }
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct SchedulerConfig {
    mongo_uri: String,
    mongo_db: String,
    vk_access_token: String,
    interval_secs: u64,
    batch_size: usize,
    twitch_client_id: Option<String>,
    twitch_access_token: Option<String>,
}

impl SchedulerConfig {
    fn from_env() -> Result<Self> {
        Ok(Self {
            mongo_uri: required_env("MONGO_URI")?,
            mongo_db: env_or("MONGO_DB", "tgBot"),
            vk_access_token: required_env("VK_ACCESS_TOKEN")?,
            interval_secs: env_parse("SCHEDULER_INTERVAL_SECS", 10)?,
            batch_size: env_parse("SCHEDULER_BATCH_SIZE", 100)?,
            twitch_client_id: optional_env("TWITCH_CLIENT_ID"),
            twitch_access_token: optional_env("TWITCH_ACCESS_TOKEN"),
        })
    }

    fn twitch_gateway(&self) -> Option<TwitchGateway> {
        Some(TwitchGateway::new(
            self.twitch_client_id.clone()?,
            self.twitch_access_token.clone()?,
        ))
    }
}

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
