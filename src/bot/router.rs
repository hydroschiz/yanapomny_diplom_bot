use std::sync::Arc;

use async_trait::async_trait;
use presentation::{parse_command, parse_payload, BotCommand, CallbackPayload};
use vk_bot_api::api::VkApi;
use vk_bot_api::error::{VkError, VkResult};
use vk_bot_api::handler::MessageHandler;
use vk_bot_api::models::{Event, Message as VkMessage, MessageEvent};

use crate::api::db::Db;
use crate::api::payments::PaymentService;
use crate::bot::keyboards::{
    back_keyboard, setup_keyboard, utc_keyboard_page, utc_keyboard_page_count,
};
use crate::bot::states::AppState;
use crate::config::Config;
use crate::transport::dialogue_store::DialogueStore;
use crate::transport::text_format::strip_html;
use crate::transport::traits::{BotTransport, TransportKeyboard};

use super::handlers;

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
        let Some(command) = parse_command(text) else {
            self.transport
                .send_text(peer_id, "Неизвестная команда. Используйте /help")
                .await?;
            return Ok(());
        };

        match command.command {
            BotCommand::Start => {
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
            BotCommand::Help => {
                handlers::commands::command_help_transport(&self.transport, peer_id).await?;
            }
            BotCommand::Yan => {
                handlers::commands::command_yan_transport(&self.transport, peer_id).await?;
            }
            BotCommand::Utc => {
                handlers::commands::command_utc_transport(
                    &self.transport,
                    peer_id,
                    user_id,
                    &self.store,
                    self.db.clone(),
                )
                .await?;
            }
            BotCommand::Setup => {
                handlers::commands::command_setup_transport(
                    &self.transport,
                    peer_id,
                    user_id,
                    &self.store,
                    self.db.clone(),
                )
                .await?;
            }
            BotCommand::Pay => {
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
            BotCommand::List => {
                handlers::reminder::handle_list_command_transport(
                    &self.transport,
                    peer_id,
                    self.db.clone(),
                )
                .await?;
            }
            BotCommand::Subs => {
                handlers::channels::command_subs_transport(
                    &self.transport,
                    peer_id,
                    user_id,
                    self.db.clone(),
                )
                .await?;
            }
            BotCommand::Profile => {
                handlers::profile::handle_profile_command_transport(
                    &self.transport,
                    peer_id,
                    user_id,
                    None,
                    self.db.clone(),
                )
                .await?;
            }
            BotCommand::Ref => {
                handlers::referral::command_ref_transport(&self.transport, peer_id).await?;
            }
            BotCommand::Remind(args) => {
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
            BotCommand::Unknown(_) => {
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
        let parsed_payload = parse_payload(&payload);

        let event_id = event.event_id.as_str();
        let user_id = event.user_id;
        let peer_id = event.peer_id;

        match &parsed_payload {
            CallbackPayload::SetupMenu => {
                self.store.update(user_id, AppState::Idle);
                self.db
                    .update_user_state(peer_id, "waiting_for_message")
                    .await?;
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
            CallbackPayload::SetupSnooze => {
                let user = self.db.ensure_user(peer_id).await?;
                self.store.update(user_id, AppState::AwaitingSnoozeButtons);
                self.db
                    .update_user_state(peer_id, "waiting_for_time")
                    .await?;
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
                let text = format!(
                    "{}\n\nТекущие: <b>{}</b>",
                    handlers::commands::SNOOZE_PROMPT,
                    current
                );
                send_html_with_keyboard(&self.transport, peer_id, &text, &back_keyboard()).await?;
                return Ok(());
            }
            CallbackPayload::SetupAuto => {
                let user = self.db.ensure_user(peer_id).await?;
                self.store.update(user_id, AppState::AwaitingAutoSnooze);
                self.db
                    .update_user_state(peer_id, "waiting_for_time")
                    .await?;
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
            CallbackPayload::SetupUtc => {
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

        if matches!(&parsed_payload, CallbackPayload::UtcCancel) {
            self.store.update(user_id, AppState::Idle);
            self.transport
                .answer_callback(event_id, user_id, peer_id, None)
                .await?;
            self.transport
                .send_text(peer_id, "Настройка часового пояса отменена.")
                .await?;
            return Ok(());
        }

        if let CallbackPayload::UtcPage(page) = &parsed_payload {
            self.transport
                .answer_callback(event_id, user_id, peer_id, None)
                .await?;
            let page_count = utc_keyboard_page_count();
            let text = format!(
                "Выберите UTC смещение кнопкой или отправьте город/смещение текстом.\n\nСтраница {}/{}",
                *page % page_count + 1,
                page_count
            );
            send_html_with_keyboard(&self.transport, peer_id, &text, &utc_keyboard_page(*page))
                .await?;
            return Ok(());
        }

        if let CallbackPayload::UtcSet(rest) = &parsed_payload {
            if let Some(offset) = handlers::text::normalize_offset(rest) {
                self.db
                    .update_utc_and_clear_timezone(peer_id, &offset)
                    .await?;
                self.db
                    .update_user_state(peer_id, "waiting_for_message")
                    .await?;
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

        if matches!(
            &parsed_payload,
            CallbackPayload::PayMenu
                | CallbackPayload::PayCancel
                | CallbackPayload::PaySelect(_)
                | CallbackPayload::PayYooKassa(_)
                | CallbackPayload::PayCheck(_)
        ) {
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

        match &parsed_payload {
            CallbackPayload::TextConfirm => {
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
            CallbackPayload::TextCancel => {
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
            CallbackPayload::ReminderConfirm => {
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
            CallbackPayload::ReminderEdit => {
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
            CallbackPayload::ReminderCancel => {
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
            CallbackPayload::ReminderDeleteStart => {
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
            CallbackPayload::ReminderDeleteBack => {
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
            CallbackPayload::SubDelete => {
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
            CallbackPayload::Subs => {
                return handlers::channels::handle_subs_callback_transport(
                    &self.transport,
                    event_id,
                    user_id,
                    peer_id,
                    self.db.clone(),
                )
                .await;
            }
            CallbackPayload::Profile => {
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
            CallbackPayload::ProfileList => {
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
            CallbackPayload::ProfileSetup => {
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
            CallbackPayload::ProfileSubs => {
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
            CallbackPayload::ProfileReferral => {
                self.transport
                    .answer_callback(event_id, user_id, peer_id, None)
                    .await?;
                return handlers::referral::command_ref_transport(&self.transport, peer_id).await;
            }
            CallbackPayload::ProfilePay => {
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
            CallbackPayload::BackMain => {
                self.store.update(user_id, AppState::Idle);
                self.transport
                    .answer_callback(event_id, user_id, peer_id, None)
                    .await?;
                self.transport
                    .send_text(peer_id, "Хорошо! Напиши мне, что нужно запомнить 📝")
                    .await?;
                return Ok(());
            }
            CallbackPayload::ReminderList => {
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

        if matches!(&parsed_payload, CallbackPayload::Snooze { .. }) {
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

        if matches!(&parsed_payload, CallbackPayload::ReminderDone(_)) {
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

fn is_group_peer(peer_id: i64) -> bool {
    peer_id >= 2_000_000_000
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
    transport
        .send_with_keyboard(peer_id, &text, keyboard)
        .await?;
    Ok(())
}
