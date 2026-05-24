use std::time::Duration;

use anyhow::{Context, Result};
use application::{
    ApplicationError, ApplicationResult, CheckAllTwitchStreamsUseCase, DeliverDueRemindersUseCase,
    Notification, Notifier, PurgeExpiredSubscriptionsUseCase, WarnExpiringSubscriptionsUseCase,
};
use async_trait::async_trait;
use domain::{DeliveryChannel, Platform, RetryPolicy, SnoozeDuration};
use infrastructure::{MongoStore, RedisPaymentCache, SystemClock, TwitchGateway};
use presentation::keyboard::reminder_snooze_keyboard;
use tracing::{error, info, warn};
use transport_core::BotTransport;
use transport_vk::VkTransport;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();

    let config = SchedulerConfig::from_env()?;
    let store = MongoStore::connect(&config.mongo_uri, &config.mongo_db).await?;
    let dedupe = RedisPaymentCache::new(&config.redis_url)?.with_prefix("scheduler");
    let transport = VkTransport::new(config.vk_access_token.clone())?;
    let notifier = TransportNotifier { transport };
    let clock = SystemClock;

    let twitch_gateway = config.twitch_gateway();
    if twitch_gateway.is_none() {
        warn!("Twitch credentials are not configured; Twitch polling loop is disabled in this service binary");
    }

    info!(
        interval_secs = config.interval_secs,
        batch_size = config.batch_size,
        "starting scheduler service"
    );

    let mut reminder_interval = tokio::time::interval(Duration::from_secs(config.interval_secs));
    let mut subscription_interval =
        tokio::time::interval(Duration::from_secs(config.subscription_interval_secs));
    let mut twitch_interval =
        tokio::time::interval(Duration::from_secs(config.twitch_interval_secs));
    loop {
        tokio::select! {
            _ = reminder_interval.tick() => {
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
            _ = subscription_interval.tick() => {
                let warning_use_case = WarnExpiringSubscriptionsUseCase::new(
                    &store,
                    &store,
                    &dedupe,
                    &notifier,
                    &clock,
                    config.subscription_warning_days,
                );
                match warning_use_case.execute().await {
                    Ok(report) if report.notified > 0 || report.failed > 0 => info!(
                        inspected = report.inspected,
                        notified = report.notified,
                        failed = report.failed,
                        "processed subscription expiration warnings"
                    ),
                    Ok(_) => {}
                    Err(error) => error!(%error, "failed to process subscription expiration warnings"),
                }

                let purge_use_case = PurgeExpiredSubscriptionsUseCase::new(
                    &store,
                    &store,
                    &notifier,
                    &clock,
                );
                match purge_use_case.execute().await {
                    Ok(report) if report.purged > 0 || report.failed > 0 => info!(
                        purged = report.purged,
                        reminders_cancelled = report.reminders_cancelled,
                        notified = report.notified,
                        failed = report.failed,
                        "processed expired subscriptions"
                    ),
                    Ok(_) => {}
                    Err(error) => error!(%error, "failed to process expired subscriptions"),
                }
            }
            _ = twitch_interval.tick() => {
                if let Some(gateway) = twitch_gateway.as_ref() {
                    let twitch_use_case = CheckAllTwitchStreamsUseCase::new(&store, gateway);
                    match twitch_use_case.execute().await {
                        Ok(changed) => {
                            for subscription in changed {
                                if subscription.platform != Platform::Twitch {
                                    continue;
                                }
                                let text = format!(
                                    "<b>{}</b> — начал трансляцию 🎮\n\n{}",
                                    subscription.channel_name,
                                    subscription.url
                                );
                                if let Err(error) = notifier.notify(Notification::Text {
                                    chat_id: domain::ChatId::new(subscription.user_id.value()),
                                    text,
                                }).await {
                                    error!(%error, channel = %subscription.channel_name, "failed to notify Twitch subscriber");
                                }
                            }
                        }
                        Err(error) => error!(%error, "failed to poll Twitch streams"),
                    }
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
    redis_url: String,
    vk_access_token: String,
    interval_secs: u64,
    batch_size: usize,
    subscription_warning_days: i64,
    subscription_interval_secs: u64,
    twitch_interval_secs: u64,
    twitch_client_id: Option<String>,
    twitch_access_token: Option<String>,
}

impl SchedulerConfig {
    fn from_env() -> Result<Self> {
        Ok(Self {
            mongo_uri: required_env("MONGO_URI")?,
            mongo_db: env_or("MONGO_DB", "tgBot"),
            redis_url: required_env("REDIS_URL")?,
            vk_access_token: required_env("VK_ACCESS_TOKEN")?,
            interval_secs: env_parse("SCHEDULER_INTERVAL_SECS", 10)?,
            batch_size: env_parse("SCHEDULER_BATCH_SIZE", 100)?,
            subscription_warning_days: env_parse("SUBSCRIPTION_WARNING_DAYS", 7)?,
            subscription_interval_secs: env_parse("SUBSCRIPTION_INTERVAL_SECS", 3600)?,
            twitch_interval_secs: env_parse("TWITCH_POLL_INTERVAL_SECS", 300)?,
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
            Notification::ReminderDue {
                chat_id,
                reminder_id,
                text,
                snooze_buttons,
            } => {
                let codes = snooze_buttons
                    .into_iter()
                    .filter_map(snooze_duration_to_code)
                    .collect::<Vec<_>>();
                let keyboard = reminder_snooze_keyboard(
                    reminder_id.value(),
                    &codes,
                    self.transport.capabilities(),
                );
                self.transport
                    .send_with_keyboard(chat_id.value(), &text, &keyboard)
                    .await
                    .map_err(|error| ApplicationError::ExternalService(error.to_string()))
            }
            Notification::Profile(_) => Ok(()),
        }
    }
}

fn snooze_duration_to_code(duration: SnoozeDuration) -> Option<String> {
    let code = match duration.minutes() {
        5 => "5minutSnooze",
        10 => "10minutSnooze",
        15 => "15minutSnooze",
        20 => "20minutSnooze",
        30 => "30minutSnooze",
        60 => "1hourSnooze",
        120 => "2hourSnooze",
        180 => "3hourSnooze",
        240 => "4hourSnooze",
        1440 => "1daySnooze",
        2880 => "2daySnooze",
        4320 => "3daySnooze",
        10080 => "7daySnooze",
        _ => return None,
    };
    Some(code.to_string())
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
