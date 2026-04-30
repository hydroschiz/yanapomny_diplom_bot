//! Reminder creation and management handlers.

use chrono::{DateTime, Datelike, Timelike, Utc};
use teloxide::prelude::*;
use teloxide::types::ParseMode;

use crate::api::db::{Db, Reminder, User};
use crate::api::llm_client::LlmClient;
use crate::api::llm_models::{ParsedReminder, ReminderType};
use crate::api::time_calculator::{calculate_reminder_time, UserTimePrefs};
use crate::bot::keyboards::{
    delete_keyboard, list_delete_keyboard, reminder_confirm_keyboard, reminder_edit_keyboard,
    text_confirm_keyboard,
};
use crate::bot::router::{AppDialogue, HandlerResult};
use crate::bot::states::{AppState, PendingReminder, PendingText};
use crate::utils::timezone::{
    user_datetime_string, user_has_timezone, user_local_time, user_offset_string_at,
};

/// LLM client shared across handlers.
static LLM_CLIENT: std::sync::OnceLock<LlmClient> = std::sync::OnceLock::new();

fn get_llm_client() -> &'static LlmClient {
    LLM_CLIENT.get_or_init(|| {
        LlmClient::from_env().expect("Failed to create LLM client")
    })
}

/// Handle any text message in Idle state - ask for confirmation BEFORE sending to LLM.
pub async fn handle_idle_text(
    bot: Bot,
    msg: Message,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let chat_id = msg.chat.id;
    let text = match msg.text() {
        Some(t) => t.trim(),
        None => return Ok(()), // Ignore non-text messages
    };

    // Skip empty messages
    if text.is_empty() {
        return Ok(());
    }

    // Skip if it looks like a command
    if text.starts_with('/') {
        return Ok(());
    }

    // Skip "Профиль" button (will be handled separately later)
    if text == "Профиль" {
        return Ok(());
    }

    // Get user to check if timezone is set
    let user = match db.find_user(chat_id.0).await? {
        Some(u) => u,
        None => {
            bot.send_message(chat_id, "Пожалуйста, сначала настройте часовой пояс командой /start")
                .await?;
            return Ok(());
        }
    };

    // Check if timezone is set
    if !user_has_timezone(&user) {
        bot.send_message(chat_id, "Пожалуйста, сначала настройте часовой пояс командой /utc")
            .await?;
        return Ok(());
    }

    // Fresh user should immediately get a trial/default record after onboarding.
    let record = db.ensure_record(chat_id.0).await?;
    if !record.is_active() {
        let no_subscription_message = 
            "⚠️ <b>Подписка не активна</b>\n\n\
            Для создания напоминаний необходима активная подписка.\n\n\
            Используйте команду /pay для оформления подписки.";
        
        bot.send_message(chat_id, no_subscription_message)
            .parse_mode(ParseMode::Html)
            .await?;
        return Ok(());
    }

    start_reminder_creation_flow(bot, chat_id, text.to_string(), dialogue).await
}

/// Start the flow of creating a reminder (confirmation -> LLM).
/// Assumes subscription and user checks are already done.
pub async fn start_reminder_creation_flow(
    bot: Bot,
    chat_id: ChatId,
    text: String,
    dialogue: AppDialogue,
) -> HandlerResult {
    // Ask for confirmation BEFORE sending to LLM
    let confirmation_text = format!(
        "📝 <b>Создать напоминание из этого текста?</b>\n\n\
        <i>{}</i>",
        html_escape(&text)
    );

    let keyboard = text_confirm_keyboard();

    bot.send_message(chat_id, confirmation_text)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;

    // Save text in dialogue state
    let pending = PendingText { text };
    dialogue.update(AppState::AwaitingTextConfirmation { pending }).await?;

    Ok(())
}

