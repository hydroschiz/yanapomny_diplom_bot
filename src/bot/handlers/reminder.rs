//! Reminder creation and management handlers.

use async_trait::async_trait;
use chrono::{DateTime, Datelike, Timelike, Utc};
#[cfg(feature = "telegram-legacy")]
use teloxide::prelude::*;

use crate::api::db::{Db, Reminder, User};
use crate::api::llm_client::LlmClient;
use crate::api::llm_models::{ParsedReminder, ReminderType};
use crate::api::time_calculator::{calculate_reminder_time, UserTimePrefs};
use crate::bot::keyboards::{
    delete_keyboard, list_delete_keyboard, reminder_confirm_keyboard, reminder_edit_keyboard,
    reminder_snoozed_keyboard, snooze_code_to_label, snooze_code_to_minutes, text_confirm_keyboard,
};
#[cfg(feature = "telegram-legacy")]
use crate::bot::router::AppDialogue;
use crate::bot::router::HandlerResult;
use crate::bot::states::{AppState, PendingReminder, PendingText};
#[cfg(feature = "telegram-legacy")]
use crate::transport::adapters::reply_markup_from_transport_keyboard;
use crate::transport::dialogue_store::DialogueStore;
use crate::transport::text_format::strip_html;
use crate::transport::traits::{BotTransport, TransportKeyboard};
use crate::utils::timezone::{
    user_datetime_string, user_has_timezone, user_local_time, user_offset_string_at,
};

/// LLM client shared across handlers.
static LLM_CLIENT: std::sync::OnceLock<LlmClient> = std::sync::OnceLock::new();

fn get_llm_client() -> &'static LlmClient {
    LLM_CLIENT.get_or_init(|| LlmClient::from_env().expect("Failed to create LLM client"))
}

#[async_trait]
trait ReminderStateStore {
    async fn get_state(&self, user_id: i64) -> anyhow::Result<AppState>;
    async fn update_state(&self, user_id: i64, state: AppState) -> anyhow::Result<()>;
}

#[async_trait]
impl ReminderStateStore for DialogueStore {
    async fn get_state(&self, user_id: i64) -> anyhow::Result<AppState> {
        Ok(self.get(user_id))
    }

    async fn update_state(&self, user_id: i64, state: AppState) -> anyhow::Result<()> {
        self.update(user_id, state);
        Ok(())
    }
}

#[cfg(feature = "telegram-legacy")]
#[async_trait]
impl ReminderStateStore for AppDialogue {
    async fn get_state(&self, _user_id: i64) -> anyhow::Result<AppState> {
        Ok(self.get().await?.unwrap_or_default())
    }

    async fn update_state(&self, _user_id: i64, state: AppState) -> anyhow::Result<()> {
        self.update(state).await?;
        Ok(())
    }
}

#[cfg(feature = "telegram-legacy")]
#[derive(Clone)]
struct TelegramReminderTransport {
    bot: Bot,
}

#[cfg(feature = "telegram-legacy")]
impl TelegramReminderTransport {
    fn new(bot: Bot) -> Self {
        Self { bot }
    }
}

#[cfg(feature = "telegram-legacy")]
#[async_trait]
impl BotTransport for TelegramReminderTransport {
    async fn send_text(&self, peer_id: i64, text: &str) -> anyhow::Result<()> {
        self.bot.send_message(ChatId(peer_id), text).await?;
        Ok(())
    }

    async fn send_with_keyboard(
        &self,
        peer_id: i64,
        text: &str,
        keyboard: &TransportKeyboard,
    ) -> anyhow::Result<()> {
        let markup = reply_markup_from_transport_keyboard(keyboard);
        self.bot
            .send_message(ChatId(peer_id), text)
            .reply_markup(markup)
            .await?;
        Ok(())
    }

    async fn answer_callback(
        &self,
        event_id: &str,
        _user_id: i64,
        _peer_id: i64,
        text: Option<&str>,
    ) -> anyhow::Result<()> {
        let request = self
            .bot
            .answer_callback_query(teloxide::types::CallbackQueryId(event_id.to_string()));

        match text {
            Some(text) => request.text(text).await?,
            None => request.await?,
        };

        Ok(())
    }
}

// ============================================================================
// Transport-native handlers
// ============================================================================

/// Handle any text message in Idle state - ask for confirmation BEFORE sending to LLM.
pub async fn handle_idle_text_transport<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    text: &str,
    store: &DialogueStore,
    db: Db,
) -> HandlerResult {
    handle_idle_text_core(transport, peer_id, user_id, text, store, db).await
}

