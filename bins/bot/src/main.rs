use std::cmp::Ordering;

use anyhow::{Context, Result};
use application::{
    active_tasks, ApplicationError, CancelReminderUseCase, CompleteReminderUseCase,
    CreateReminderFromTextCommand, CreateReminderFromTextUseCase,
    DeleteExternalChannelSubscriptionCommand, DeleteExternalChannelSubscriptionUseCase,
    DialogState, DialogStateStore, EnsureSubscriptionUseCase, EnsureUserUseCase, GetProfileUseCase,
    ListActiveRemindersUseCase, ListExternalChannelSubscriptionsUseCase, ListTasksUseCase,
    ReminderActionCommand, SaveExternalChannelSubscriptionCommand,
    SaveExternalChannelSubscriptionUseCase, SetAutoSnoozeUseCase, SetSnoozeButtonsUseCase,
    SetUserTimezoneUseCase, SnoozeReminderUseCase,
};
use async_trait::async_trait;
use domain::{
    ChatId, ExternalChannelSubscription, Platform, Reminder, ReminderId, ReminderStatus,
    SnoozeDuration, SubscriptionPolicy, Task, TimePreferences, UserId,
};
use infrastructure::{
    HttpLlmInterpreter, HttpYooKassaPaymentGateway, MongoStore, RedisPaymentCache, SystemClock,
};
use presentation::keyboard::{channel_subs_keyboard, snooze_code_to_label, snooze_code_to_minutes};
use presentation::{
    CallbackRoute, ChannelPlatform, IncomingCallback, IncomingMessage, MessageRoute, Notification,
    ParsedChannelLink, Renderer, RouteContext, Router, TimezoneDisplay,
};
use tracing::{error, info};
use transport_core::BotTransport;
use transport_vk::{run_long_poll, VkEventHandler, VkIncomingEvent, VkTransport};

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
        store: store.clone(),
        llm,
        clock: SystemClock,
        state_store: store,
        bot_username: config.bot_username.clone(),
        router: Router,
        renderer: Renderer,
    };

    info!(group_id = config.vk_group_id, "starting VK bot service");
    tokio::select! {
        result = run_long_poll(config.vk_access_token, config.vk_group_id, handler) => result?,
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
    state_store: MongoStore,
    bot_username: String,
    router: Router,
    renderer: Renderer,
}

#[async_trait]
impl VkEventHandler for BotHandler {
    async fn handle_vk_event(&self, event: VkIncomingEvent) -> Result<()> {
        self.handle_event(event).await
    }
}