/// Handle text confirmation - now send to LLM and show parsed result.
pub async fn handle_text_confirm(
    bot: Bot,
    q: CallbackQuery,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let msg = match q.message.as_ref() {
        Some(m) => m,
        None => return Ok(()),
    };
    let chat_id = msg.chat().id;

    let state = dialogue.get().await?.unwrap_or_default();
    let pending_text = match state {
        AppState::AwaitingTextConfirmation { pending } => pending,
        _ => {
            bot.answer_callback_query(q.id.clone()).await?;
            return Ok(());
        }
    };

    bot.answer_callback_query(q.id.clone()).await?;

    // Edit message to show processing
    bot.edit_message_text(chat_id, msg.id(), "⏳ Анализирую текст напоминания...")
        .await?;

    // Get user for timezone
    let user = match db.find_user(chat_id.0).await? {
        Some(u) => u,
        None => {
            bot.edit_message_text(chat_id, msg.id(), "❌ Пользователь не найден")
                .await?;
            dialogue.update(AppState::Idle).await?;
            return Ok(());
        }
    };

    // Call LLM API with user's timezone context
    let llm_client = get_llm_client();
    let user_tz = get_user_timezone_str(&user);
    let user_dt = get_user_datetime_str(&user);
    let llm_response = match llm_client.parse_reminder(&pending_text.text, &user_tz, &user_dt).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!("LLM API error: {}", e);
            bot.edit_message_text(
                chat_id,
                msg.id(),
                "❌ Не удалось обработать текст. Попробуйте ещё раз.",
            ).await?;
            dialogue.update(AppState::Idle).await?;
            return Ok(());
        }
    };

    // Check if parsing was successful
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

        bot.edit_message_text(
            chat_id,
            msg.id(),
            response_text,
        ).await?;
        dialogue.update(AppState::Idle).await?;
        return Ok(());
    }

    let parsed = match llm_response.reminder {
        Some(r) => r,
        None => {
            bot.edit_message_text(
                chat_id,
                msg.id(),
                "❌ Не удалось распознать напоминание. Попробуйте ещё раз.",
            ).await?;
            dialogue.update(AppState::Idle).await?;
            return Ok(());
        }
    };

    // Calculate reminder time
    let prefs = user_time_prefs(&user);
    let reminder_time = match calculate_reminder_time(&parsed, Utc::now(), &prefs) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("Time calculation error: {}", e);
            bot.edit_message_text(
                chat_id,
                msg.id(),
                "❌ Не удалось определить время напоминания.",
            ).await?;
            dialogue.update(AppState::Idle).await?;
            return Ok(());
        }
    };

    // Format time for display
    let time_display = format_reminder_time(reminder_time, &user);
    let type_str = match parsed.reminder_type {
        ReminderType::OneTime => "разовое",
        ReminderType::Recurring => "повторяющееся",
    };

    // Show parsed result with confirmation
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

    bot.edit_message_text(chat_id, msg.id(), confirmation_text)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;

    // Save pending reminder in dialogue state
    let pending = PendingReminder {
        original_text: pending_text.text,
        description: parsed.description.clone(),
        time_display,
        parsed_json: serde_json::to_string(&parsed)?,
    };

    dialogue.update(AppState::AwaitingReminderConfirmation { pending }).await?;

    Ok(())
}

/// Handle text cancel - user doesn't want to create reminder.
pub async fn handle_text_cancel(
    bot: Bot,
    q: CallbackQuery,
    dialogue: AppDialogue,
) -> HandlerResult {
    let msg = match q.message.as_ref() {
        Some(m) => m,
        None => return Ok(()),
    };
    let chat_id = msg.chat().id;

    bot.answer_callback_query(q.id.clone()).await?;
    
    // Delete the confirmation message
    let _ = bot.delete_message(chat_id, msg.id()).await;

    dialogue.update(AppState::Idle).await?;

    Ok(())
}

/// Handle reminder confirmation callback.
pub async fn handle_reminder_confirm(
    bot: Bot,
    q: CallbackQuery,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let chat_id = match q.message.as_ref() {
        Some(m) => m.chat().id,
        None => return Ok(()),
    };

    let state = dialogue.get().await?.unwrap_or_default();
    let pending = match state {
        AppState::AwaitingReminderConfirmation { pending } => pending,
        _ => {
            bot.answer_callback_query(q.id.clone()).await?;
            return Ok(());
        }
    };

    // Parse the saved reminder JSON
    let parsed: ParsedReminder = serde_json::from_str(&pending.parsed_json)?;

    // Get user for time preferences
    let user = db.find_user(chat_id.0).await?.unwrap_or_else(|| User::new(chat_id.0));
    let prefs = user_time_prefs(&user);

    // Calculate reminder time
    let reminder_time = calculate_reminder_time(&parsed, Utc::now(), &prefs)?;

    // Create reminder in database
    let reminder = Reminder {
        chat_id: chat_id.0,
        text: parsed.description.clone(),
        delay: parsed.to_legacy_delay(),
        time: reminder_time,
        status: "active".to_string(),
        rem_id: None, // Will be set by insert_reminder
        messageID: None,
        snooze_time: None,
        retry_count: 0,
        retry_at: None,
    };

    let saved_reminder = db.insert_reminder(reminder).await?;

    // Answer callback
    bot.answer_callback_query(q.id.clone()).await?;

    // Delete confirmation message
    if let Some(msg) = &q.message {
        let _ = bot.delete_message(chat_id, msg.id()).await;
    }

    // Send success message (use HTML for simpler escaping)
    let success_text = format!(
        "✅ <b>Напоминание создано!</b>\n\n\
        📌 {}\n\
        🕐 {}\n\
        🆔 #{}",
        html_escape(&parsed.description),
        html_escape(&pending.time_display),
        saved_reminder.rem_id.unwrap_or(0)
    );

    bot.send_message(chat_id, success_text)
        .parse_mode(ParseMode::Html)
        .await?;

    // Reset dialogue to Idle
    dialogue.update(AppState::Idle).await?;

    Ok(())
}