/// Start the flow of creating a reminder (confirmation -> LLM).
/// Assumes subscription and user checks are already done.
pub async fn start_reminder_creation_flow_transport<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    text: String,
    store: &DialogueStore,
) -> HandlerResult {
    start_reminder_creation_flow_core(transport, peer_id, user_id, text, store).await
}

/// Handle text confirmation - now send to LLM and show parsed result.
pub async fn handle_text_confirm_transport<T: BotTransport>(
    transport: &T,
    event_id: &str,
    user_id: i64,
    peer_id: i64,
    _payload: &str,
    store: &DialogueStore,
    db: Db,
) -> HandlerResult {
    handle_text_confirm_core(transport, event_id, user_id, peer_id, store, db).await
}

/// Handle text cancel - user doesn't want to create reminder.
pub async fn handle_text_cancel_transport<T: BotTransport>(
    transport: &T,
    event_id: &str,
    user_id: i64,
    peer_id: i64,
    _payload: &str,
    store: &DialogueStore,
) -> HandlerResult {
    handle_text_cancel_core(transport, event_id, user_id, peer_id, store).await
}

/// Handle reminder confirmation callback.
pub async fn handle_reminder_confirm_transport<T: BotTransport>(
    transport: &T,
    event_id: &str,
    user_id: i64,
    peer_id: i64,
    _payload: &str,
    store: &DialogueStore,
    db: Db,
) -> HandlerResult {
    handle_reminder_confirm_core(transport, event_id, user_id, peer_id, store, db).await
}

/// Handle reminder edit callback - ask for new text.
pub async fn handle_reminder_edit_transport<T: BotTransport>(
    transport: &T,
    event_id: &str,
    user_id: i64,
    peer_id: i64,
    _payload: &str,
    store: &DialogueStore,
) -> HandlerResult {
    handle_reminder_edit_core(transport, event_id, user_id, peer_id, store).await
}

/// Handle edited reminder text.
pub async fn handle_reminder_edit_text_transport<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    text: &str,
    store: &DialogueStore,
    db: Db,
) -> HandlerResult {
    handle_reminder_edit_text_core(transport, peer_id, user_id, text, store, db).await
}

/// Handle reminder cancellation.
pub async fn handle_reminder_cancel_transport<T: BotTransport>(
    transport: &T,
    event_id: &str,
    user_id: i64,
    peer_id: i64,
    _payload: &str,
    store: &DialogueStore,
) -> HandlerResult {
    handle_reminder_cancel_core(transport, event_id, user_id, peer_id, store).await
}

/// Handle /list command - show user's reminders.
pub async fn handle_list_command_transport<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    db: Db,
) -> HandlerResult {
    handle_list_command_core(transport, peer_id, db).await
}

/// Handle delete button press - start deletion flow.
pub async fn handle_delete_start_transport<T: BotTransport>(
    transport: &T,
    event_id: &str,
    user_id: i64,
    peer_id: i64,
    _payload: &str,
    store: &DialogueStore,
) -> HandlerResult {
    handle_delete_start_core(transport, event_id, user_id, peer_id, store).await
}

/// Handle deletion number input.
pub async fn handle_deletion_input_transport<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    text: &str,
    store: &DialogueStore,
    db: Db,
) -> HandlerResult {
    handle_deletion_input_core(transport, peer_id, user_id, text, store, db).await
}

/// Handle back button in deletion flow.
pub async fn handle_delete_back_transport<T: BotTransport>(
    transport: &T,
    event_id: &str,
    user_id: i64,
    peer_id: i64,
    _payload: &str,
    store: &DialogueStore,
    db: Db,
) -> HandlerResult {
    handle_delete_back_core(transport, event_id, user_id, peer_id, store, db).await
}

