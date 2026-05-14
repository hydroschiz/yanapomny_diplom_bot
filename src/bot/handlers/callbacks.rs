use std::sync::Arc;

use teloxide::prelude::*;

use crate::api::db::Db;
use crate::api::payments::PaymentService;
use crate::bot::{
    keyboards::{back_keyboard, setup_keyboard, utc_keyboard_page, utc_keyboard_page_count},
    router::{AppDialogue, HandlerResult},
    states::AppState,
};
use crate::transport::adapters::TelegramTransport;

use super::commands::{
    start_utc_flow, AUTO_SNOOZE_PROMPT, SETUP_PROMPT, SNOOZE_PROMPT, UTC_SUCCESS_MESSAGE,
};
use super::text::{human_readable_auto, human_readable_snooze, normalize_offset};

pub async fn handle_callback(
    bot: Bot,
    cq: CallbackQuery,
    dialogue: AppDialogue,
    db: Db,
    payment_svc: Arc<PaymentService>,
) -> HandlerResult {
    let data = if let Some(data) = cq.data.clone() {
        data
    } else {
        return Ok(());
    };
    let chat_id = if let Some(msg) = &cq.message {
        msg.chat().id
    } else {
        return Ok(());
    };

    match data.as_str() {
        "setup_menu" => {
            dialogue.update(AppState::Idle).await?;
            db.update_user_state(chat_id.0, "waiting_for_message")
                .await?;
            bot.answer_callback_query(cq.id).await?;
            bot.send_message(chat_id, SETUP_PROMPT)
                .parse_mode(teloxide::types::ParseMode::Html)
                .reply_markup(setup_keyboard())
                .await?;
            return Ok(());
        }
        "setup_snooze" => {
            let user = db.ensure_user(chat_id.0).await?;
            dialogue.update(AppState::AwaitingSnoozeButtons).await?;
            db.update_user_state(chat_id.0, "waiting_for_time").await?;
            bot.answer_callback_query(cq.id).await?;
            let current = if user.snooze_buttons.is_empty() {
                "15 мин, 1 час, 3 часа".to_string()
            } else {
                user.snooze_buttons
                    .iter()
                    .filter_map(|c| human_readable_snooze(c))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let text = format!("{SNOOZE_PROMPT}\n\nТекущие: <b>{current}</b>");
            bot.send_message(chat_id, text)
                .parse_mode(teloxide::types::ParseMode::Html)
                .reply_markup(back_keyboard())
                .await?;
            return Ok(());
        }
        "setup_auto" => {
            let user = db.ensure_user(chat_id.0).await?;
            dialogue.update(AppState::AwaitingAutoSnooze).await?;
            db.update_user_state(chat_id.0, "waiting_for_time").await?;
            bot.answer_callback_query(cq.id).await?;
            let current = if user.auto_snooze.is_empty() {
                "15 мин".to_string()
            } else {
                human_readable_auto(&user.auto_snooze)
            };
            let text = format!("{AUTO_SNOOZE_PROMPT}\n\nТекущее: <b>{current}</b>");
            bot.send_message(chat_id, text)
                .parse_mode(teloxide::types::ParseMode::Html)
                .reply_markup(back_keyboard())
                .await?;
            return Ok(());
        }
        "setup_utc" => {
            bot.answer_callback_query(cq.id).await?;
            start_utc_flow(bot, chat_id, dialogue, db).await?;
            return Ok(());
        }
        _ => {}
    }

    if data == "utc_cancel" {
        dialogue.update(AppState::Idle).await?;
        bot.answer_callback_query(cq.id).await?;
        bot.send_message(chat_id, "Настройка часового пояса отменена.")
            .await?;
        return Ok(());
    }

    if let Some(rest) = data.strip_prefix("utc_page:") {
        if let Ok(page) = rest.parse::<usize>() {
            bot.answer_callback_query(cq.id).await?;
            let page_count = utc_keyboard_page_count();
            bot.send_message(
                chat_id,
                format!(
                    "Выберите UTC смещение кнопкой или отправьте город/смещение текстом.\n\nСтраница {}/{}",
                    page % page_count + 1,
                    page_count
                ),
            )
            .reply_markup(utc_keyboard_page(page))
            .await?;
            return Ok(());
        }
    }

    if let Some(rest) = data.strip_prefix("utc_set:") {
        if let Some(offset) = normalize_offset(rest) {
            db.update_utc_and_clear_timezone(chat_id.0, &offset).await?;
            db.update_user_state(chat_id.0, "waiting_for_message")
                .await?;
            dialogue.update(AppState::Idle).await?;

            bot.answer_callback_query(cq.id).await?;
            bot.send_message(chat_id, UTC_SUCCESS_MESSAGE.replace("+3:00", &offset))
                .parse_mode(teloxide::types::ParseMode::Html)
                .await?;
            return Ok(());
        }
    }

    // Handle pay_* callbacks
    if data.starts_with("pay_") {
        return super::pay::handle_pay_callback(bot, cq, dialogue, db, payment_svc).await;
    }

    // Handle text confirmation callbacks (before LLM)
    match data.as_str() {
        "text_confirm" => {
            return super::reminder::handle_text_confirm(bot, cq, dialogue, db).await;
        }
        "text_cancel" => {
            return super::reminder::handle_text_cancel(bot, cq, dialogue).await;
        }
        _ => {}
    }

    // Handle reminder callbacks (after LLM)
    match data.as_str() {
        "reminder_confirm" => {
            return super::reminder::handle_reminder_confirm(bot, cq, dialogue, db).await;
        }
        "reminder_edit" => {
            return super::reminder::handle_reminder_edit(bot, cq, dialogue).await;
        }
        "reminder_cancel" => {
            return super::reminder::handle_reminder_cancel(bot, cq, dialogue).await;
        }
        "reminder_delete_start" => {
            return super::reminder::handle_delete_start(bot, cq, dialogue).await;
        }
        "reminder_delete_back" => {
            return super::reminder::handle_delete_back(bot, cq, dialogue, db).await;
        }
        _ => {}
    }

    // Handle channel subscription callbacks
    match data.as_str() {
        "sub_delete" => {
            return super::channels::handle_sub_delete_callback(bot, cq, dialogue, db).await;
        }
        "subs" => {
            return super::channels::handle_subs_callback(bot, cq, db).await;
        }
        "profile" | "profile_stub" => {
            // Redirect to profile
            bot.answer_callback_query(cq.id).await?;
            if let Some(msg) = cq.message {
                if let Some(regular_msg) = msg.regular_message().cloned() {
                    return super::profile::handle_profile_command(bot, regular_msg, db).await;
                }
            }
            return Ok(());
        }
        "profile_list" => {
            // Show reminders list
            bot.answer_callback_query(cq.id).await?;
            if let Some(msg) = cq.message {
                if let Some(regular_msg) = msg.regular_message().cloned() {
                    return super::reminder::handle_list_command(bot, regular_msg, db).await;
                }
            }
            return Ok(());
        }
        "profile_setup" => {
            // Show setup menu
            dialogue.update(AppState::Idle).await?;
            bot.answer_callback_query(cq.id).await?;
            bot.send_message(chat_id, SETUP_PROMPT)
                .parse_mode(teloxide::types::ParseMode::Html)
                .reply_markup(setup_keyboard())
                .await?;
            return Ok(());
        }
        "profile_subs" => {
            // Show channel subscriptions
            bot.answer_callback_query(cq.id).await?;
            if let Some(msg) = cq.message {
                if let Some(regular_msg) = msg.regular_message().cloned() {
                    return super::channels::command_subs(bot, regular_msg, db).await;
                }
            }
            return Ok(());
        }
        "profile_referral" => {
            // Show referral program
            bot.answer_callback_query(cq.id).await?;
            let transport = TelegramTransport::new(bot);
            return super::referral::send_referral_message(&transport, chat_id.0, chat_id.0, &db)
                .await;
        }
        "profile_pay" => {
            // Show payment menu
            bot.answer_callback_query(cq.id).await?;
            if let Some(msg) = cq.message {
                if let Some(regular_msg) = msg.regular_message().cloned() {
                    return super::pay::command_pay(bot, regular_msg, dialogue, db, payment_svc)
                        .await;
                }
            }
            return Ok(());
        }
        "back_main" => {
            dialogue.update(AppState::Idle).await?;
            bot.answer_callback_query(cq.id).await?;
            bot.send_message(chat_id, "Хорошо! Напиши мне, что нужно запомнить 📝")
                .await?;
            return Ok(());
        }
        _ => {}
    }

    // Handle snooze callbacks: snooze:{rem_id}:{code}
    if let Some(rest) = data.strip_prefix("snooze:") {
        return handle_snooze_callback(bot, cq, db, rest).await;
    }

    // Handle reminder done callbacks: reminder_done:{rem_id}
    if let Some(rest) = data.strip_prefix("reminder_done:") {
        return handle_reminder_done_callback(bot, cq, db, rest).await;
    }

    // Handle reminder list callback
    if data == "reminder_list" {
        return super::reminder::handle_list_command(
            bot,
            cq.message.unwrap().regular_message().cloned().unwrap(),
            db,
        )
        .await;
    }

    // Unknown callback data: send quick error.
    bot.answer_callback_query(cq.id)
        .text("Не удалось обработать выбор. Попробуйте снова.")
        .await?;
    Ok(())
}

/// Handle snooze callback: snooze:{rem_id}:{code}
async fn handle_snooze_callback(bot: Bot, cq: CallbackQuery, db: Db, data: &str) -> HandlerResult {
    use crate::bot::keyboards::{
        reminder_snoozed_keyboard, snooze_code_to_label, snooze_code_to_minutes,
    };
    use crate::scheduler::format_full_reminder_time_for_user;

    let chat_id = match cq.message {
        Some(ref m) => m.chat().id,
        None => return Ok(()),
    };

    // Parse rem_id and snooze code
    let parts: Vec<&str> = data.split(':').collect();
    if parts.len() != 2 {
        bot.answer_callback_query(cq.id)
            .text("Ошибка: неверный формат")
            .await?;
        return Ok(());
    }

    let rem_id: i32 = match parts[0].parse() {
        Ok(id) => id,
        Err(_) => {
            bot.answer_callback_query(cq.id)
                .text("Ошибка: неверный ID напоминания")
                .await?;
            return Ok(());
        }
    };

    let snooze_code = parts[1];
    let minutes = match snooze_code_to_minutes(snooze_code) {
        Some(m) => m,
        None => {
            bot.answer_callback_query(cq.id)
                .text("Ошибка: неверный интервал")
                .await?;
            return Ok(());
        }
    };

    // Get reminder to get text
    let reminder = match db.find_reminder(rem_id).await? {
        Some(r) => r,
        None => {
            bot.answer_callback_query(cq.id)
                .text("Напоминание не найдено")
                .await?;
            return Ok(());
        }
    };

    // Snooze the reminder
    let new_time = db.snooze_reminder(rem_id, minutes).await?;

    // Get user for timezone
    let user = db.ensure_user(chat_id.0).await?;
    let time_display = format_full_reminder_time_for_user(&new_time, &user);
    let snooze_label = snooze_code_to_label(snooze_code);

    // Build snoozed message
    let message = format!(
        "Напоминание отложено на {} ✅\n\n\
         📅 {} ▹ {}",
        snooze_label,
        time_display,
        crate::scheduler::html_escape(&reminder.text)
    );

    let keyboard = reminder_snoozed_keyboard(rem_id);

    // Edit the message
    if let Some(ref msg) = cq.message {
        bot.edit_message_text(chat_id, msg.id(), &message)
            .parse_mode(teloxide::types::ParseMode::Html)
            .reply_markup(keyboard.into())
            .await?;
    }

    bot.answer_callback_query(cq.id).await?;
    Ok(())
}

/// Handle reminder done callback: reminder_done:{rem_id}
async fn handle_reminder_done_callback(
    bot: Bot,
    cq: CallbackQuery,
    db: Db,
    data: &str,
) -> HandlerResult {
    use crate::scheduler::format_full_reminder_time_for_user;

    let chat_id = match cq.message {
        Some(ref m) => m.chat().id,
        None => return Ok(()),
    };

    let rem_id: i32 = match data.parse() {
        Ok(id) => id,
        Err(_) => {
            bot.answer_callback_query(cq.id)
                .text("Ошибка: неверный ID напоминания")
                .await?;
            return Ok(());
        }
    };

    // Get reminder before deleting to show in message
    let reminder = match db.find_reminder(rem_id).await? {
        Some(r) => r,
        None => {
            bot.answer_callback_query(cq.id)
                .text("Напоминание уже удалено")
                .await?;
            // Remove keyboard from message
            if let Some(ref msg) = cq.message {
                let _ = bot.edit_message_reply_markup(chat_id, msg.id()).await;
            }
            return Ok(());
        }
    };

    // Get user for timezone
    let user = db.ensure_user(chat_id.0).await?;
    let time_display = format_full_reminder_time_for_user(&reminder.time, &user);

    // Check if recurring (delay field not empty)
    let is_recurring = !reminder.delay.is_empty();

    let message = if is_recurring {
        // Recurring reminder: just acknowledge, don't delete
        // The scheduler will update the time for the next occurrence
        format!(
            "Напоминание отмечено ✅\n\n\
             📅 {} ▹ {}\n\n\
             🔄 Это периодическое напоминание, оно продолжит работать.",
            time_display,
            crate::scheduler::html_escape(&reminder.text)
        )
    } else {
        // One-time reminder: delete it
        db.complete_reminder(rem_id).await?;

        // Build completed message (with strikethrough)
        format!(
            "Напоминание выполнено ✅ 📅 <s>{} ▹ {}</s>",
            time_display,
            crate::scheduler::html_escape(&reminder.text)
        )
    };

    // Edit the message (remove keyboard)
    if let Some(ref msg) = cq.message {
        bot.edit_message_text(chat_id, msg.id(), &message)
            .parse_mode(teloxide::types::ParseMode::Html)
            .await?;
    }

    bot.answer_callback_query(cq.id).await?;
    Ok(())
}

// Клавиатура utc_keyboard перенесена в crate::bot::keyboards::common