/// Handle reminder edit callback - ask for new text.
pub async fn handle_reminder_edit(
    bot: Bot,
    q: CallbackQuery,
    dialogue: AppDialogue,
) -> HandlerResult {
    let chat_id = match q.message.as_ref() {
        Some(m) => m.chat().id,
        None => return Ok(()),
    };

    let state = dialogue.get().await?.unwrap_or_default();
    let pending = match state {
        AppState::AwaitingReminderConfirmation { pending } => pending,
        _ => {
            bot.answer_callback_query(q.id.clone()).await?;
            return Ok(());
        }
    };

    bot.answer_callback_query(q.id.clone()).await?;

    // Delete confirmation message
    if let Some(msg) = &q.message {
        let _ = bot.delete_message(chat_id, msg.id()).await;
    }

    // Ask for new text
    let keyboard = reminder_edit_keyboard();

    bot.send_message(
        chat_id,
        format!(
            "✏️ Введите новый текст напоминания:\n\n<i>Текущий:</i> {}",
            html_escape(&pending.original_text)
        ),
    )
    .parse_mode(ParseMode::Html)
    .reply_markup(keyboard)
    .await?;

    dialogue.update(AppState::AwaitingReminderEdit { pending }).await?;

    Ok(())
}

/// Handle edited reminder text.
pub async fn handle_reminder_edit_text(
    bot: Bot,
    msg: Message,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let chat_id = msg.chat.id;
    let new_text = match msg.text() {
        Some(t) => t.trim(),
        None => return Ok(()),
    };

    if new_text.is_empty() {
        bot.send_message(chat_id, "Текст не может быть пустым. Введите новый текст:").await?;
        return Ok(());
    }

    // Reset to Idle and process as new reminder
    dialogue.update(AppState::Idle).await?;

    // Re-process the new text
    handle_idle_text(bot, msg, dialogue, db).await
}

/// Handle reminder cancellation.
pub async fn handle_reminder_cancel(
    bot: Bot,
    q: CallbackQuery,
    dialogue: AppDialogue,
) -> HandlerResult {
    let chat_id = match q.message.as_ref() {
        Some(m) => m.chat().id,
        None => return Ok(()),
    };

    bot.answer_callback_query(q.id.clone()).await?;

    // Delete message
    if let Some(msg) = &q.message {
        let _ = bot.delete_message(chat_id, msg.id()).await;
    }

    bot.send_message(chat_id, "❌ Создание напоминания отменено.").await?;

    dialogue.update(AppState::Idle).await?;

    Ok(())
}

// ============================================================================
// /list command
// ============================================================================