/// Handle snooze callback: `snooze:{rem_id}:{code}`.
pub async fn handle_snooze_callback_transport<T: BotTransport>(
    transport: &T,
    event_id: &str,
    user_id: i64,
    peer_id: i64,
    payload: &str,
    db: Db,
) -> HandlerResult {
    let data = payload.strip_prefix("snooze:").unwrap_or(payload);
    let parts: Vec<&str> = data.split(':').collect();
    if parts.len() != 2 {
        transport
            .answer_callback(event_id, user_id, peer_id, Some("Ошибка: неверный формат"))
            .await?;
        return Ok(());
    }

    let rem_id: i32 = match parts[0].parse() {
        Ok(id) => id,
        Err(_) => {
            transport
                .answer_callback(
                    event_id,
                    user_id,
                    peer_id,
                    Some("Ошибка: неверный ID напоминания"),
                )
                .await?;
            return Ok(());
        }
    };

    let snooze_code = parts[1];
    let minutes = match snooze_code_to_minutes(snooze_code) {
        Some(m) => m,
        None => {
            transport
                .answer_callback(
                    event_id,
                    user_id,
                    peer_id,
                    Some("Ошибка: неверный интервал"),
                )
                .await?;
            return Ok(());
        }
    };

    let reminder = match db.find_reminder(rem_id).await? {
        Some(r) => r,
        None => {
            transport
                .answer_callback(event_id, user_id, peer_id, Some("Напоминание не найдено"))
                .await?;
            return Ok(());
        }
    };

    let new_time = db.snooze_reminder(rem_id, minutes).await?;
    let user = db.ensure_user(peer_id).await?;
    let time_display = crate::scheduler::format_full_reminder_time_for_user(&new_time, &user);
    let snooze_label = snooze_code_to_label(snooze_code);
    let message = format!(
        "Напоминание отложено на {} ✅\n\n\
         📅 {} ▹ {}",
        snooze_label,
        time_display,
        html_escape(&reminder.text)
    );
    let keyboard = reminder_snoozed_keyboard(rem_id);

    send_html_with_keyboard(transport, peer_id, &message, &keyboard).await?;
    transport
        .answer_callback(event_id, user_id, peer_id, None)
        .await?;

    Ok(())
}

/// Handle reminder done callback: `reminder_done:{rem_id}`.
pub async fn handle_reminder_done_callback_transport<T: BotTransport>(
    transport: &T,
    event_id: &str,
    user_id: i64,
    peer_id: i64,
    payload: &str,
    db: Db,
) -> HandlerResult {
    let data = payload.strip_prefix("reminder_done:").unwrap_or(payload);
    let rem_id: i32 = match data.parse() {
        Ok(id) => id,
        Err(_) => {
            transport
                .answer_callback(
                    event_id,
                    user_id,
                    peer_id,
                    Some("Ошибка: неверный ID напоминания"),
                )
                .await?;
            return Ok(());
        }
    };

    let reminder = match db.find_reminder(rem_id).await? {
        Some(r) => r,
        None => {
            transport
                .answer_callback(event_id, user_id, peer_id, Some("Напоминание уже удалено"))
                .await?;
            return Ok(());
        }
    };

    let user = db.ensure_user(peer_id).await?;
    let time_display = crate::scheduler::format_full_reminder_time_for_user(&reminder.time, &user);
    let is_recurring = !reminder.delay.is_empty();

    let message = if is_recurring {
        format!(
            "Напоминание отмечено ✅\n\n\
             📅 {} ▹ {}\n\n\
             🔄 Это периодическое напоминание, оно продолжит работать.",
            time_display,
            html_escape(&reminder.text)
        )
    } else {
        db.complete_reminder(rem_id).await?;
        format!(
            "Напоминание выполнено ✅ 📅 {} ▹ {}",
            time_display,
            html_escape(&reminder.text)
        )
    };

    send_html_text(transport, peer_id, &message).await?;
    transport
        .answer_callback(event_id, user_id, peer_id, None)
        .await?;

    Ok(())
}

// ============================================================================
// Telegram compatibility entrypoints
// ============================================================================

#[cfg(feature = "telegram-legacy")]
pub async fn handle_idle_text(
    bot: Bot,
    msg: Message,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let text = match msg.text() {
        Some(t) => t,
        None => return Ok(()),
    };
    let peer_id = msg.chat.id.0;
    let user_id = message_user_id(&msg).unwrap_or(peer_id);
    let transport = TelegramReminderTransport::new(bot);

    handle_idle_text_core(&transport, peer_id, user_id, text, &dialogue, db).await
}

#[cfg(feature = "telegram-legacy")]
pub async fn start_reminder_creation_flow(
    bot: Bot,
    chat_id: ChatId,
    text: String,
    dialogue: AppDialogue,
) -> HandlerResult {
    let transport = TelegramReminderTransport::new(bot);

    start_reminder_creation_flow_core(&transport, chat_id.0, chat_id.0, text, &dialogue).await
}

