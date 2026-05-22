use anyhow::{Context, Result};
use application::{
    CreateReminderFromTextCommand, CreateReminderFromTextUseCase, EnsureSubscriptionUseCase,
    EnsureUserUseCase, GetProfileUseCase, SetUserTimezoneUseCase,
};
use async_trait::async_trait;
use domain::{ChatId, SubscriptionPolicy, TimePreferences, UserId};
use infrastructure::{
    HttpLlmInterpreter, HttpYooKassaPaymentGateway, MongoStore, RedisPaymentCache, SystemClock,
};
use presentation::{
    CallbackRoute, ConversationState, IncomingCallback, IncomingMessage, MessageRoute,
    Notification, Renderer, RouteContext, Router, TimezoneDisplay,
};
use tracing::{error, info};
use transport_core::BotTransport;
use transport_vk::{normalize_event, VkIncomingEvent, VkTransport};
use vk_bot_api::{
    api::VkApi,
    bot::VkBot,
    error::{VkError, VkResult},
    handler::MessageHandler,
    models::Event,
};

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();

    let config = BotConfig::from_env()?;
    let store = MongoStore::connect(&config.mongo_uri, &config.mongo_db).await?;
    let _payment_cache = RedisPaymentCache::new(&config.redis_url)?;
    let _payment_gateway = config.payment_gateway();
    let llm = HttpLlmInterpreter::new(config.llm_api_url.clone())?;
    let transport = VkTransport::new(config.vk_access_token.clone())?;

    let handler = BotHandler {
        transport,
        store,
        llm,
        clock: SystemClock,
        bot_username: config.bot_username.clone(),
        router: Router,
        renderer: Renderer,
    };

    let mut bot = VkBot::builder()
        .token(config.vk_access_token)
        .group_id(config.vk_group_id)
        .build()?;
    bot.add_handler(handler);

    info!(group_id = config.vk_group_id, "starting VK bot service");
    tokio::select! {
        result = bot.run() => result?,
        () = shutdown_signal() => {}
    }

    Ok(())
}

#[derive(Debug, Clone)]
struct BotConfig {
    mongo_uri: String,
    mongo_db: String,
    redis_url: String,
    vk_access_token: String,
    vk_group_id: i64,
    bot_username: String,
    llm_api_url: String,
    yk_shop_id: Option<String>,
    yk_secret_key: Option<String>,
    yk_return_url: String,
}

impl BotConfig {
    fn from_env() -> Result<Self> {
        Ok(Self {
            mongo_uri: required_env("MONGO_URI")?,
            mongo_db: env_or("MONGO_DB", "tgBot"),
            redis_url: required_env("REDIS_URL")?,
            vk_access_token: required_env("VK_ACCESS_TOKEN")?,
            vk_group_id: required_env("VK_GROUP_ID")?
                .parse()
                .context("VK_GROUP_ID must be an integer")?,
            bot_username: env_or("BOT_USERNAME", "yanapomnyu_bot"),
            llm_api_url: env_or("LLM_API_URL", "http://localhost:8080"),
            yk_shop_id: optional_env("YK_SHOP_ID"),
            yk_secret_key: optional_env("YK_SECRET_KEY"),
            yk_return_url: env_or("YK_RETURN_URL", "https://vk.com/yanapomnyu"),
        })
    }

    fn payment_gateway(&self) -> Option<HttpYooKassaPaymentGateway> {
        Some(HttpYooKassaPaymentGateway::new(
            self.yk_shop_id.clone()?,
            self.yk_secret_key.clone()?,
            self.yk_return_url.clone(),
        ))
    }
}

#[derive(Clone)]
struct BotHandler {
    transport: VkTransport,
    store: MongoStore,
    llm: HttpLlmInterpreter,
    clock: SystemClock,
    bot_username: String,
    router: Router,
    renderer: Renderer,
}

