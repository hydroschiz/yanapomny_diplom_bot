use std::sync::Arc;

use async_trait::async_trait;
#[cfg(feature = "telegram-legacy")]
use teloxide::dispatching::dialogue::InMemStorage;
#[cfg(feature = "telegram-legacy")]
use teloxide::dispatching::UpdateHandler;
#[cfg(feature = "telegram-legacy")]
use teloxide::prelude::*;
use vk_bot_api::api::VkApi;
use vk_bot_api::error::{VkError, VkResult};
use vk_bot_api::handler::MessageHandler;
use vk_bot_api::models::{Event, Message as VkMessage, MessageEvent};

use crate::api::db::Db;
use crate::api::payments::PaymentService;
use crate::bot::keyboards::{back_keyboard, setup_keyboard};
use crate::bot::states::AppState;
use crate::config::Config;
use crate::transport::dialogue_store::DialogueStore;
use crate::transport::text_format::strip_html;
use crate::transport::traits::{BotTransport, TransportKeyboard};

use super::handlers;

#[cfg(feature = "telegram-legacy")]
pub type AppDialogue = Dialogue<AppState, InMemStorage<AppState>>;
pub type HandlerResult = Result<(), anyhow::Error>;

/// VK long-poll handler that routes raw VK events to platform-agnostic handlers.
#[derive(Clone)]
pub struct AppHandler<T: BotTransport> {
    transport: T,
    db: Db,
    payment_svc: Arc<PaymentService>,
    store: DialogueStore,
    config: Config,
}

impl<T: BotTransport> AppHandler<T> {
    pub fn new(
        transport: T,
        db: Db,
        payment_svc: Arc<PaymentService>,
        store: DialogueStore,
        config: Config,
    ) -> Self {
        Self {
            transport,
            db,
            payment_svc,
            store,
            config,
        }
    }

    async fn handle_message_new(&self, message: &VkMessage) -> HandlerResult {
        let peer_id = message.peer_id;
        let user_id = message.from_id;
        let text = message.text.trim();

        if text.is_empty() {
            return Ok(());
        }

        if text.starts_with('/') {
            self.handle_command(message, text).await?;
            return Ok(());
        }

        match self.store.get(user_id) {
            AppState::AwaitingUtc => {
                handlers::text::handle_utc_input_transport(
                    &self.transport,
                    peer_id,
                    user_id,
                    text,
                    &self.store,
                    self.db.clone(),
                )
                .await?;
            }
            AppState::AwaitingSnoozeButtons => {
                handlers::text::handle_snooze_input_transport(
                    &self.transport,
                    peer_id,
                    user_id,
                    text,
                    &self.store,
                    self.db.clone(),
                )
                .await?;
            }
            AppState::AwaitingAutoSnooze => {
                handlers::text::handle_auto_snooze_input_transport(
                    &self.transport,
                    peer_id,
                    user_id,
                    text,
                    &self.store,
                    self.db.clone(),
                )
                .await?;
            }
            AppState::AwaitingReminderEdit { .. } => {
                handlers::reminder::handle_reminder_edit_text_transport(
                    &self.transport,
                    peer_id,
                    user_id,
                    text,
                    &self.store,
                    self.db.clone(),
                )
                .await?;
            }
            AppState::AwaitingReminderDeletion => {
                handlers::reminder::handle_deletion_input_transport(
                    &self.transport,
                    peer_id,
                    user_id,
                    text,
                    &self.store,
                    self.db.clone(),
                )
                .await?;
            }
            AppState::AwaitingSubDeleteNum => {
                handlers::channels::handle_sub_delete_num_transport(
                    &self.transport,
                    peer_id,
                    user_id,
                    text,
                    &self.store,
                    self.db.clone(),
                )
                .await?;
            }
            AppState::Idle => {
                if is_group_peer(peer_id) {
                    handlers::text::handle_group_text_transport(
                        &self.transport,
                        peer_id,
                        user_id,
                        text,
                        None,
                        &self.store,
                        self.db.clone(),
                        self.config.clone(),
                    )
                    .await?;
                } else if handlers::channels::parse_channel_url(text).is_some() {
                    handlers::channels::handle_channel_url_transport(
                        &self.transport,
                        peer_id,
                        user_id,
                        text,
                        &self.store,
                        self.db.clone(),
                    )
                    .await?;
                } else {
                    handlers::reminder::handle_idle_text_transport(
                        &self.transport,
                        peer_id,
                        user_id,
                        text,
                        &self.store,
                        self.db.clone(),
                    )
                    .await?;
                }
            }
            AppState::AwaitingPayment { .. }
            | AppState::AwaitingTextConfirmation { .. }
            | AppState::AwaitingReminderConfirmation { .. } => {}
        }

        Ok(())
    }