#[cfg(feature = "telegram-legacy")]
pub async fn handle_text_confirm(
    bot: Bot,
    q: CallbackQuery,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let Some((event_id, user_id, peer_id)) = callback_context(&q) else {
        return Ok(());
    };
    let transport = TelegramReminderTransport::new(bot);

    handle_text_confirm_core(&transport, &event_id, user_id, peer_id, &dialogue, db).await
}

#[cfg(feature = "telegram-legacy")]
pub async fn handle_text_cancel(
    bot: Bot,
    q: CallbackQuery,
    dialogue: AppDialogue,
) -> HandlerResult {
    let Some((event_id, user_id, peer_id)) = callback_context(&q) else {
        return Ok(());
    };
    let transport = TelegramReminderTransport::new(bot);

    handle_text_cancel_core(&transport, &event_id, user_id, peer_id, &dialogue).await
}

#[cfg(feature = "telegram-legacy")]
pub async fn handle_reminder_confirm(
    bot: Bot,
    q: CallbackQuery,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let Some((event_id, user_id, peer_id)) = callback_context(&q) else {
        return Ok(());
    };
    let transport = TelegramReminderTransport::new(bot);

    handle_reminder_confirm_core(&transport, &event_id, user_id, peer_id, &dialogue, db).await
}

#[cfg(feature = "telegram-legacy")]
pub async fn handle_reminder_edit(
    bot: Bot,
    q: CallbackQuery,
    dialogue: AppDialogue,
) -> HandlerResult {
    let Some((event_id, user_id, peer_id)) = callback_context(&q) else {
        return Ok(());
    };
    let transport = TelegramReminderTransport::new(bot);

    handle_reminder_edit_core(&transport, &event_id, user_id, peer_id, &dialogue).await
}

#[cfg(feature = "telegram-legacy")]
pub async fn handle_reminder_edit_text(
    bot: Bot,
    msg: Message,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let text = match msg.text() {
        Some(t) => t,
        None => return Ok(()),
    };
    let peer_id = msg.chat.id.0;
    let user_id = message_user_id(&msg).unwrap_or(peer_id);
    let transport = TelegramReminderTransport::new(bot);

    handle_reminder_edit_text_core(&transport, peer_id, user_id, text, &dialogue, db).await
}

#[cfg(feature = "telegram-legacy")]
pub async fn handle_reminder_cancel(
    bot: Bot,
    q: CallbackQuery,
    dialogue: AppDialogue,
) -> HandlerResult {
    let Some((event_id, user_id, peer_id)) = callback_context(&q) else {
        return Ok(());
    };
    let transport = TelegramReminderTransport::new(bot);

    handle_reminder_cancel_core(&transport, &event_id, user_id, peer_id, &dialogue).await
}

#[cfg(feature = "telegram-legacy")]
pub async fn handle_list_command(bot: Bot, msg: Message, db: Db) -> HandlerResult {
    let transport = TelegramReminderTransport::new(bot);
    handle_list_command_core(&transport, msg.chat.id.0, db).await
}

#[cfg(feature = "telegram-legacy")]
pub async fn handle_delete_start(
    bot: Bot,
    q: CallbackQuery,
    dialogue: AppDialogue,
) -> HandlerResult {
    let Some((event_id, user_id, peer_id)) = callback_context(&q) else {
        return Ok(());
    };
    let transport = TelegramReminderTransport::new(bot);

    handle_delete_start_core(&transport, &event_id, user_id, peer_id, &dialogue).await
}

#[cfg(feature = "telegram-legacy")]
pub async fn handle_deletion_input(
    bot: Bot,
    msg: Message,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let text = match msg.text() {
        Some(t) => t,
        None => return Ok(()),
    };
    let peer_id = msg.chat.id.0;
    let user_id = message_user_id(&msg).unwrap_or(peer_id);
    let transport = TelegramReminderTransport::new(bot);

    handle_deletion_input_core(&transport, peer_id, user_id, text, &dialogue, db).await
}

#[cfg(feature = "telegram-legacy")]
pub async fn handle_delete_back(
    bot: Bot,
    q: CallbackQuery,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let Some((event_id, user_id, peer_id)) = callback_context(&q) else {
        return Ok(());
    };
    let transport = TelegramReminderTransport::new(bot);

    handle_delete_back_core(&transport, &event_id, user_id, peer_id, &dialogue, db).await
}

// ============================================================================
// Shared handler implementation
// ============================================================================