impl BotHandler {
    async fn handle_event(&self, event: VkIncomingEvent) -> Result<()> {
        match event {
            VkIncomingEvent::Message(message) => {
                let incoming = IncomingMessage {
                    peer_id: message.peer_id,
                    user_id: message.user_id,
                    text: message.text,
                    is_group: message.is_group,
                    group_title: message.group_title,
                };
                let state = self
                    .state_store
                    .get_state(UserId::new(incoming.user_id))
                    .await?;
                let route = self.router.route_message_with_context(
                    &incoming,
                    state,
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
                self.state_store
                    .set_state(UserId::new(message.user_id), DialogState::AwaitingUtc)
                    .await?;
                self.send_notification(
                    message.peer_id,
                    Notification::UtcPrompt {
                        current: TimezoneDisplay::NotSet,
                    },
                )
                .await
            }
            MessageRoute::UtcInput(input) => {
                self.update_timezone(message.peer_id, message.user_id, &input)
                    .await
            }
            MessageRoute::SnoozeButtonsInput(input) => {
                let buttons = parse_snooze_buttons(&input);
                if buttons.is_empty() {
                    return self
                        .send_text(
                            message.peer_id,
                            "Не смог распознать время. Отправьте числа минутами, например: 15 60 180.",
                        )
                        .await;
                }
                SetSnoozeButtonsUseCase::new(&self.store)
                    .execute(UserId::new(message.user_id), buttons.clone())
                    .await?;
                self.state_store
                    .set_state(UserId::new(message.user_id), DialogState::Idle)
                    .await?;
                self.send_text(
                    message.peer_id,
                    &format!(
                        "Кнопки откладывания обновлены: {}.",
                        format_snooze_buttons(&buttons)
                    ),
                )
                .await
            }
            MessageRoute::AutoSnoozeInput(input) => {
                let Some(minutes) = parse_first_minutes(&input) else {
                    return self
                        .send_text(
                            message.peer_id,
                            "Не смог распознать время. Отправьте число минутами, например: 15.",
                        )
                        .await;
                };
                SetAutoSnoozeUseCase::new(&self.store)
                    .execute(
                        UserId::new(message.user_id),
                        SnoozeDuration::from_minutes(minutes),
                    )
                    .await?;
                self.state_store
                    .set_state(UserId::new(message.user_id), DialogState::Idle)
                    .await?;
                self.send_text(
                    message.peer_id,
                    &format!("Автооткладывание обновлено: {} мин.", minutes),
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
                self.show_active_tasks(message.peer_id, message.user_id)
                    .await
            }
            MessageRoute::ShowSubscriptions => {
                self.show_channel_subscriptions(message.peer_id, message.user_id)
                    .await
            }
            MessageRoute::ShowReferral => {
                self.send_text(message.peer_id, "Реферальные ссылки VK временно отключены.")
                    .await
            }
            MessageRoute::ChannelSubscriptionUrl(channel) => {
                self.save_external_channel_subscription(message.peer_id, message.user_id, channel)
                    .await
            }
            MessageRoute::UnknownCommand(_) => {
                self.send_text(message.peer_id, "Неизвестная команда. Используйте /help")
                    .await
            }
            MessageRoute::Ignored | MessageRoute::Empty => Ok(()),
            MessageRoute::ReminderDeletionInput(input) => {
                self.delete_reminder_from_input(message.peer_id, message.user_id, &input)
                    .await
            }
            MessageRoute::ChannelDeletionInput(input) => {
                self.delete_channel_subscription_from_input(
                    message.peer_id,
                    message.user_id,
                    &input,
                )
                .await
            }
            MessageRoute::ReminderEditText(_) => {
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
            CallbackRoute::StartSnoozeSetup => {
                self.state_store
                    .set_state(
                        UserId::new(callback.user_id),
                        DialogState::AwaitingSnoozeButtons,
                    )
                    .await?;
                self.send_notification(
                    callback.peer_id,
                    Notification::SnoozePrompt {
                        current: "60, 180, 1440 мин".to_string(),
                    },
                )
                .await
            }
            CallbackRoute::StartAutoSnoozeSetup => {
                self.state_store
                    .set_state(
                        UserId::new(callback.user_id),
                        DialogState::AwaitingAutoSnooze,
                    )
                    .await?;
                self.send_notification(
                    callback.peer_id,
                    Notification::AutoSnoozePrompt {
                        current: "15 мин".to_string(),
                    },
                )
                .await
            }
            CallbackRoute::StartUtcSetup => {
                self.state_store
                    .set_state(UserId::new(callback.user_id), DialogState::AwaitingUtc)
                    .await?;
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
                self.update_timezone(callback.peer_id, callback.user_id, &offset)
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
            CallbackRoute::ListReminders => {
                self.show_active_tasks(callback.peer_id, callback.user_id)
                    .await
            }
            CallbackRoute::SnoozeReminder { reminder_id, code } => {
                self.snooze_reminder(callback.peer_id, reminder_id, &code)
                    .await
            }
            CallbackRoute::CompleteReminder(reminder_id) => {
                self.complete_reminder(callback.peer_id, reminder_id).await
            }
            CallbackRoute::StartReminderDeletion => {
                self.start_reminder_deletion(callback.peer_id, callback.user_id)
                    .await
            }
            CallbackRoute::ShowSubscriptions => {
                self.show_channel_subscriptions(callback.peer_id, callback.user_id)
                    .await
            }
            CallbackRoute::StartSubscriptionDeletion => {
                self.start_channel_subscription_deletion(callback.peer_id, callback.user_id)
                    .await
            }
            CallbackRoute::BackFromReminderDeletion | CallbackRoute::CancelReminder => {
                self.state_store
                    .set_state(UserId::new(callback.user_id), DialogState::Idle)
                    .await?;
                self.send_text(callback.peer_id, "Действие отменено.").await
            }
            CallbackRoute::CancelUtc => {
                self.state_store
                    .set_state(UserId::new(callback.user_id), DialogState::Idle)
                    .await?;
                self.send_notification(callback.peer_id, Notification::UtcCancelled)
                    .await
            }
            CallbackRoute::BackMain => {
                self.state_store
                    .set_state(UserId::new(callback.user_id), DialogState::Idle)
                    .await?;
                self.send_notification(callback.peer_id, Notification::Start)
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

    async fn update_timezone(&self, peer_id: i64, user_id: i64, offset: &str) -> Result<()> {
        let preferences =
            TimePreferences::from_fixed_offset_strings("08:00", "14:00", "19:00", offset)?;
        let display_offset = preferences.utc_offset.to_string();
        SetUserTimezoneUseCase::new(&self.store)
            .execute(UserId::new(user_id), preferences)
            .await?;
        self.state_store
            .set_state(UserId::new(user_id), DialogState::Idle)
            .await?;
        self.send_notification(
            peer_id,
            Notification::UtcSuccess {
                offset: display_offset,
            },
        )
        .await
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

    async fn show_active_tasks(&self, peer_id: i64, user_id: i64) -> Result<()> {
        let mut tasks = active_tasks(
            ListTasksUseCase::new(&self.store)
                .execute(UserId::new(user_id))
                .await?,
        );
        tasks.sort_by(compare_tasks_for_list);

        if tasks.is_empty() {
            return self.send_text(peer_id, "Активных задач пока нет.").await;
        }

        self.send_text(peer_id, &format_active_tasks(&tasks)).await
    }

    async fn save_external_channel_subscription(
        &self,
        peer_id: i64,
        user_id: i64,
        channel: ParsedChannelLink,
    ) -> Result<()> {
        let user_id = UserId::new(user_id);
        EnsureUserUseCase::new(&self.store).execute(user_id).await?;
        let subscription = SaveExternalChannelSubscriptionUseCase::new(&self.store, &self.clock)
            .execute(SaveExternalChannelSubscriptionCommand {
                user_id,
                platform: channel_platform(channel.platform),
                channel_id: channel.channel_id,
                channel_name: channel.channel_name,
                url: channel.url,
            })
            .await?;

        self.send_text(
            peer_id,
            &format!(
                "Подписка сохранена: {} ({}).\n{}",
                subscription.channel_name, subscription.platform, subscription.url
            ),
        )
        .await
    }

    async fn show_channel_subscriptions(&self, peer_id: i64, user_id: i64) -> Result<()> {
        let subscriptions = self.channel_subscriptions_for_user(user_id).await?;
        self.send_notification(
            peer_id,
            Notification::PlainText {
                text: format_channel_subscriptions(&subscriptions),
                keyboard: Some(channel_subs_keyboard(self.transport.capabilities())),
            },
        )
        .await
    }

    async fn start_channel_subscription_deletion(&self, peer_id: i64, user_id: i64) -> Result<()> {
        let subscriptions = self.channel_subscriptions_for_user(user_id).await?;
        if subscriptions.is_empty() {
            self.state_store
                .set_state(UserId::new(user_id), DialogState::Idle)
                .await?;
            return self
                .send_text(peer_id, "Подписок для удаления пока нет.")
                .await;
        }

        self.state_store
            .set_state(
                UserId::new(user_id),
                DialogState::AwaitingChannelSubscriptionDeletion,
            )
            .await?;
        self.send_text(
            peer_id,
            &format!(
                "Введите номер подписки для удаления:\n{}",
                format_channel_subscriptions_list(&subscriptions)
            ),
        )
        .await
    }

    async fn delete_channel_subscription_from_input(
        &self,
        peer_id: i64,
        user_id: i64,
        input: &str,
    ) -> Result<()> {
        let input = input.trim();
        if input.eq_ignore_ascii_case("назад") || input == "/cancel" {
            self.state_store
                .set_state(UserId::new(user_id), DialogState::Idle)
                .await?;
            return self.show_channel_subscriptions(peer_id, user_id).await;
        }

        let Ok(sub_num) = input.parse::<i32>() else {
            return self
                .send_text(peer_id, "Введите номер подписки из списка.")
                .await;
        };
        if sub_num <= 0 {
            return self
                .send_text(peer_id, "Введите номер подписки из списка.")
                .await;
        }

        let deleted = DeleteExternalChannelSubscriptionUseCase::new(&self.store)
            .execute(DeleteExternalChannelSubscriptionCommand {
                user_id: UserId::new(user_id),
                sub_num,
            })
            .await?;
        self.state_store
            .set_state(UserId::new(user_id), DialogState::Idle)
            .await?;

        match deleted {
            Some(subscription) => {
                self.send_text(
                    peer_id,
                    &format!("Подписка удалена: {}", subscription.channel_name),
                )
                .await?;
                self.show_channel_subscriptions(peer_id, user_id).await
            }
            None => {
                self.send_text(peer_id, "Подписка с таким номером не найдена.")
                    .await
            }
        }
    }

    async fn channel_subscriptions_for_user(
        &self,
        user_id: i64,
    ) -> Result<Vec<ExternalChannelSubscription>> {
        Ok(ListExternalChannelSubscriptionsUseCase::new(&self.store)
            .execute(UserId::new(user_id))
            .await?)
    }

    async fn snooze_reminder(&self, peer_id: i64, reminder_id: i32, code: &str) -> Result<()> {
        let Some(minutes) = snooze_code_to_minutes(code) else {
            return self
                .send_text(peer_id, "Не смог распознать интервал откладывания.")
                .await;
        };

        let reminder = match SnoozeReminderUseCase::new(&self.store, &self.clock)
            .execute(ReminderId::new(reminder_id), minutes)
            .await
        {
            Ok(reminder) => reminder,
            Err(ApplicationError::NotFound { .. }) => {
                return self.send_text(peer_id, "Напоминание не найдено.").await;
            }
            Err(error) => return Err(error.into()),
        };

        self.send_text(
            peer_id,
            &format!(
                "Напоминание отложено на {}.\nНовое время: {}",
                snooze_code_to_label(code),
                reminder.next_at.format("%d.%m.%Y %H:%M UTC")
            ),
        )
        .await
    }

    async fn complete_reminder(&self, peer_id: i64, reminder_id: i32) -> Result<()> {
        let reminder =
            match CompleteReminderUseCase::new(&self.store, &self.store, &self.store, &self.clock)
                .execute(ReminderActionCommand {
                    reminder_id: ReminderId::new(reminder_id),
                    chat_id: ChatId::new(peer_id),
                })
                .await
            {
                Ok(reminder) => reminder,
                Err(ApplicationError::NotFound { .. }) => {
                    return self.send_text(peer_id, "Напоминание не найдено.").await;
                }
                Err(ApplicationError::Conflict(_)) => {
                    return self.send_text(peer_id, "Напоминание уже закрыто.").await;
                }
                Err(error) => return Err(error.into()),
            };

        let text = if reminder.status == ReminderStatus::Sent {
            "Напоминание выполнено.".to_string()
        } else {
            format!(
                "Напоминание отмечено. Следующее сработает: {}",
                reminder.next_at.format("%d.%m.%Y %H:%M UTC")
            )
        };
        self.send_text(peer_id, &text).await
    }

    async fn start_reminder_deletion(&self, peer_id: i64, user_id: i64) -> Result<()> {
        let reminders = self.active_reminders_for_chat(peer_id).await?;
        if reminders.is_empty() {
            self.state_store
                .set_state(UserId::new(user_id), DialogState::Idle)
                .await?;
            return self
                .send_text(peer_id, "Активных напоминаний для удаления нет.")
                .await;
        }

        self.state_store
            .set_state(UserId::new(user_id), DialogState::AwaitingReminderDeletion)
            .await?;
        self.send_text(
            peer_id,
            &format!(
                "Введите номер напоминания для удаления:\n{}",
                format_active_reminders(&reminders)
            ),
        )
        .await
    }

    async fn delete_reminder_from_input(
        &self,
        peer_id: i64,
        user_id: i64,
        input: &str,
    ) -> Result<()> {
        let Ok(number) = input.trim().parse::<usize>() else {
            return self
                .send_text(peer_id, "Введите номер напоминания из списка.")
                .await;
        };
        if number == 0 {
            return self
                .send_text(peer_id, "Введите номер напоминания из списка.")
                .await;
        }

        let reminders = self.active_reminders_for_chat(peer_id).await?;
        let Some(reminder) = reminders.get(number - 1) else {
            return self
                .send_text(peer_id, "Напоминания с таким номером нет.")
                .await;
        };
        let Some(reminder_id) = reminder.id else {
            return self
                .send_text(peer_id, "Не смог определить ID напоминания.")
                .await;
        };

        let cancelled = CancelReminderUseCase::new(&self.store, &self.store, &self.clock)
            .execute(ReminderActionCommand {
                reminder_id,
                chat_id: ChatId::new(peer_id),
            })
            .await?;
        self.state_store
            .set_state(UserId::new(user_id), DialogState::Idle)
            .await?;
        self.send_text(peer_id, &format!("Напоминание удалено: {}", cancelled.text))
            .await
    }

    async fn active_reminders_for_chat(&self, peer_id: i64) -> Result<Vec<Reminder>> {
        let mut reminders = ListActiveRemindersUseCase::new(&self.store)
            .execute(ChatId::new(peer_id))
            .await?;
        reminders.sort_by(compare_reminders_for_list);
        Ok(reminders)
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

fn parse_snooze_buttons(input: &str) -> Vec<SnoozeDuration> {
    let mut buttons = Vec::new();
    for minutes in input
        .split(|ch: char| !ch.is_ascii_digit())
        .filter_map(|part| part.parse::<u32>().ok())
        .filter(|minutes| *minutes > 0)
    {
        let duration = SnoozeDuration::from_minutes(minutes);
        if !buttons.contains(&duration) {
            buttons.push(duration);
        }
    }
    buttons
}

fn parse_first_minutes(input: &str) -> Option<u32> {
    input
        .split(|ch: char| !ch.is_ascii_digit())
        .find_map(|part| part.parse::<u32>().ok())
        .filter(|minutes| *minutes > 0)
}

fn format_snooze_buttons(buttons: &[SnoozeDuration]) -> String {
    buttons
        .iter()
        .map(|button| format!("{} мин", button.minutes()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn channel_platform(platform: ChannelPlatform) -> Platform {
    match platform {
        ChannelPlatform::Twitch => Platform::Twitch,
        ChannelPlatform::Youtube => Platform::Youtube,
    }
}

fn compare_tasks_for_list(left: &Task, right: &Task) -> Ordering {
    compare_optional_due(left, right).then_with(|| left.created_at.cmp(&right.created_at))
}

fn compare_reminders_for_list(left: &Reminder, right: &Reminder) -> Ordering {
    left.next_at
        .cmp(&right.next_at)
        .then_with(|| left.text.cmp(&right.text))
}

fn compare_optional_due(left: &Task, right: &Task) -> Ordering {
    match (left.due_at.as_ref(), right.due_at.as_ref()) {
        (Some(left), Some(right)) => left.cmp(right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn format_active_tasks(tasks: &[Task]) -> String {
    let mut text = String::from("Активные задачи:\n");
    for (index, task) in tasks.iter().enumerate() {
        let due_at = task
            .due_at
            .map(|due_at| due_at.format("%d.%m.%Y %H:%M UTC").to_string())
            .unwrap_or_else(|| "без срока".to_string());
        text.push_str(&format!("{}. {} - {}\n", index + 1, task.title, due_at));
    }
    text
}

fn format_active_reminders(reminders: &[Reminder]) -> String {
    let mut text = String::new();
    for (index, reminder) in reminders.iter().enumerate() {
        text.push_str(&format!(
            "{}. {} - {}\n",
            index + 1,
            reminder.text,
            reminder.next_at.format("%d.%m.%Y %H:%M UTC")
        ));
    }
    text
}

fn format_channel_subscriptions(subscriptions: &[ExternalChannelSubscription]) -> String {
    let intro =
        "Отправьте ссылку Twitch или YouTube, и я буду уведомлять о новых видео и трансляциях.";
    if subscriptions.is_empty() {
        format!("{}\n\nВаши подписки: пока нет.", intro)
    } else {
        format!(
            "{}\n\nВаши подписки:\n{}",
            intro,
            format_channel_subscriptions_list(subscriptions)
        )
    }
}

fn format_channel_subscriptions_list(subscriptions: &[ExternalChannelSubscription]) -> String {
    let mut text = String::new();
    for subscription in subscriptions {
        text.push_str(&format!(
            "{}. [{}] {} - {}\n",
            subscription.sub_num,
            subscription.platform,
            subscription.channel_name,
            subscription.url
        ));
    }
    text
}