    async fn handle_command(&self, message: &VkMessage, text: &str) -> HandlerResult {
        let peer_id = message.peer_id;
        let user_id = message.from_id;
        let is_group = is_group_peer(peer_id);
        let (command, args) = parse_command(text);

        match command.as_str() {
            "start" => {
                handlers::commands::command_start_transport(
                    &self.transport,
                    peer_id,
                    user_id,
                    is_group,
                    None,
                    &self.store,
                    self.db.clone(),
                )
                .await?;
            }
            "help" => {
                handlers::commands::command_help_transport(&self.transport, peer_id).await?;
            }
            "yan" => {
                handlers::commands::command_yan_transport(&self.transport, peer_id).await?;
            }
            "utc" => {
                handlers::commands::command_utc_transport(
                    &self.transport,
                    peer_id,
                    user_id,
                    &self.store,
                    self.db.clone(),
                )
                .await?;
            }
            "setup" => {
                handlers::commands::command_setup_transport(
                    &self.transport,
                    peer_id,
                    user_id,
                    &self.store,
                    self.db.clone(),
                )
                .await?;
            }
            "pay" => {
                handlers::pay::command_pay_transport(
                    &self.transport,
                    peer_id,
                    user_id,
                    &self.store,
                    self.db.clone(),
                    self.payment_svc.clone(),
                )
                .await?;
            }
            "list" => {
                handlers::reminder::handle_list_command_transport(
                    &self.transport,
                    peer_id,
                    self.db.clone(),
                )
                .await?;
            }
            "subs" => {
                handlers::channels::command_subs_transport(
                    &self.transport,
                    peer_id,
                    user_id,
                    self.db.clone(),
                )
                .await?;
            }
            "profile" => {
                handlers::profile::handle_profile_command_transport(
                    &self.transport,
                    peer_id,
                    user_id,
                    None,
                    self.db.clone(),
                )
                .await?;
            }
            "ref" => {
                handlers::referral::command_ref_transport(&self.transport, peer_id).await?;
            }
            "remind" => {
                handlers::commands::command_remind_transport(
                    &self.transport,
                    peer_id,
                    user_id,
                    is_group,
                    None,
                    args,
                    &self.store,
                    self.db.clone(),
                    self.config.clone(),
                )
                .await?;
            }
            _ => {
                self.transport
                    .send_text(peer_id, "Неизвестная команда. Используйте /help")
                    .await?;
            }
        }

        Ok(())
    }