async fn handle_idle_text_core<T, S>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    text: &str,
    store: &S,
    db: Db,
) -> HandlerResult
where
    T: BotTransport,
    S: ReminderStateStore + Sync,
{
    let text = text.trim();

    if text.is_empty() || text.starts_with('/') || text == "Профиль" {
        return Ok(());
    }

    let user = match db.find_user(peer_id).await? {
        Some(u) => u,
        None => {
            transport
                .send_text(
                    peer_id,
                    "Пожалуйста, сначала настройте часовой пояс командой /start",
                )
                .await?;
            return Ok(());
        }
    };

    if !user_has_timezone(&user) {
        transport
            .send_text(
                peer_id,
                "Пожалуйста, сначала настройте часовой пояс командой /utc",
            )
            .await?;
        return Ok(());
    }

    let record = db.ensure_record(peer_id).await?;
    if !record.is_active() {
        let no_subscription_message = "⚠️ <b>Подписка не активна</b>\n\n\
            Для создания напоминаний необходима активная подписка.\n\n\
            Используйте команду /pay для оформления подписки.";

        send_html_text(transport, peer_id, no_subscription_message).await?;
        return Ok(());
    }

    start_reminder_creation_flow_core(transport, peer_id, user_id, text.to_string(), store).await
}

async fn start_reminder_creation_flow_core<T, S>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    text: String,
    store: &S,
) -> HandlerResult
where
    T: BotTransport,
    S: ReminderStateStore + Sync,
{
    let confirmation_text = format!(
        "📝 <b>Создать напоминание из этого текста?</b>\n\n\
        <i>{}</i>",
        html_escape(&text)
    );

    let keyboard = text_confirm_keyboard();
    send_html_with_keyboard(transport, peer_id, &confirmation_text, &keyboard).await?;

    let pending = PendingText { text };
    store
        .update_state(user_id, AppState::AwaitingTextConfirmation { pending })
        .await?;

    Ok(())
}

async fn handle_text_confirm_core<T, S>(
    transport: &T,
    event_id: &str,
    user_id: i64,
    peer_id: i64,
    store: &S,
    db: Db,
) -> HandlerResult
where
    T: BotTransport,
    S: ReminderStateStore + Sync,
{
    let state = store.get_state(user_id).await?;
    let pending_text = match state {
        AppState::AwaitingTextConfirmation { pending } => pending,
        _ => {
            transport
                .answer_callback(event_id, user_id, peer_id, None)
                .await?;
            return Ok(());
        }
    };

    transport
        .answer_callback(event_id, user_id, peer_id, None)
        .await?;
    transport
        .send_text(peer_id, "⏳ Анализирую текст напоминания...")
        .await?;

    let user = match db.find_user(peer_id).await? {
        Some(u) => u,
        None => {
            transport
                .send_text(peer_id, "❌ Пользователь не найден")
                .await?;
            store.update_state(user_id, AppState::Idle).await?;
            return Ok(());
        }
    };

    let llm_client = get_llm_client();
    let user_tz = get_user_timezone_str(&user);
    let user_dt = get_user_datetime_str(&user);
    let llm_response = match llm_client
        .parse_reminder(&pending_text.text, &user_tz, &user_dt)
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!("LLM API error: {}", e);
            transport
                .send_text(
                    peer_id,
                    "❌ Не удалось обработать текст. Попробуйте ещё раз.",
                )
                .await?;
            store.update_state(user_id, AppState::Idle).await?;
            return Ok(());
        }
    };

    if !llm_response.is_success() {
        let (error_code, error_msg) = llm_response
            .error
            .map(|e| (e.code, e.message))
            .unwrap_or_else(|| ("unknown".to_string(), "Неизвестная ошибка".to_string()));

        let response_text = if error_code == "ambiguous" {
            format!(
                "❓ Нужна дополнительная точность: {}\n\nУкажите дату или время более явно, и я попробую снова.",
                error_msg
            )
        } else {
            format!("❌ Не удалось распознать напоминание: {}", error_msg)
        };

        transport.send_text(peer_id, &response_text).await?;
        store.update_state(user_id, AppState::Idle).await?;
        return Ok(());
    }

    let parsed = match llm_response.reminder {
        Some(r) => r,
        None => {
            transport
                .send_text(
                    peer_id,
                    "❌ Не удалось распознать напоминание. Попробуйте ещё раз.",
                )
                .await?;
            store.update_state(user_id, AppState::Idle).await?;
            return Ok(());
        }
    };

    let prefs = user_time_prefs(&user);
    let reminder_time = match calculate_reminder_time(&parsed, Utc::now(), &prefs) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Time calculation error: {}", e);
            transport
                .send_text(peer_id, "❌ Не удалось определить время напоминания.")
                .await?;
            store.update_state(user_id, AppState::Idle).await?;
            return Ok(());
        }
    };

    let time_display = format_reminder_time(reminder_time, &user);
    let type_str = match parsed.reminder_type {
        ReminderType::OneTime => "разовое",
        ReminderType::Recurring => "повторяющееся",
    };

    let confirmation_text = format!(
        "📝 <b>Подтвердите напоминание:</b>\n\n\
        📌 <b>Текст:</b> {}\n\
        🕐 <b>Время:</b> {}\n\
        🔄 <b>Тип:</b> {}\n\n\
        Подтвердите создание или отредактируйте текст.",
        html_escape(&parsed.description),
        html_escape(&time_display),
        type_str
    );

    let keyboard = reminder_confirm_keyboard();
    send_html_with_keyboard(transport, peer_id, &confirmation_text, &keyboard).await?;

    let pending = PendingReminder {
        original_text: pending_text.text,
        description: parsed.description.clone(),
        time_display,
        parsed_json: serde_json::to_string(&parsed)?,
    };

    store
        .update_state(user_id, AppState::AwaitingReminderConfirmation { pending })
        .await?;

    Ok(())
}