/// Handle /list command - show user's reminders.
pub async fn handle_list_command(
    bot: Bot,
    msg: Message,
    db: Db,
) -> HandlerResult {
    let chat_id = msg.chat.id;

    let reminders = db.get_user_reminders(chat_id.0).await?;

    if reminders.is_empty() {
        bot.send_message(chat_id, "📭 У вас нет активных напоминаний.").await?;
        return Ok(());
    }

    // Get user for timezone
    let user = db.find_user(chat_id.0).await?.unwrap_or_else(|| User::new(chat_id.0));

    // Group reminders by month
    let mut text = String::from("📌 <b>Активные напоминания</b>\n");
    let mut current_month: Option<(i32, u32)> = None;
    let mut index = 1;

    for reminder in &reminders {
        let local_time = convert_to_user_tz(reminder.time, &user);
        let year = local_time.year();
        let month = local_time.month();

        // Add month header if changed
        if current_month != Some((year, month)) {
            current_month = Some((year, month));
            let month_name = get_russian_month_name(month);
            text.push_str(&format!("\n📅 <b>{} {} г.</b>\n", month_name, year));
        }

        // Format reminder line
        let day = local_time.day();
        let weekday = get_russian_weekday_short(local_time.weekday());
        let time_str = format!("{:02}:{:02}", local_time.hour(), local_time.minute());
        let escaped_text = html_escape(&reminder.text);

        text.push_str(&format!(
            "{}. {:02}.{:02} ({}, {}) ▹ {}\n",
            index,
            day,
            month,
            weekday,
            time_str,
            escaped_text
        ));

        index += 1;
    }

    // Add delete button
    let keyboard = list_delete_keyboard();

    bot.send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}

/// Handle delete button press - start deletion flow.
pub async fn handle_delete_start(
    bot: Bot,
    q: CallbackQuery,
    dialogue: AppDialogue,
) -> HandlerResult {
    let chat_id = match q.message.as_ref() {
        Some(m) => m.chat().id,
        None => return Ok(()),
    };

    bot.answer_callback_query(q.id.clone()).await?;

    let keyboard = delete_keyboard();

    bot.send_message(
        chat_id,
        "🗑 Введите номер напоминания для удаления:",
    )
    .reply_markup(keyboard)
    .await?;

    dialogue.update(AppState::AwaitingReminderDeletion).await?;

    Ok(())
}

/// Handle deletion number input.
pub async fn handle_deletion_input(
    bot: Bot,
    msg: Message,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let chat_id = msg.chat.id;
    let text = match msg.text() {
        Some(t) => t.trim(),
        None => return Ok(()),
    };

    // Parse number
    let number: usize = match text.parse() {
        Ok(n) if n > 0 => n,
        _ => {
            bot.send_message(chat_id, "❌ Введите корректный номер напоминания (число больше 0):").await?;
            return Ok(());
        }
    };

    // Get user's reminders
    let reminders = db.get_user_reminders(chat_id.0).await?;

    if number > reminders.len() {
        bot.send_message(
            chat_id,
            format!("❌ Напоминание с номером {} не найдено. У вас {} напоминаний.", number, reminders.len()),
        ).await?;
        return Ok(());
    }

    // Get the reminder to delete
    let reminder_to_delete = &reminders[number - 1];
    let rem_id = reminder_to_delete.rem_id.unwrap_or(0);

    // Delete reminder
    let deleted = db.delete_reminder(chat_id.0, rem_id).await?;

    if deleted {
        bot.send_message(
            chat_id,
            format!("✅ Напоминание #{} \"{}\" удалено.", number, reminder_to_delete.text),
        ).await?;
    } else {
        bot.send_message(chat_id, "❌ Не удалось удалить напоминание. Попробуйте ещё раз.").await?;
    }

    dialogue.update(AppState::Idle).await?;

    Ok(())
}

/// Handle back button in deletion flow.
pub async fn handle_delete_back(
    bot: Bot,
    q: CallbackQuery,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let chat_id = match q.message.as_ref() {
        Some(m) => m.chat().id,
        None => return Ok(()),
    };

    bot.answer_callback_query(q.id.clone()).await?;

    // Delete the deletion prompt message
    if let Some(msg) = &q.message {
        let _ = bot.delete_message(chat_id, msg.id()).await;
    }

    // Reset to Idle
    dialogue.update(AppState::Idle).await?;

    // Show list again
    let reminders = db.get_user_reminders(chat_id.0).await?;
    if !reminders.is_empty() {
        // Get user for timezone
        let user = db.find_user(chat_id.0).await?.unwrap_or_else(|| User::new(chat_id.0));

        // Group reminders by month
        let mut text = String::from("📌 <b>Активные напоминания</b>\n");
        let mut current_month: Option<(i32, u32)> = None;
        let mut index = 1;

        for reminder in &reminders {
            let local_time = convert_to_user_tz(reminder.time, &user);
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
                index, day, month, weekday, time_str, escaped_text
            ));

            index += 1;
        }

        let keyboard = list_delete_keyboard();

        bot.send_message(chat_id, text)
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
    }

    Ok(())
}

// ============================================================================
// Helper functions
// ============================================================================

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