#[async_trait]
impl MessageHandler for BotHandler {
    async fn handle(&self, event: &Event, _api: &VkApi) -> VkResult<()> {
        self.handle_event(event)
            .await
            .map_err(|error| VkError::Custom(error.to_string()))
    }
}

impl BotHandler {
    async fn handle_event(&self, event: &Event) -> Result<()> {
        let Some(event) = normalize_event(event) else {
            return Ok(());
        };

        match event {
            VkIncomingEvent::Message(message) => {
                let incoming = IncomingMessage {
                    peer_id: message.peer_id,
                    user_id: message.user_id,
                    text: message.text,
                    is_group: message.is_group,
                    group_title: message.group_title,
                };
                let route = self.router.route_message_with_context(
                    &incoming,
                    ConversationState::Idle,
                    RouteContext::for_bot(&self.bot_username),
                );
                self.handle_message_route(&incoming, route).await
            }
            VkIncomingEvent::Callback(callback) => {
                let incoming = IncomingCallback::new(
                    callback.event_id,
                    callback.peer_id,
                    callback.user_id,
                    callback.payload,
                );
                let route = self.router.route_callback_action(&incoming);
                self.handle_callback_route(&incoming, route).await
            }
        }
    }

    async fn handle_message_route(
        &self,
        message: &IncomingMessage,
        route: MessageRoute,
    ) -> Result<()> {
        match route {
            MessageRoute::Start => {
                EnsureUserUseCase::new(&self.store)
                    .execute(UserId::new(message.user_id))
                    .await?;
                EnsureSubscriptionUseCase::new(
                    &self.store,
                    &self.clock,
                    SubscriptionPolicy::default(),
                )
                .execute(ChatId::new(message.peer_id))
                .await?;
                self.send_notification(message.peer_id, Notification::Start)
                    .await
            }
            MessageRoute::Help => {
                self.send_notification(message.peer_id, Notification::Help)
                    .await
            }
            MessageRoute::Yan => {
                self.send_notification(message.peer_id, Notification::Yan)
                    .await
            }
            MessageRoute::ShowUtc => {
                self.send_notification(
                    message.peer_id,
                    Notification::UtcPrompt {
                        current: TimezoneDisplay::NotSet,
                    },
                )
                .await
            }
            MessageRoute::ShowSetup => {
                self.send_notification(message.peer_id, Notification::SetupMenu)
                    .await
            }
            MessageRoute::ShowPay => {
                self.send_notification(
                    message.peer_id,
                    Notification::PayMenu {
                        is_active: false,
                        expiry: None,
                    },
                )
                .await
            }
            MessageRoute::ShowProfile => {
                let profile = GetProfileUseCase::new(&self.store, &self.store, &self.clock)
                    .execute(UserId::new(message.user_id), ChatId::new(message.peer_id))
                    .await?;
                self.send_notification(
                    message.peer_id,
                    Notification::ProfilePlaceholder {
                        user_id: profile.user.id.value(),
                    },
                )
                .await
            }
            MessageRoute::CreateReminderFromCommand(text)
            | MessageRoute::ReminderText(text)
            | MessageRoute::GroupReminderText(text) => {
                self.create_task_and_reminder(message.peer_id, message.user_id, &text)
                    .await
            }
            MessageRoute::ListReminders => {
                self.send_text(
                    message.peer_id,
                    "Список задач будет доступен в следующем шаге cutover.",
                )
                .await
            }
            MessageRoute::ShowSubscriptions => {
                self.send_text(
                    message.peer_id,
                    "Отправьте ссылку Twitch или YouTube для подписки на канал.",
                )
                .await
            }
            MessageRoute::ShowReferral => {
                self.send_text(message.peer_id, "Реферальные ссылки VK временно отключены.")
                    .await
            }
            MessageRoute::ChannelSubscriptionUrl(channel) => {
                let text = format!(
                    "Канал распознан: {} ({:?}). Сохранение подписки будет включено в следующем шаге cutover.",
                    channel.channel_name, channel.platform
                );
                self.send_text(message.peer_id, &text).await
            }
            MessageRoute::UnknownCommand(_) => {
                self.send_text(message.peer_id, "Неизвестная команда. Используйте /help")
                    .await
            }
            MessageRoute::Ignored | MessageRoute::Empty => Ok(()),
            MessageRoute::UtcInput(_)
            | MessageRoute::SnoozeButtonsInput(_)
            | MessageRoute::AutoSnoozeInput(_)
            | MessageRoute::ReminderEditText(_)
            | MessageRoute::ReminderDeletionInput(_)
            | MessageRoute::ChannelDeletionInput(_) => {
                self.send_text(
                    message.peer_id,
                    "Этот ввод пока не ожидается новым service binary.",
                )
                .await
            }
        }
    }