async fn handle_text_cancel_core<T, S>(
    transport: &T,
    event_id: &str,
    user_id: i64,
    peer_id: i64,
    store: &S,
) -> HandlerResult
where
    T: BotTransport,
    S: ReminderStateStore + Sync,
{
    transport
        .answer_callback(event_id, user_id, peer_id, None)
        .await?;
    transport
        .send_text(peer_id, "❌ Создание напоминания отменено.")
        .await?;
    store.update_state(user_id, AppState::Idle).await?;

    Ok(())
}

async fn handle_reminder_confirm_core<T, S>(
    transport: &T,
    event_id: &str,
    user_id: i64,
    peer_id: i64,
    store: &S,
    db: Db,
) -> HandlerResult
where
    T: BotTransport,
    S: ReminderStateStore + Sync,
{
    let state = store.get_state(user_id).await?;
    let pending = match state {
        AppState::AwaitingReminderConfirmation { pending } => pending,
        _ => {
            transport
                .answer_callback(event_id, user_id, peer_id, None)
                .await?;
            return Ok(());
        }
    };

    let parsed: ParsedReminder = serde_json::from_str(&pending.parsed_json)?;
    let user = db
        .find_user(peer_id)
        .await?
        .unwrap_or_else(|| User::new(peer_id));
    let prefs = user_time_prefs(&user);
    let reminder_time = calculate_reminder_time(&parsed, Utc::now(), &prefs)?;

    let reminder = Reminder {
        chat_id: peer_id,
        text: parsed.description.clone(),
        delay: parsed.to_legacy_delay(),
        time: reminder_time,
        status: "active".to_string(),
        rem_id: None,
        messageID: None,
        snooze_time: None,
        retry_count: 0,
        retry_at: None,
    };

    let saved_reminder = db.insert_reminder(reminder).await?;

    transport
        .answer_callback(event_id, user_id, peer_id, None)
        .await?;

    let success_text = format!(
        "✅ <b>Напоминание создано!</b>\n\n\
        📌 {}\n\
        🕐 {}\n\
        🆔 #{}",
        html_escape(&parsed.description),
        html_escape(&pending.time_display),
        saved_reminder.rem_id.unwrap_or(0)
    );

    send_html_text(transport, peer_id, &success_text).await?;
    store.update_state(user_id, AppState::Idle).await?;

    Ok(())
}

async fn handle_reminder_edit_core<T, S>(
    transport: &T,
    event_id: &str,
    user_id: i64,
    peer_id: i64,
    store: &S,
) -> HandlerResult
where
    T: BotTransport,
    S: ReminderStateStore + Sync,
{
    let state = store.get_state(user_id).await?;
    let pending = match state {
        AppState::AwaitingReminderConfirmation { pending } => pending,
        _ => {
            transport
                .answer_callback(event_id, user_id, peer_id, None)
                .await?;
            return Ok(());
        }
    };

    transport
        .answer_callback(event_id, user_id, peer_id, None)
        .await?;

    let keyboard = reminder_edit_keyboard();
    let text = format!(
        "✏️ Введите новый текст напоминания:\n\n<i>Текущий:</i> {}",
        html_escape(&pending.original_text)
    );

    send_html_with_keyboard(transport, peer_id, &text, &keyboard).await?;
    store
        .update_state(user_id, AppState::AwaitingReminderEdit { pending })
        .await?;

    Ok(())
}

