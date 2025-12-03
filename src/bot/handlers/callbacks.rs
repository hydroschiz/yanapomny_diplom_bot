use teloxide::prelude::*;

use crate::api::db::Db;
use crate::bot::{
    router::{AppDialogue, HandlerResult},
    states::AppState,
};

use super::commands::{
    back_keyboard, setup_keyboard, start_utc_flow, AUTO_SNOOZE_PROMPT, SETUP_PROMPT,
    SNOOZE_PROMPT, UTC_SUCCESS_MESSAGE,
};
use super::text::{human_readable_auto, human_readable_snooze, normalize_offset, OFFSETS};

pub async fn handle_callback(
    bot: Bot,
    cq: CallbackQuery,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let data = if let Some(data) = cq.data.clone() {
        data
    } else {
        return Ok(());
    };
    let chat_id = if let Some(msg) = &cq.message { msg.chat().id } else { return Ok(()) };

    match data.as_str() {
        "setup_menu" => {
            dialogue.update(AppState::Idle).await?;
            db.update_user_state(chat_id.0, "waiting_for_message").await?;
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

    // Unknown callback data: send quick error.
    bot.answer_callback_query(cq.id)
        .text("Не удалось обработать выбор. Попробуйте снова.")
        .await?;
    Ok(())
}

/// Inline keyboard with UTC offsets and control buttons.
pub fn utc_keyboard() -> teloxide::types::InlineKeyboardMarkup {
    use teloxide::types::InlineKeyboardButton as Btn;

    let mut rows: Vec<Vec<Btn>> = Vec::new();
    for chunk in OFFSETS.chunks(4) {
        let row = chunk
            .iter()
            .map(|o| {
                let label = format!("UTC{}", o);
                Btn::callback(label, format!("utc_set:{}", o))
            })
            .collect();
        rows.push(row);
    }

    rows.push(vec![Btn::callback("⬅ Назад", "utc_cancel")]);
    teloxide::types::InlineKeyboardMarkup::new(rows)
}