    async fn handle_callback_route(
        &self,
        callback: &IncomingCallback,
        route: CallbackRoute,
    ) -> Result<()> {
        self.transport
            .answer_callback(&callback.event_id, callback.user_id, callback.peer_id, None)
            .await?;

        match route {
            CallbackRoute::ShowSetupMenu => {
                self.send_notification(callback.peer_id, Notification::SetupMenu)
                    .await
            }
            CallbackRoute::StartUtcSetup => {
                self.send_notification(
                    callback.peer_id,
                    Notification::UtcPrompt {
                        current: TimezoneDisplay::NotSet,
                    },
                )
                .await
            }
            CallbackRoute::ShowUtcPage(page) => {
                self.send_notification(callback.peer_id, Notification::UtcPage { page })
                    .await
            }
            CallbackRoute::SetUtc(offset) => {
                let preferences =
                    TimePreferences::from_fixed_offset_strings("08:00", "14:00", "19:00", &offset)?;
                SetUserTimezoneUseCase::new(&self.store)
                    .execute(UserId::new(callback.user_id), preferences)
                    .await?;
                self.send_notification(callback.peer_id, Notification::UtcSuccess { offset })
                    .await
            }
            CallbackRoute::ShowPayMenu => {
                self.send_notification(
                    callback.peer_id,
                    Notification::PayMenu {
                        is_active: false,
                        expiry: None,
                    },
                )
                .await
            }
            CallbackRoute::ShowProfile => {
                self.send_notification(
                    callback.peer_id,
                    Notification::ProfilePlaceholder {
                        user_id: callback.user_id,
                    },
                )
                .await
            }
            CallbackRoute::CancelUtc => {
                self.send_notification(callback.peer_id, Notification::UtcCancelled)
                    .await
            }
            _ => {
                self.send_text(
                    callback.peer_id,
                    "Действие будет доступно в следующем шаге cutover.",
                )
                .await
            }
        }
    }

    async fn create_task_and_reminder(&self, peer_id: i64, user_id: i64, text: &str) -> Result<()> {
        let created = CreateReminderFromTextUseCase::new(
            &self.store,
            &self.store,
            &self.store,
            &self.llm,
            &self.clock,
        )
        .execute(CreateReminderFromTextCommand {
            user_id: UserId::new(user_id),
            chat_id: ChatId::new(peer_id),
            text: text.to_string(),
        })
        .await?;
        let reminder = created.reminder;

        let text = format!(
            "Запомнил: {}\nСработает: {}",
            reminder.text,
            reminder.next_at.format("%d.%m.%Y %H:%M UTC")
        );
        self.send_text(peer_id, &text).await
    }

    async fn send_notification(&self, peer_id: i64, notification: Notification) -> Result<()> {
        let content = self
            .renderer
            .render(notification, self.transport.capabilities());
        self.transport.send_message(peer_id, content).await?;
        Ok(())
    }

    async fn send_text(&self, peer_id: i64, text: &str) -> Result<()> {
        self.transport.send_text(peer_id, text).await?;
        Ok(())
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