async fn handle_reminder_edit_text_core<T, S>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    text: &str,
    store: &S,
    db: Db,
) -> HandlerResult
where
    T: BotTransport,
    S: ReminderStateStore + Sync,
{
    let new_text = text.trim();

    if new_text.is_empty() {
        transport
            .send_text(peer_id, "Текст не может быть пустым. Введите новый текст:")
            .await?;
        return Ok(());
    }

    store.update_state(user_id, AppState::Idle).await?;
    handle_idle_text_core(transport, peer_id, user_id, new_text, store, db).await
}

async fn handle_reminder_cancel_core<T, S>(
    transport: &T,
    event_id: &str,
    user_id: i64,
    peer_id: i64,
    store: &S,
) -> HandlerResult
where
    T: BotTransport,
    S: ReminderStateStore + Sync,
{
    transport
        .answer_callback(event_id, user_id, peer_id, None)
        .await?;
    transport
        .send_text(peer_id, "❌ Создание напоминания отменено.")
        .await?;
    store.update_state(user_id, AppState::Idle).await?;

    Ok(())
}

async fn handle_list_command_core<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    db: Db,
) -> HandlerResult {
    let reminders = db.get_user_reminders(peer_id).await?;

    if reminders.is_empty() {
        transport
            .send_text(peer_id, "📭 У вас нет активных напоминаний.")
            .await?;
        return Ok(());
    }

    let user = db
        .find_user(peer_id)
        .await?
        .unwrap_or_else(|| User::new(peer_id));
    let text = format_reminders_list(&reminders, &user);
    let keyboard = list_delete_keyboard();

    send_html_with_keyboard(transport, peer_id, &text, &keyboard).await?;

    Ok(())
}

async fn handle_delete_start_core<T, S>(
    transport: &T,
    event_id: &str,
    user_id: i64,
    peer_id: i64,
    store: &S,
) -> HandlerResult
where
    T: BotTransport,
    S: ReminderStateStore + Sync,
{
    transport
        .answer_callback(event_id, user_id, peer_id, None)
        .await?;

    let keyboard = delete_keyboard();
    transport
        .send_with_keyboard(
            peer_id,
            "🗑 Введите номер напоминания для удаления:",
            &keyboard,
        )
        .await?;

    store
        .update_state(user_id, AppState::AwaitingReminderDeletion)
        .await?;

    Ok(())
}

async fn handle_deletion_input_core<T, S>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    text: &str,
    store: &S,
    db: Db,
) -> HandlerResult
where
    T: BotTransport,
    S: ReminderStateStore + Sync,
{
    let number: usize = match text.trim().parse() {
        Ok(n) if n > 0 => n,
        _ => {
            transport
                .send_text(
                    peer_id,
                    "❌ Введите корректный номер напоминания (число больше 0):",
                )
                .await?;
            return Ok(());
        }
    };

    let reminders = db.get_user_reminders(peer_id).await?;

    if number > reminders.len() {
        transport
            .send_text(
                peer_id,
                &format!(
                    "❌ Напоминание с номером {} не найдено. У вас {} напоминаний.",
                    number,
                    reminders.len()
                ),
            )
            .await?;
        return Ok(());
    }

    let reminder_to_delete = &reminders[number - 1];
    let rem_id = reminder_to_delete.rem_id.unwrap_or(0);
    let deleted = db.delete_reminder(peer_id, rem_id).await?;

    if deleted {
        transport
            .send_text(
                peer_id,
                &format!(
                    "✅ Напоминание #{} \"{}\" удалено.",
                    number, reminder_to_delete.text
                ),
            )
            .await?;
    } else {
        transport
            .send_text(
                peer_id,
                "❌ Не удалось удалить напоминание. Попробуйте ещё раз.",
            )
            .await?;
    }

    store.update_state(user_id, AppState::Idle).await?;

    Ok(())
}

