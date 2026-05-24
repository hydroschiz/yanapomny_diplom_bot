use std::cmp::Ordering;

use anyhow::{Context, Result};
use application::{
    ApplicationError, CancelReminderUseCase, CheckSubscriptionPaymentUseCase, Clock,
    CompleteReminderUseCase, CreateReminderFromPreviewCommand, CreateReminderFromPreviewUseCase,
    CreateSubscriptionPaymentCommand, CreateSubscriptionPaymentUseCase,
    DeleteExternalChannelSubscriptionCommand, DeleteExternalChannelSubscriptionUseCase,
    DialogState, DialogStateStore, EnsureSubscriptionUseCase, EnsureUserUseCase, GetProfileUseCase,
    InterpretedTask, ListActiveRemindersUseCase, ListExternalChannelSubscriptionsUseCase,
    PreviewReminderFromTextCommand, PreviewReminderFromTextUseCase, ProfileView,
    ReminderActionCommand, ReminderRepository, SaveExternalChannelSubscriptionCommand,
    SaveExternalChannelSubscriptionUseCase, SetAutoSnoozeUseCase, SetSnoozeButtonsUseCase,
    SetUserTimezoneUseCase, SnoozeReminderUseCase, SubscriptionRepository, UserRepository,
};
use async_trait::async_trait;
use chrono::{DateTime, Datelike, FixedOffset, Offset, TimeZone as ChronoTimeZone, Timelike, Utc};
use domain::{
    tariff_for_months, ChatId, ExternalChannelSubscription, Months, PaymentId, PaymentStatus,
    Platform, Reminder, ReminderId, ReminderStatus, SnoozeDuration, SubscriptionPolicy,
    SubscriptionStatus, TimePreferences, TimeZone, UserId,
};
use infrastructure::{
    HttpLlmInterpreter, HttpYooKassaPaymentGateway, MongoStore, RedisPaymentCache, SystemClock,
    UuidPaymentIdGenerator, YooKassaReceiptConfig,
};
use presentation::keyboard::{
    channel_subs_keyboard, delete_keyboard, list_delete_keyboard, pay_link_keyboard,
    pay_provider_keyboard, reminder_confirm_keyboard, reminder_edit_keyboard, snooze_code_to_label,
    snooze_code_to_minutes, text_confirm_keyboard,
};
use presentation::{
    CallbackRoute, ChannelPlatform, IncomingCallback, IncomingMessage, MessageRoute, Notification,
    ParsedChannelLink, Renderer, RouteContext, Router, TimezoneDisplay,
};
use tracing::{error, info};
use transport_core::BotTransport;
use transport_vk::{run_long_poll, VkEventHandler, VkIncomingEvent, VkTransport};
use webhook::spawn_yookassa_webhook_server;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();

    let config = BotConfig::from_env()?;
    let store = MongoStore::connect(&config.mongo_uri, &config.mongo_db).await?;
    let payment_cache = RedisPaymentCache::new(&config.redis_url)?;
    let payment_gateway = config.payment_gateway();
    let llm = HttpLlmInterpreter::new(config.llm_api_url.clone())?;
    let transport = VkTransport::new(config.vk_access_token.clone())?;

    if config.bot_webhook_enabled {
        if payment_gateway.is_some() {
            spawn_yookassa_webhook_server(
                &config.bind_ip,
                config.webhook_port,
                store.clone(),
                payment_cache.clone(),
                transport.clone(),
            )
            .await?;
        } else {
            info!("embedded YooKassa webhook server skipped because payment gateway is disabled");
        }
    } else {
        info!("embedded YooKassa webhook server disabled");
    }

    let handler = BotHandler {
        transport,
        store: store.clone(),
        llm,
        clock: SystemClock,
        payment_cache,
        payment_gateway,
        payment_ids: UuidPaymentIdGenerator,
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
    yk_receipt: Option<YooKassaReceiptConfig>,
    bind_ip: String,
    webhook_port: u16,
    bot_webhook_enabled: bool,
}

impl BotConfig {
    fn from_env() -> Result<Self> {
        let yk_receipt = build_yookassa_receipt_config()?;
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
            yk_receipt,
            bind_ip: env_or("IP", "0.0.0.0"),
            webhook_port: env_parse("PORT", 3001)?,
            bot_webhook_enabled: parse_bool_env("BOT_WEBHOOK_ENABLED").unwrap_or(true),
        })
    }

    fn payment_gateway(&self) -> Option<HttpYooKassaPaymentGateway> {
        Some(HttpYooKassaPaymentGateway::new(
            self.yk_shop_id.clone()?,
            self.yk_secret_key.clone()?,
            self.yk_return_url.clone(),
            self.yk_receipt.clone(),
        ))
    }
}