    async fn handle_message_event(&self, event: &MessageEvent) -> HandlerResult {
        let Some(payload) = callback_payload(event) else {
            return Ok(());
        };

        let event_id = event.event_id.as_str();
        let user_id = event.user_id;
        let peer_id = event.peer_id;

        match payload.as_str() {
            "setup_menu" => {
                self.store.update(user_id, AppState::Idle);
                self.db.update_user_state(peer_id, "waiting_for_message").await?;
                self.transport
                    .answer_callback(event_id, user_id, peer_id, None)
                    .await?;
                send_html_with_keyboard(
                    &self.transport,
                    peer_id,
                    handlers::commands::SETUP_PROMPT,
                    &setup_keyboard(),
                )
                .await?;
                return Ok(());
            }
            "setup_snooze" => {
                let user = self.db.ensure_user(peer_id).await?;
                self.store.update(user_id, AppState::AwaitingSnoozeButtons);
                self.db.update_user_state(peer_id, "waiting_for_time").await?;
                self.transport
                    .answer_callback(event_id, user_id, peer_id, None)
                    .await?;
                let current = if user.snooze_buttons.is_empty() {
                    "15 мин, 1 час, 3 часа".to_string()
                } else {
                    user.snooze_buttons
                        .iter()
                        .filter_map(|c| handlers::text::human_readable_snooze(c))
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                let text = format!("{}\n\nТекущие: <b>{}</b>", handlers::commands::SNOOZE_PROMPT, current);
                send_html_with_keyboard(&self.transport, peer_id, &text, &back_keyboard()).await?;
                return Ok(());
            }
            "setup_auto" => {
                let user = self.db.ensure_user(peer_id).await?;
                self.store.update(user_id, AppState::AwaitingAutoSnooze);
                self.db.update_user_state(peer_id, "waiting_for_time").await?;
                self.transport
                    .answer_callback(event_id, user_id, peer_id, None)
                    .await?;
                let current = if user.auto_snooze.is_empty() {
                    "15 мин".to_string()
                } else {
                    handlers::text::human_readable_auto(&user.auto_snooze)
                };
                let text = format!(
                    "{}\n\nТекущее: <b>{}</b>",
                    handlers::commands::AUTO_SNOOZE_PROMPT,
                    current
                );
                send_html_with_keyboard(&self.transport, peer_id, &text, &back_keyboard()).await?;
                return Ok(());
            }
            "setup_utc" => {
                self.transport
                    .answer_callback(event_id, user_id, peer_id, None)
                    .await?;
                handlers::commands::start_utc_flow_transport(
                    &self.transport,
                    peer_id,
                    user_id,
                    &self.store,
                    self.db.clone(),
                )
                .await?;
                return Ok(());
            }
            _ => {}
        }

        if payload == "utc_cancel" {
            self.store.update(user_id, AppState::Idle);
            self.transport
                .answer_callback(event_id, user_id, peer_id, None)
                .await?;
            self.transport
                .send_text(peer_id, "Настройка часового пояса отменена.")
                .await?;
            return Ok(());
        }

        if let Some(rest) = payload.strip_prefix("utc_set:") {
            if let Some(offset) = handlers::text::normalize_offset(rest) {
                self.db.update_utc_and_clear_timezone(peer_id, &offset).await?;
                self.db.update_user_state(peer_id, "waiting_for_message").await?;
                self.store.update(user_id, AppState::Idle);
                self.transport
                    .answer_callback(event_id, user_id, peer_id, None)
                    .await?;
                send_html_text(
                    &self.transport,
                    peer_id,
                    &handlers::commands::UTC_SUCCESS_MESSAGE.replace("+3:00", &offset),
                )
                .await?;
                return Ok(());
            }
        }

        if payload.starts_with("pay_") {
            return handlers::pay::handle_pay_callback_transport(
                &self.transport,
                event_id,
                user_id,
                peer_id,
                &payload,
                &self.store,
                self.db.clone(),
                self.payment_svc.clone(),
            )
            .await;
        }

        match payload.as_str() {
            "text_confirm" => {
                return handlers::reminder::handle_text_confirm_transport(
                    &self.transport,
                    event_id,
                    user_id,
                    peer_id,
                    &payload,
                    &self.store,
                    self.db.clone(),
                )
                .await;
            }
            "text_cancel" => {
                return handlers::reminder::handle_text_cancel_transport(
                    &self.transport,
                    event_id,
                    user_id,
                    peer_id,
                    &payload,
                    &self.store,
                )
                .await;
            }
            "reminder_confirm" => {
                return handlers::reminder::handle_reminder_confirm_transport(
                    &self.transport,
                    event_id,
                    user_id,
                    peer_id,
                    &payload,
                    &self.store,
                    self.db.clone(),
                )
                .await;
            }
            "reminder_edit" => {
                return handlers::reminder::handle_reminder_edit_transport(
                    &self.transport,
                    event_id,
                    user_id,
                    peer_id,
                    &payload,
                    &self.store,
                )
                .await;
            }
            "reminder_cancel" => {
                return handlers::reminder::handle_reminder_cancel_transport(
                    &self.transport,
                    event_id,
                    user_id,
                    peer_id,
                    &payload,
                    &self.store,
                )
                .await;
            }
            "reminder_delete_start" => {
                return handlers::reminder::handle_delete_start_transport(
                    &self.transport,
                    event_id,
                    user_id,
                    peer_id,
                    &payload,
                    &self.store,
                )
                .await;
            }
            "reminder_delete_back" => {
                return handlers::reminder::handle_delete_back_transport(
                    &self.transport,
                    event_id,
                    user_id,
                    peer_id,
                    &payload,
                    &self.store,
                    self.db.clone(),
                )
                .await;
            }
            "sub_delete" => {
                return handlers::channels::handle_sub_delete_callback_transport(
                    &self.transport,
                    event_id,
                    user_id,
                    peer_id,
                    &self.store,
                    self.db.clone(),
                )
                .await;
            }
            "subs" => {
                return handlers::channels::handle_subs_callback_transport(
                    &self.transport,
                    event_id,
                    user_id,
                    peer_id,
                    self.db.clone(),
                )
                .await;
            }
            "profile" | "profile_stub" => {
                self.transport
                    .answer_callback(event_id, user_id, peer_id, None)
                    .await?;
                return handlers::profile::handle_profile_command_transport(
                    &self.transport,
                    peer_id,
                    user_id,
                    None,
                    self.db.clone(),
                )
                .await;
            }
            "profile_list" => {
                self.transport
                    .answer_callback(event_id, user_id, peer_id, None)
                    .await?;
                return handlers::reminder::handle_list_command_transport(
                    &self.transport,
                    peer_id,
                    self.db.clone(),
                )
                .await;
            }
            "profile_setup" => {
                self.transport
                    .answer_callback(event_id, user_id, peer_id, None)
                    .await?;
                return handlers::commands::command_setup_transport(
                    &self.transport,
                    peer_id,
                    user_id,
                    &self.store,
                    self.db.clone(),
                )
                .await;
            }
            "profile_subs" => {
                self.transport
                    .answer_callback(event_id, user_id, peer_id, None)
                    .await?;
                return handlers::channels::command_subs_transport(
                    &self.transport,
                    peer_id,
                    user_id,
                    self.db.clone(),
                )
                .await;
            }
            "profile_referral" => {
                self.transport
                    .answer_callback(event_id, user_id, peer_id, None)
                    .await?;
                return handlers::referral::command_ref_transport(&self.transport, peer_id).await;
            }
            "profile_pay" => {
                self.transport
                    .answer_callback(event_id, user_id, peer_id, None)
                    .await?;
                return handlers::pay::command_pay_transport(
                    &self.transport,
                    peer_id,
                    user_id,
                    &self.store,
                    self.db.clone(),
                    self.payment_svc.clone(),
                )
                .await;
            }
            "back_main" => {
                self.store.update(user_id, AppState::Idle);
                self.transport
                    .answer_callback(event_id, user_id, peer_id, None)
                    .await?;
                self.transport
                    .send_text(peer_id, "Хорошо! Напиши мне, что нужно запомнить 📝")
                    .await?;
                return Ok(());
            }
            "reminder_list" => {
                self.transport
                    .answer_callback(event_id, user_id, peer_id, None)
                    .await?;
                return handlers::reminder::handle_list_command_transport(
                    &self.transport,
                    peer_id,
                    self.db.clone(),
                )
                .await;
            }
            _ => {}
        }

        if payload.starts_with("snooze:") {
            return handlers::reminder::handle_snooze_callback_transport(
                &self.transport,
                event_id,
                user_id,
                peer_id,
                &payload,
                self.db.clone(),
            )
            .await;
        }

        if payload.starts_with("reminder_done:") {
            return handlers::reminder::handle_reminder_done_callback_transport(
                &self.transport,
                event_id,
                user_id,
                peer_id,
                &payload,
                self.db.clone(),
            )
            .await;
        }

        self.transport
            .answer_callback(
                event_id,
                user_id,
                peer_id,
                Some("Не удалось обработать выбор. Попробуйте снова."),
            )
            .await?;
        Ok(())
    }
}

#[async_trait]
impl<T: BotTransport> MessageHandler for AppHandler<T> {
    async fn handle(&self, event: &Event, _api: &VkApi) -> VkResult<()> {
        let result = match event {
            Event::MessageNew(message) => self.handle_message_new(message).await,
            Event::MessageEvent(event) => self.handle_message_event(event).await,
            _ => Ok(()),
        };

        result.map_err(|e| VkError::Custom(e.to_string()))
    }
}

#[cfg(feature = "telegram-legacy")]
pub async fn build_deps() -> anyhow::Result<DependencyMap> {
    let config = crate::config::Config::from_env();
    let storage = InMemStorage::<AppState>::new();
    let db = Db::connect(&config.mongo_uri, None).await?;

    let payment_svc: Arc<PaymentService> = if config.payments_enabled {
        Arc::new(PaymentService::from_env(db.clone())?)
    } else {
        Arc::new(PaymentService::disabled(db.clone()))
    };

    Ok(dptree::deps![config, storage, db, payment_svc])
}

/// Legacy Telegram schema kept behind `telegram-legacy` for local compatibility.
#[cfg(feature = "telegram-legacy")]
pub fn schema() -> UpdateHandler<anyhow::Error> {
    use teloxide::dispatching::UpdateFilterExt;

    let messages = Update::filter_message()
        .enter_dialogue::<Message, InMemStorage<AppState>, AppState>()
        .branch(handlers::commands::router())
        .branch(handlers::text::router())
        .branch(
            dptree::case![AppState::AwaitingReminderEdit { pending }]
                .endpoint(handlers::reminder::handle_reminder_edit_text),
        )
        .branch(
            dptree::case![AppState::AwaitingReminderDeletion]
                .endpoint(handlers::reminder::handle_deletion_input),
        )
        .branch(
            dptree::case![AppState::AwaitingSubDeleteNum]
                .endpoint(handlers::channels::handle_sub_delete_num),
        )
        .branch(dptree::case![AppState::Idle].endpoint(handlers::reminder::handle_idle_text));

    let callbacks = Update::filter_callback_query()
        .enter_dialogue::<CallbackQuery, InMemStorage<AppState>, AppState>()
        .branch(dptree::endpoint(handlers::callbacks::handle_callback));

    dptree::entry().branch(messages).branch(callbacks)
}

fn is_group_peer(peer_id: i64) -> bool {
    peer_id >= 2_000_000_000
}

fn parse_command(text: &str) -> (String, String) {
    let mut parts = text.trim().splitn(2, char::is_whitespace);
    let raw = parts.next().unwrap_or_default();
    let args = parts.next().unwrap_or_default().trim().to_string();
    let command = raw
        .trim_start_matches('/')
        .split('@')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();

    (command, args)
}

fn callback_payload(event: &MessageEvent) -> Option<String> {
    let payload = event.payload.as_ref()?;

    payload
        .get("action")
        .or_else(|| payload.get("command"))
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .or_else(|| {
            payload
                .values()
                .find_map(|value| value.as_str().map(ToOwned::to_owned))
        })
}

async fn send_html_text<T: BotTransport>(transport: &T, peer_id: i64, text: &str) -> HandlerResult {
    let text = strip_html(text);
    transport.send_text(peer_id, &text).await?;
    Ok(())
}

async fn send_html_with_keyboard<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    text: &str,
    keyboard: &TransportKeyboard,
) -> HandlerResult {
    let text = strip_html(text);
    transport.send_with_keyboard(peer_id, &text, keyboard).await?;
    Ok(())
}