async fn handle_delete_back_core<T, S>(
    transport: &T,
    event_id: &str,
    user_id: i64,
    peer_id: i64,
    store: &S,
    db: Db,
) -> HandlerResult
where
    T: BotTransport,
    S: ReminderStateStore + Sync,
{
    transport
        .answer_callback(event_id, user_id, peer_id, None)
        .await?;

    store.update_state(user_id, AppState::Idle).await?;

    let reminders = db.get_user_reminders(peer_id).await?;
    if !reminders.is_empty() {
        let user = db
            .find_user(peer_id)
            .await?
            .unwrap_or_else(|| User::new(peer_id));
        let text = format_reminders_list(&reminders, &user);
        let keyboard = list_delete_keyboard();

        send_html_with_keyboard(transport, peer_id, &text, &keyboard).await?;
    }

    Ok(())
}

// ============================================================================
// Helper functions
// ============================================================================

#[cfg(feature = "telegram-legacy")]
fn message_user_id(msg: &Message) -> Option<i64> {
    msg.from.as_ref().map(|user| user.id.0 as i64)
}

#[cfg(feature = "telegram-legacy")]
fn callback_context(q: &CallbackQuery) -> Option<(String, i64, i64)> {
    let peer_id = q.message.as_ref()?.chat().id.0;
    let user_id = q.from.id.0 as i64;

    Some((q.id.0.clone(), user_id, peer_id))
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

fn format_reminders_list(reminders: &[Reminder], user: &User) -> String {
    let mut text = String::from("📌 <b>Активные напоминания</b>\n");
    let mut current_month: Option<(i32, u32)> = None;

    for (index, reminder) in reminders.iter().enumerate() {
        let local_time = convert_to_user_tz(reminder.time, user);
        let year = local_time.year();
        let month = local_time.month();

        if current_month != Some((year, month)) {
            current_month = Some((year, month));
            let month_name = get_russian_month_name(month);
            text.push_str(&format!("\n📅 <b>{} {} г.</b>\n", month_name, year));
        }

        let day = local_time.day();
        let weekday = get_russian_weekday_short(local_time.weekday());
        let time_str = format!("{:02}:{:02}", local_time.hour(), local_time.minute());
        let escaped_text = html_escape(&reminder.text);

        text.push_str(&format!(
            "{}. {:02}.{:02} ({}, {}) ▹ {}\n",
            index + 1,
            day,
            month,
            weekday,
            time_str,
            escaped_text
        ));
    }

    text
}

fn user_time_prefs(user: &User) -> UserTimePrefs {
    UserTimePrefs::from_db(
        &user.morning,
        &user.afternoon,
        &user.evening,
        &user.time_zone,
        &user.utc,
    )
}

/// Format reminder time in user's timezone for display.
fn format_reminder_time(time: DateTime<Utc>, user: &User) -> String {
    let local = user_local_time(user, time);
    let weekday = get_russian_weekday(local.weekday());

    format!(
        "{:02}.{:02}.{} ({}) в {:02}:{:02}",
        local.day(),
        local.month(),
        local.year(),
        weekday,
        local.hour(),
        local.minute()
    )
}

/// Get user's timezone as string for LLM API (e.g., "+07:00").
fn get_user_timezone_str(user: &User) -> String {
    user_offset_string_at(user, Utc::now())
}

/// Get user's current datetime string for LLM API (e.g., "2025-12-05 00:42").
fn get_user_datetime_str(user: &User) -> String {
    user_datetime_string(user, Utc::now())
}

/// Convert UTC time to user's local time (for display purposes).
fn convert_to_user_tz(time: DateTime<Utc>, user: &User) -> chrono::DateTime<chrono::FixedOffset> {
    user_local_time(user, time)
}

fn get_russian_weekday(wd: chrono::Weekday) -> &'static str {
    match wd {
        chrono::Weekday::Mon => "понедельник",
        chrono::Weekday::Tue => "вторник",
        chrono::Weekday::Wed => "среда",
        chrono::Weekday::Thu => "четверг",
        chrono::Weekday::Fri => "пятница",
        chrono::Weekday::Sat => "суббота",
        chrono::Weekday::Sun => "воскресенье",
    }
}

fn get_russian_weekday_short(wd: chrono::Weekday) -> &'static str {
    match wd {
        chrono::Weekday::Mon => "пн",
        chrono::Weekday::Tue => "вт",
        chrono::Weekday::Wed => "ср",
        chrono::Weekday::Thu => "чт",
        chrono::Weekday::Fri => "пт",
        chrono::Weekday::Sat => "сб",
        chrono::Weekday::Sun => "вс",
    }
}

fn get_russian_month_name(month: u32) -> &'static str {
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
        _ => "Неизвестно",
    }
}

/// Escape special characters for HTML.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