fn build_yookassa_receipt_config() -> Result<Option<YooKassaReceiptConfig>> {
    let Some(vat_code_str) = optional_env("YK_VAT_CODE") else {
        return Ok(None);
    };
    let Some(vat_code) = vat_code_str.parse::<u8>().ok() else {
        return Ok(None);
    };
    let payment_subject =
        optional_env("YK_PAYMENT_SUBJECT").unwrap_or_else(|| "service".to_string());
    let payment_mode =
        optional_env("YK_PAYMENT_MODE").unwrap_or_else(|| "full_payment".to_string());
    let tax_system_code = optional_env("YK_TAX_SYSTEM_CODE").and_then(|s| s.parse::<u8>().ok());
    let email_suffix =
        optional_env("YK_RECEIPT_EMAIL_SUFFIX").unwrap_or_else(|| "yanapomnyu.ru".to_string());

    Ok(Some(YooKassaReceiptConfig::new(
        vat_code,
        payment_subject,
        payment_mode,
        tax_system_code,
        email_suffix,
    )))
}

#[derive(Clone)]
struct BotHandler {
    transport: VkTransport,
    store: MongoStore,
    llm: HttpLlmInterpreter,
    clock: SystemClock,
    payment_cache: RedisPaymentCache,
    payment_gateway: Option<HttpYooKassaPaymentGateway>,
    payment_ids: UuidPaymentIdGenerator,
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
                let buttons = match parse_snooze_buttons(&input) {
                    Ok(buttons) => buttons,
                    Err(error_text) => {
                        return self.send_text(message.peer_id, error_text).await;
                    }
                };
                if buttons.is_empty() {
                    return self
                        .send_text(
                            message.peer_id,
                            "Не смог распознать время. Пример: 15 мин, 1 час, 2 дня.",
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
                let Some(minutes) = parse_auto_snooze_minutes(&input) else {
                    return self
                        .send_text(
                            message.peer_id,
                            "Не смог распознать время. Доступно: 5 мин, 10 мин, 15 мин, 20 мин.",
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
            MessageRoute::ShowPay => self.show_payment_menu(message.peer_id).await,
            MessageRoute::ShowProfile => self.show_profile(message.peer_id, message.user_id).await,
            MessageRoute::CreateReminderFromCommand(text) | MessageRoute::ReminderText(text) => {
                self.start_personal_reminder_text_confirmation(
                    message.peer_id,
                    message.user_id,
                    text,
                )
                .await
            }
            MessageRoute::GroupReminderText(text) => {
                self.start_reminder_text_confirmation(message.peer_id, message.user_id, text)
                    .await
            }
            MessageRoute::ListReminders => {
                self.show_active_reminders(message.peer_id, message.user_id)
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
            MessageRoute::ExistingReminderEditSelection(input) => {
                self.select_reminder_for_edit(message.peer_id, message.user_id, &input)
                    .await
            }
            MessageRoute::ExistingReminderEditText(input) => {
                self.update_existing_reminder_text(message.peer_id, message.user_id, &input)
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
            MessageRoute::ReminderEditText(input) => {
                self.handle_reminder_edit_text(message.peer_id, message.user_id, &input)
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
            CallbackRoute::ShowPayMenu => self.show_payment_menu(callback.peer_id).await,
            CallbackRoute::SelectPaymentPeriod(months) => {
                self.select_payment_period(callback.peer_id, months).await
            }
            CallbackRoute::StartYooKassaPayment(months) => {
                self.start_yookassa_payment(callback.peer_id, callback.user_id, months)
                    .await
            }
            CallbackRoute::CheckPayment(payment_id) => {
                self.check_payment(callback.peer_id, &payment_id).await
            }
            CallbackRoute::CancelPayment => {
                self.state_store
                    .set_state(UserId::new(callback.user_id), DialogState::Idle)
                    .await?;
                self.send_text(callback.peer_id, "Оплата отменена.").await
            }
            CallbackRoute::ShowProfile => {
                self.show_profile(callback.peer_id, callback.user_id).await
            }
            CallbackRoute::ListReminders => {
                self.show_active_reminders(callback.peer_id, callback.user_id)
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
            CallbackRoute::StartReminderEdit => {
                self.start_existing_reminder_edit(callback.peer_id, callback.user_id)
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
            CallbackRoute::BackFromReminderDeletion => {
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
            CallbackRoute::ShowReferral => {
                self.send_text(
                    callback.peer_id,
                    "Реферальные ссылки VK временно отключены.",
                )
                .await
            }
            CallbackRoute::ConfirmText => {
                self.preview_reminder_from_pending_text(callback.peer_id, callback.user_id)
                    .await
            }
            CallbackRoute::ConfirmReminder => {
                self.create_confirmed_reminder(callback.peer_id, callback.user_id)
                    .await
            }
            CallbackRoute::EditReminder => {
                self.start_reminder_edit(callback.peer_id, callback.user_id)
                    .await
            }
            CallbackRoute::CancelText | CallbackRoute::CancelReminder => {
                self.state_store
                    .set_state(UserId::new(callback.user_id), DialogState::Idle)
                    .await?;
                self.send_text(callback.peer_id, "Действие отменено.").await
            }
            CallbackRoute::Unknown(_) => {
                self.send_text(callback.peer_id, "Неизвестное действие.")
                    .await
            }
        }
    }

    async fn show_payment_menu(&self, peer_id: i64) -> Result<()> {
        let subscription = self.store.find_subscription(ChatId::new(peer_id)).await?;
        let now = self.clock.now();
        let is_active = subscription
            .as_ref()
            .is_some_and(|subscription| subscription.is_active(now));
        let expiry = subscription
            .as_ref()
            .filter(|subscription| subscription.is_active(now))
            .map(|subscription| subscription.expires_at.format("%d.%m.%Y").to_string());

        self.send_notification(peer_id, Notification::PayMenu { is_active, expiry })
            .await
    }

    async fn show_profile(&self, peer_id: i64, user_id: i64) -> Result<()> {
        let profile = GetProfileUseCase::new(&self.store, &self.store, &self.clock)
            .execute(UserId::new(user_id), ChatId::new(peer_id))
            .await?;
        self.send_notification(peer_id, profile_notification(profile))
            .await
    }

    async fn select_payment_period(&self, peer_id: i64, months: i32) -> Result<()> {
        let Some(tariff) = supported_tariff(months) else {
            return self.send_text(peer_id, "Такой тариф недоступен.").await;
        };
        let text = format!(
            "Вы выбрали подписку на {} мес.\nСтоимость: {}₽.\n\nВыберите способ оплаты:",
            tariff.months.value(),
            tariff.price.amount
        );

        self.send_notification(
            peer_id,
            Notification::PlainText {
                text,
                keyboard: Some(pay_provider_keyboard(
                    i32::from(tariff.months.value()),
                    self.transport.capabilities(),
                )),
            },
        )
        .await
    }

    async fn start_yookassa_payment(&self, peer_id: i64, user_id: i64, months: i32) -> Result<()> {
        let Some(tariff) = supported_tariff(months) else {
            return self.send_text(peer_id, "Такой тариф недоступен.").await;
        };
        let Some(gateway) = self.payment_gateway.as_ref() else {
            return self
                .send_notification(peer_id, Notification::PaymentDisabled)
                .await;
        };

        let user_id = UserId::new(user_id);
        EnsureUserUseCase::new(&self.store).execute(user_id).await?;
        let created = match CreateSubscriptionPaymentUseCase::new(
            &self.store,
            gateway,
            &self.payment_cache,
            &self.payment_ids,
            &self.clock,
        )
        .execute(CreateSubscriptionPaymentCommand {
            user_id,
            months: tariff.months,
        })
        .await
        {
            Ok(created) => created,
            Err(ApplicationError::Conflict(_)) => {
                return self.send_text(peer_id, "Такой тариф недоступен.").await;
            }
            Err(error) => {
                error!(%error, "failed to create YooKassa payment");
                return self
                    .send_text(peer_id, "Не удалось создать платёж. Попробуйте позже.")
                    .await;
            }
        };

        let text = format!(
            "Платёж на {} мес. создан.\nСумма: {}₽.\n\nНажмите кнопку оплаты, затем вернитесь и нажмите «Проверить оплату».",
            tariff.months.value(),
            tariff.price.amount
        );

        self.send_notification(
            peer_id,
            Notification::PlainText {
                text,
                keyboard: Some(pay_link_keyboard(
                    &created.confirmation_url,
                    created.transaction.payment_id.as_str(),
                    self.transport.capabilities(),
                )),
            },
        )
        .await
    }

    async fn check_payment(&self, peer_id: i64, payment_id: &str) -> Result<()> {
        let payment_id = payment_id.trim();
        if payment_id.is_empty() {
            return self
                .send_text(peer_id, "Не смог определить платёж для проверки.")
                .await;
        }
        let payment_id = PaymentId::new(payment_id);
        tracing::info!(payment_id = %payment_id, "Manual payment check requested");
        let status = match CheckSubscriptionPaymentUseCase::new(
            &self.store,
            self.payment_gateway.as_ref(),
            &self.store,
            &self.clock,
        )
        .execute(&payment_id)
        .await
        {
            Ok(status) => status,
            Err(ApplicationError::NotFound { .. }) => {
                return self
                    .send_text(peer_id, "Платёж не найден. Оформите новый платёж.")
                    .await;
            }
            Err(error) => return Err(error.into()),
        };

        let text = match &status.transaction.status {
            PaymentStatus::Succeeded => {
                if status.transaction.fulfilled || status.subscription.is_some() {
                    status
                        .subscription
                        .as_ref()
                        .map(|subscription| {
                            format!(
                                "✅ Платёж подтверждён. Подписка активна до {}.",
                                subscription.expires_at.format("%d.%m.%Y")
                            )
                        })
                        .unwrap_or_else(|| {
                            "✅ Платёж подтверждён. Подписка активирована.".to_string()
                        })
                } else {
                    "✅ Платёж подтверждён. Подписка скоро будет активирована.".to_string()
                }
            }
            PaymentStatus::Pending | PaymentStatus::WaitingForCapture => {
                "⏳ Платёж ещё обрабатывается. Попробуйте проверить позже.".to_string()
            }
            PaymentStatus::Canceled => "❌ Платёж отменён. Оформите новый платёж.".to_string(),
            PaymentStatus::Failed => "❌ Платёж не прошёл. Оформите новый платёж.".to_string(),
            PaymentStatus::Unknown(status) => format!(
                "Статус платежа: {}. Подождите или попробуйте позже.",
                status
            ),
        };

        self.send_text(peer_id, &text).await
    }

    async fn update_timezone(&self, peer_id: i64, user_id: i64, offset: &str) -> Result<()> {
        let Some((preferences, display_offset)) = parse_timezone_preferences(offset) else {
            return self
                .send_text(
                    peer_id,
                    "Не смог распознать часовой пояс. Напишите город, IANA timezone вроде Europe/Moscow или UTC offset вроде +07:00.",
                )
                .await;
        };
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

    async fn start_reminder_text_confirmation(
        &self,
        peer_id: i64,
        user_id: i64,
        text: String,
    ) -> Result<()> {
        let confirmation_text = format!(
            "📝 <b>Создать напоминание из этого текста?</b>\n\n<i>{}</i>",
            html_escape(&text)
        );
        self.send_notification(
            peer_id,
            Notification::PlainText {
                text: confirmation_text,
                keyboard: Some(text_confirm_keyboard(self.transport.capabilities())),
            },
        )
        .await?;
        self.state_store
            .set_state(
                UserId::new(user_id),
                DialogState::AwaitingTextConfirmation { text },
            )
            .await?;
        Ok(())
    }

    async fn start_personal_reminder_text_confirmation(
        &self,
        peer_id: i64,
        user_id: i64,
        text: String,
    ) -> Result<()> {
        let user_id = UserId::new(user_id);
        let Some(user) = self.store.find_user(user_id).await? else {
            EnsureUserUseCase::new(&self.store).execute(user_id).await?;
            return self
                .send_text(
                    peer_id,
                    "Сначала настройте часовой пояс: /utc или кнопка «Время суток (UTC)» в настройках.",
                )
                .await;
        };
        if matches!(user.time_preferences.time_zone, TimeZone::Utc) {
            return self
                .send_text(
                    peer_id,
                    "Сначала настройте часовой пояс: /utc или кнопка «Время суток (UTC)» в настройках.",
                )
                .await;
        }

        let subscription = self.store.find_subscription(ChatId::new(peer_id)).await?;
        if !subscription.as_ref().is_some_and(|subscription| {
            subscription.active && subscription.is_active(self.clock.now())
        }) {
            return self
                .send_text(
                    peer_id,
                    "Для создания напоминаний нужна активная подписка. Откройте /pay или профиль.",
                )
                .await;
        }

        self.start_reminder_text_confirmation(peer_id, user_id.value(), text)
            .await
    }

    async fn preview_reminder_from_pending_text(&self, peer_id: i64, user_id: i64) -> Result<()> {
        let state = self.state_store.get_state(UserId::new(user_id)).await?;
        let DialogState::AwaitingTextConfirmation { text } = state else {
            return self
                .send_text(peer_id, "Нет текста напоминания для подтверждения.")
                .await;
        };

        self.send_text(peer_id, "Анализирую текст напоминания...")
            .await?;
        let preview = match PreviewReminderFromTextUseCase::new(&self.store, &self.llm)
            .execute(PreviewReminderFromTextCommand {
                user_id: UserId::new(user_id),
                text,
            })
            .await
        {
            Ok(preview) => preview,
            Err(error) => {
                error!(%error, "failed to preview reminder text");
                self.state_store
                    .set_state(UserId::new(user_id), DialogState::Idle)
                    .await?;
                return self
                    .send_text(
                        peer_id,
                        "Не удалось распознать напоминание. Попробуйте ещё раз.",
                    )
                    .await;
            }
        };

        let offset = self.user_fixed_offset(user_id).await?;
        self.send_notification(
            peer_id,
            Notification::PlainText {
                text: format_reminder_preview(&preview.interpreted, offset),
                keyboard: Some(reminder_confirm_keyboard(self.transport.capabilities())),
            },
        )
        .await?;
        self.state_store
            .set_state(
                UserId::new(user_id),
                DialogState::AwaitingReminderConfirmation {
                    original_text: preview.original_text,
                    interpreted: preview.interpreted,
                },
            )
            .await?;
        Ok(())
    }

    async fn create_confirmed_reminder(&self, peer_id: i64, user_id: i64) -> Result<()> {
        let state = self.state_store.get_state(UserId::new(user_id)).await?;
        let DialogState::AwaitingReminderConfirmation { interpreted, .. } = state else {
            return self
                .send_text(peer_id, "Нет напоминания для подтверждения.")
                .await;
        };

        let created = CreateReminderFromPreviewUseCase::new(&self.store, &self.store, &self.clock)
            .execute(CreateReminderFromPreviewCommand {
                user_id: UserId::new(user_id),
                chat_id: ChatId::new(peer_id),
                interpreted,
            })
            .await?;
        self.state_store
            .set_state(UserId::new(user_id), DialogState::Idle)
            .await?;

        let reminder = created.reminder;
        let offset = self.user_fixed_offset(user_id).await?;
        self.send_notification(
            peer_id,
            Notification::PlainText {
                text: format!(
                    "✅ <b>Напоминание создано!</b>\n\n📌 {}\n🕐 {}\n🆔 #{}",
                    html_escape(&reminder.text),
                    format_local_datetime(reminder.next_at, offset),
                    reminder.id.map(|id| id.value()).unwrap_or_default()
                ),
                keyboard: None,
            },
        )
        .await
    }

    async fn start_reminder_edit(&self, peer_id: i64, user_id: i64) -> Result<()> {
        let state = self.state_store.get_state(UserId::new(user_id)).await?;
        let DialogState::AwaitingReminderConfirmation {
            original_text,
            interpreted,
        } = state
        else {
            return self
                .send_text(peer_id, "Нет напоминания для редактирования.")
                .await;
        };
        let text = format!(
            "✏️ Введите новый текст напоминания:\n\n<i>Текущий:</i> {}",
            html_escape(&original_text)
        );
        self.send_notification(
            peer_id,
            Notification::PlainText {
                text,
                keyboard: Some(reminder_edit_keyboard(self.transport.capabilities())),
            },
        )
        .await?;
        self.state_store
            .set_state(
                UserId::new(user_id),
                DialogState::AwaitingReminderEdit {
                    original_text,
                    interpreted,
                },
            )
            .await?;
        Ok(())
    }

    async fn handle_reminder_edit_text(
        &self,
        peer_id: i64,
        user_id: i64,
        text: &str,
    ) -> Result<()> {
        let text = text.trim();
        if text.is_empty() {
            return self
                .send_text(peer_id, "Текст не может быть пустым. Введите новый текст.")
                .await;
        }
        self.start_reminder_text_confirmation(peer_id, user_id, text.to_string())
            .await
    }

    async fn show_active_reminders(&self, peer_id: i64, user_id: i64) -> Result<()> {
        let reminders = self.active_reminders_for_chat(peer_id).await?;
        if reminders.is_empty() {
            return self
                .send_text(peer_id, "Активных напоминаний пока нет.")
                .await;
        }

        let offset = self.user_fixed_offset(user_id).await?;
        self.send_notification(
            peer_id,
            Notification::PlainText {
                text: format_active_reminders(&reminders, offset),
                keyboard: Some(list_delete_keyboard(self.transport.capabilities())),
            },
        )
        .await
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

        let offset = self.chat_fixed_offset(peer_id).await?;
        self.send_text(
            peer_id,
            &format!(
                "Напоминание отложено на {}.\nНовое время: {}",
                snooze_code_to_label(code),
                format_local_datetime(reminder.next_at, offset)
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
            let offset = self.chat_fixed_offset(peer_id).await?;
            format!(
                "Напоминание отмечено. Следующее сработает: {}",
                format_local_datetime(reminder.next_at, offset)
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
        self.send_notification(
            peer_id,
            Notification::PlainText {
                text: format!(
                    "Введите номер напоминания для удаления:\n{}",
                    format_active_reminders(&reminders, self.user_fixed_offset(user_id).await?)
                ),
                keyboard: Some(delete_keyboard(self.transport.capabilities())),
            },
        )
        .await
    }

    async fn start_existing_reminder_edit(&self, peer_id: i64, user_id: i64) -> Result<()> {
        let reminders = self.active_reminders_for_chat(peer_id).await?;
        if reminders.is_empty() {
            self.state_store
                .set_state(UserId::new(user_id), DialogState::Idle)
                .await?;
            return self
                .send_text(peer_id, "Активных напоминаний для изменения нет.")
                .await;
        }

        self.state_store
            .set_state(
                UserId::new(user_id),
                DialogState::AwaitingExistingReminderEditSelection,
            )
            .await?;
        self.send_notification(
            peer_id,
            Notification::PlainText {
                text: format!(
                    "Введите номер напоминания для изменения:\n{}",
                    format_active_reminders(&reminders, self.user_fixed_offset(user_id).await?)
                ),
                keyboard: Some(delete_keyboard(self.transport.capabilities())),
            },
        )
        .await
    }

    async fn select_reminder_for_edit(
        &self,
        peer_id: i64,
        user_id: i64,
        input: &str,
    ) -> Result<()> {
        let Some(reminder) = self.reminder_by_list_number(peer_id, input).await? else {
            return self
                .send_text(peer_id, "Введите номер напоминания из списка.")
                .await;
        };
        let Some(reminder_id) = reminder.id else {
            return self
                .send_text(peer_id, "Не смог определить ID напоминания.")
                .await;
        };

        self.state_store
            .set_state(
                UserId::new(user_id),
                DialogState::AwaitingExistingReminderText { reminder_id },
            )
            .await?;
        self.send_notification(
            peer_id,
            Notification::PlainText {
                text: format!(
                    "✏️ Введите новый текст для напоминания:\n\n<i>Текущий:</i> {}",
                    html_escape(&reminder.text)
                ),
                keyboard: Some(reminder_edit_keyboard(self.transport.capabilities())),
            },
        )
        .await
    }

    async fn update_existing_reminder_text(
        &self,
        peer_id: i64,
        user_id: i64,
        input: &str,
    ) -> Result<()> {
        let input = input.trim();
        if input.is_empty() {
            return self
                .send_text(peer_id, "Текст не может быть пустым. Введите новый текст.")
                .await;
        }
        let state = self.state_store.get_state(UserId::new(user_id)).await?;
        let DialogState::AwaitingExistingReminderText { reminder_id } = state else {
            return self
                .send_text(peer_id, "Нет выбранного напоминания для изменения.")
                .await;
        };
        let Some(mut reminder) = self.store.find_reminder(reminder_id).await? else {
            self.state_store
                .set_state(UserId::new(user_id), DialogState::Idle)
                .await?;
            return self.send_text(peer_id, "Напоминание не найдено.").await;
        };
        if reminder.chat_id != ChatId::new(peer_id) || reminder.status.is_terminal() {
            self.state_store
                .set_state(UserId::new(user_id), DialogState::Idle)
                .await?;
            return self.send_text(peer_id, "Напоминание уже закрыто.").await;
        }

        reminder.text = input.to_string();
        self.store.save_reminder(&reminder).await?;
        self.state_store
            .set_state(UserId::new(user_id), DialogState::Idle)
            .await?;
        self.send_notification(
            peer_id,
            Notification::PlainText {
                text: format!(
                    "✅ Напоминание изменено.\n\n{}",
                    format_active_reminders(&[reminder], self.user_fixed_offset(user_id).await?)
                ),
                keyboard: Some(list_delete_keyboard(self.transport.capabilities())),
            },
        )
        .await
    }

    async fn delete_reminder_from_input(
        &self,
        peer_id: i64,
        user_id: i64,
        input: &str,
    ) -> Result<()> {
        let Some(reminder) = self.reminder_by_list_number(peer_id, input).await? else {
            return self
                .send_text(peer_id, "Введите номер напоминания из списка.")
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

    async fn reminder_by_list_number(&self, peer_id: i64, input: &str) -> Result<Option<Reminder>> {
        let Ok(number) = input.trim().parse::<usize>() else {
            return Ok(None);
        };
        if number == 0 {
            return Ok(None);
        }
        Ok(self
            .active_reminders_for_chat(peer_id)
            .await?
            .get(number - 1)
            .cloned())
    }

    async fn user_fixed_offset(&self, user_id: i64) -> Result<FixedOffset> {
        Ok(self
            .store
            .find_user(UserId::new(user_id))
            .await?
            .map(|user| user.time_preferences.fixed_offset())
            .unwrap_or_else(|| TimePreferences::default().fixed_offset()))
    }

    async fn chat_fixed_offset(&self, peer_id: i64) -> Result<FixedOffset> {
        self.user_fixed_offset(peer_id).await
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

fn parse_bool_env(name: &str) -> Option<bool> {
    let value = optional_env(name)?;
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
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

fn parse_snooze_buttons(input: &str) -> Result<Vec<SnoozeDuration>, &'static str> {
    let mut buttons = Vec::new();
    for minutes in parse_duration_minutes(input) {
        if !ALLOWED_SNOOZE_MINUTES.contains(&minutes) {
            return Err("Такой интервал недоступен. Доступно: 5, 10, 15, 20, 30 мин; 1-4 часа; 1, 2, 3, 7 дней.");
        }
        let duration = SnoozeDuration::from_minutes(minutes);
        if !buttons.contains(&duration) {
            buttons.push(duration);
        }
    }
    Ok(buttons)
}

fn parse_auto_snooze_minutes(input: &str) -> Option<u32> {
    let mut durations = parse_duration_minutes(input);
    let minutes = durations.pop()?;
    if durations.is_empty() && ALLOWED_AUTO_SNOOZE_MINUTES.contains(&minutes) {
        Some(minutes)
    } else {
        None
    }
}

fn format_snooze_buttons(buttons: &[SnoozeDuration]) -> String {
    buttons
        .iter()
        .map(|button| format_duration_minutes(button.minutes()))
        .collect::<Vec<_>>()
        .join(", ")
}

const ALLOWED_SNOOZE_MINUTES: &[u32] = &[
    5, 10, 15, 20, 30, 60, 120, 180, 240, 1440, 2880, 4320, 10080,
];
const ALLOWED_AUTO_SNOOZE_MINUTES: &[u32] = &[5, 10, 15, 20];

fn parse_duration_minutes(input: &str) -> Vec<u32> {
    let normalized = input.to_lowercase().replace([',', ';', '\n'], " ");
    let parts = normalized.split_whitespace().collect::<Vec<_>>();
    let mut durations = Vec::new();
    let mut index = 0;

    while index < parts.len() {
        let Ok(value) = parts[index].parse::<u32>() else {
            index += 1;
            continue;
        };
        let unit = parts.get(index + 1).copied().unwrap_or("мин");
        let (multiplier, consumed_unit) = if unit.starts_with("час") || unit.starts_with("ч") {
            (60, true)
        } else if unit.starts_with("ден") || unit.starts_with("дн") || unit.starts_with("сут")
        {
            (1440, true)
        } else {
            (1, unit.starts_with("мин") || unit.starts_with("м"))
        };
        durations.push(value.saturating_mul(multiplier));
        index += if consumed_unit { 2 } else { 1 };
    }

    durations
}

fn format_duration_minutes(minutes: u32) -> String {
    if minutes.is_multiple_of(1440) {
        let days = minutes / 1440;
        format!("{} {}", days, plural(days, "день", "дня", "дней"))
    } else if minutes.is_multiple_of(60) {
        let hours = minutes / 60;
        format!("{} {}", hours, plural(hours, "час", "часа", "часов"))
    } else {
        format!("{} мин", minutes)
    }
}

fn parse_timezone_preferences(input: &str) -> Option<(TimePreferences, String)> {
    if let Ok(preferences) =
        TimePreferences::from_fixed_offset_strings("08:00", "14:00", "19:00", input)
    {
        let display = preferences.utc_offset.to_string();
        return Some((preferences, display));
    }

    let tz_name = resolve_timezone(input)?;
    let tz: chrono_tz::Tz = tz_name.parse().ok()?;
    let offset = tz
        .offset_from_utc_datetime(&Utc::now().naive_utc())
        .fix()
        .local_minus_utc();
    let offset_string = offset_seconds_to_string(offset);
    let mut preferences =
        TimePreferences::from_fixed_offset_strings("08:00", "14:00", "19:00", &offset_string)
            .ok()?;
    preferences.time_zone = TimeZone::Iana(tz_name);
    Some((preferences, offset_string))
}

fn resolve_timezone(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.contains('/') {
        return trimmed
            .replace(' ', "")
            .parse::<chrono_tz::Tz>()
            .ok()
            .map(|_| trimmed.replace(' ', ""));
    }

    let lower = trimmed.to_lowercase();
    CITY_MAP
        .iter()
        .find(|(city, _)| *city == lower)
        .map(|(_, timezone)| (*timezone).to_string())
}

const CITY_MAP: &[(&str, &str)] = &[
    ("москва", "Europe/Moscow"),
    ("moscow", "Europe/Moscow"),
    ("питер", "Europe/Moscow"),
    ("spb", "Europe/Moscow"),
    ("санкт-петербург", "Europe/Moscow"),
    ("yekaterinburg", "Asia/Yekaterinburg"),
    ("екатеринбург", "Asia/Yekaterinburg"),
    ("novosibirsk", "Asia/Novosibirsk"),
    ("новосибирск", "Asia/Novosibirsk"),
    ("krasnoyarsk", "Asia/Krasnoyarsk"),
    ("красноярск", "Asia/Krasnoyarsk"),
    ("kazan", "Europe/Moscow"),
    ("казань", "Europe/Moscow"),
    ("omsk", "Asia/Omsk"),
    ("omsk city", "Asia/Omsk"),
    ("омск", "Asia/Omsk"),
    ("vladivostok", "Asia/Vladivostok"),
    ("владивосток", "Asia/Vladivostok"),
    ("irkutsk", "Asia/Irkutsk"),
    ("иркутск", "Asia/Irkutsk"),
    ("samara", "Europe/Samara"),
    ("самара", "Europe/Samara"),
];

fn offset_seconds_to_string(seconds: i32) -> String {
    let sign = if seconds < 0 { '-' } else { '+' };
    let total = seconds.abs();
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    format!("{sign}{hours:02}:{minutes:02}")
}

fn format_local_datetime(value: DateTime<Utc>, offset: FixedOffset) -> String {
    let local = value.with_timezone(&offset);
    format!(
        "{}, {:02}:{:02} {}",
        local.format("%d.%m.%Y"),
        local.hour(),
        local.minute(),
        offset_seconds_to_string(offset.local_minus_utc())
    )
}

fn russian_month_name(month: u32) -> &'static str {
    match month {
        1 => "Январь",
        2 => "Февраль",
        3 => "Март",
        4 => "Апрель",
        5 => "Май",
        6 => "Июнь",
        7 => "Июль",
        8 => "Август",
        9 => "Сентябрь",
        10 => "Октябрь",
        11 => "Ноябрь",
        12 => "Декабрь",
        _ => "",
    }
}

fn russian_weekday_short(day: chrono::Weekday) -> &'static str {
    match day {
        chrono::Weekday::Mon => "пн",
        chrono::Weekday::Tue => "вт",
        chrono::Weekday::Wed => "ср",
        chrono::Weekday::Thu => "чт",
        chrono::Weekday::Fri => "пт",
        chrono::Weekday::Sat => "сб",
        chrono::Weekday::Sun => "вс",
    }
}

fn plural(value: u32, one: &'static str, few: &'static str, many: &'static str) -> &'static str {
    match value % 100 {
        11..=14 => many,
        _ => match value % 10 {
            1 => one,
            2..=4 => few,
            _ => many,
        },
    }
}

fn channel_platform(platform: ChannelPlatform) -> Platform {
    match platform {
        ChannelPlatform::Twitch => Platform::Twitch,
        ChannelPlatform::Youtube => Platform::Youtube,
    }
}

fn supported_tariff(months: i32) -> Option<domain::Tariff> {
    let months = Months::new(months).ok()?;
    tariff_for_months(months).copied()
}

fn profile_notification(profile: ProfileView) -> Notification {
    let ProfileView {
        user,
        subscription_status,
    } = profile;
    Notification::Profile {
        user_id: user.id.value(),
        utc_offset: user.time_preferences.utc_offset.to_string(),
        snooze_buttons: format_profile_snooze_buttons(&user.snooze_buttons),
        auto_snooze: format!("{} мин", user.auto_snooze.minutes()),
        subscription: format_profile_subscription(subscription_status),
    }
}

fn format_profile_snooze_buttons(buttons: &[SnoozeDuration]) -> String {
    if buttons.is_empty() {
        "не настроены".to_string()
    } else {
        format_snooze_buttons(buttons)
    }
}

fn format_profile_subscription(status: Option<SubscriptionStatus>) -> String {
    match status {
        Some(SubscriptionStatus::Trial { until }) => {
            format!("пробный период до {}", until.format("%d.%m.%Y"))
        }
        Some(SubscriptionStatus::Active { until }) => {
            format!("активна до {}", until.format("%d.%m.%Y"))
        }
        Some(SubscriptionStatus::Expired) => "не активна".to_string(),
        None => "не оформлена".to_string(),
    }
}

fn format_reminder_preview(interpreted: &InterpretedTask, offset: FixedOffset) -> String {
    format!(
        "📝 <b>Подтвердите напоминание:</b>\n\n\
         📌 <b>Текст:</b> {}\n\
         🕐 <b>Время:</b> {}\n\
         🔄 <b>Тип:</b> {}\n\n\
         Подтвердите создание или отредактируйте текст.",
        html_escape(&interpreted.title),
        format_local_datetime(interpreted.trigger_at, offset),
        reminder_type_label(interpreted)
    )
}

fn reminder_type_label(interpreted: &InterpretedTask) -> &'static str {
    match &interpreted.schedule {
        domain::Schedule::OneTime(_) => "разовое",
        domain::Schedule::Recurring { .. } => "повторяющееся",
    }
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn compare_reminders_for_list(left: &Reminder, right: &Reminder) -> Ordering {
    left.next_at
        .cmp(&right.next_at)
        .then_with(|| left.text.cmp(&right.text))
}

fn format_active_reminders(reminders: &[Reminder], offset: FixedOffset) -> String {
    let mut text = String::from("📋 Активные напоминания:\n");
    let mut current_month: Option<(i32, u32)> = None;
    for (index, reminder) in reminders.iter().enumerate() {
        let local = reminder.next_at.with_timezone(&offset);
        let month = (local.year(), local.month());
        if current_month != Some(month) {
            current_month = Some(month);
            text.push_str(&format!(
                "\n<b>{} {}</b>\n",
                russian_month_name(local.month()),
                local.year()
            ));
        }
        text.push_str(&format!(
            "{}. {} — {}, {:02}:{:02}\n   {}\n",
            index + 1,
            html_escape(&reminder.text),
            russian_weekday_short(local.weekday()),
            local.hour(),
            local.minute(),
            local.format("%d.%m.%Y")
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
