//! Handlers for channel subscriptions (Twitch/YouTube).

use regex::Regex;
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tracing::info;

use crate::api::db::{ChannelSubscription, Db, Platform};
use crate::bot::keyboards::channel_subs_keyboard;
use crate::bot::router::{AppDialogue, HandlerResult};
use crate::bot::states::AppState;

// ============================================================================
// URL Parsing
// ============================================================================

/// Parsed channel info from URL.
pub struct ParsedChannel {
    pub platform: Platform,
    pub channel_id: String,
    pub channel_name: String,
    pub url: String,
}

/// Parse Twitch or YouTube URL to extract channel info.
pub fn parse_channel_url(url: &str) -> Option<ParsedChannel> {
    let url = url.trim();
    
    // Twitch patterns
    // https://twitch.tv/username
    // https://www.twitch.tv/username
    // twitch.tv/username
    let twitch_re = Regex::new(r"(?:https?://)?(?:www\.)?twitch\.tv/([a-zA-Z0-9_]+)").ok()?;
    if let Some(caps) = twitch_re.captures(url) {
        let username = caps.get(1)?.as_str().to_lowercase();
        return Some(ParsedChannel {
            platform: Platform::Twitch,
            channel_id: username.clone(),
            channel_name: username.clone(),
            url: format!("https://twitch.tv/{}", username),
        });
    }
    
    // YouTube patterns
    // https://youtube.com/@username
    // https://www.youtube.com/@username
    // https://youtube.com/channel/UCxxxx
    // https://www.youtube.com/c/channelname
    let youtube_handle_re = Regex::new(r"(?:https?://)?(?:www\.)?youtube\.com/@([a-zA-Z0-9_-]+)").ok()?;
    if let Some(caps) = youtube_handle_re.captures(url) {
        let handle = caps.get(1)?.as_str();
        return Some(ParsedChannel {
            platform: Platform::Youtube,
            channel_id: format!("@{}", handle),
            channel_name: handle.to_string(),
            url: format!("https://youtube.com/@{}", handle),
        });
    }
    
    let youtube_channel_re = Regex::new(r"(?:https?://)?(?:www\.)?youtube\.com/channel/([a-zA-Z0-9_-]+)").ok()?;
    if let Some(caps) = youtube_channel_re.captures(url) {
        let channel_id = caps.get(1)?.as_str();
        return Some(ParsedChannel {
            platform: Platform::Youtube,
            channel_id: channel_id.to_string(),
            channel_name: channel_id.to_string(),
            url: format!("https://youtube.com/channel/{}", channel_id),
        });
    }
    
    let youtube_c_re = Regex::new(r"(?:https?://)?(?:www\.)?youtube\.com/c/([a-zA-Z0-9_-]+)").ok()?;
    if let Some(caps) = youtube_c_re.captures(url) {
        let name = caps.get(1)?.as_str();
        return Some(ParsedChannel {
            platform: Platform::Youtube,
            channel_id: format!("c/{}", name),
            channel_name: name.to_string(),
            url: format!("https://youtube.com/c/{}", name),
        });
    }
    
    None
}

// ============================================================================
// Command Handler
// ============================================================================

/// Handle /subs command - show subscriptions list.
pub async fn command_subs(bot: Bot, msg: Message, db: Db) -> HandlerResult {
    let chat_id = msg.chat.id;
    let user_id = chat_id.0;

    let subs = db.get_user_channel_subs(user_id).await?;
    let text = format_subs_message(&subs);
    let keyboard = channel_subs_keyboard();

    bot.send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}

/// Format subscriptions list message.
fn format_subs_message(subs: &[ChannelSubscription]) -> String {
    let intro = "Отправь ссылку на канал — и я буду уведомлять о новых видео и трансляциях. \
                 Поддерживаются <b>YouTube</b> и <b>Twitch</b> 🎬";

    if subs.is_empty() {
        format!("{}\n\n<b>Твои подписки</b>: пока нет", intro)
    } else {
        let subs_list: String = subs
            .iter()
            .map(|s| {
                let icon = match s.platform {
                    Platform::Twitch => "🟣",
                    Platform::Youtube => "🔴",
                };
                format!("{}. {} <a href=\"{}\">{}</a>", s.sub_num, icon, s.url, s.channel_name)
            })
            .collect::<Vec<_>>()
            .join("\n");

        format!("{}\n\n<b>Твои подписки</b>:\n{}", intro, subs_list)
    }
}

// ============================================================================
// Text Handlers for Adding/Deleting Subscriptions
// ============================================================================

/// Handle text in AwaitingChannelUrl state - user sends a channel URL.
pub async fn handle_channel_url(
    bot: Bot,
    msg: Message,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let chat_id = msg.chat.id;
    let user_id = chat_id.0;

    let text = match msg.text() {
        Some(t) => t.trim(),
        None => {
            bot.send_message(chat_id, "Пожалуйста, отправь ссылку на канал Twitch или YouTube.")
                .await?;
            return Ok(());
        }
    };

    // Check if user wants to cancel
    if text == "назад" || text == "Назад" || text == "/cancel" {
        dialogue.update(AppState::Idle).await?;
        bot.send_message(chat_id, "Отменено.").await?;
        return Ok(());
    }

    // Try to parse the URL
    let parsed = match parse_channel_url(text) {
        Some(p) => p,
        None => {
            bot.send_message(
                chat_id,
                "❌ Не удалось распознать ссылку.\n\n\
                 Поддерживаемые форматы:\n\
                 • <code>https://twitch.tv/username</code>\n\
                 • <code>https://youtube.com/@handle</code>\n\
                 • <code>https://youtube.com/channel/UCxxxx</code>",
            )
            .parse_mode(ParseMode::Html)
            .await?;
            return Ok(());
        }
    };

    // Check if already subscribed
    if db.is_channel_subscribed(user_id, parsed.platform, &parsed.channel_id).await? {
        bot.send_message(
            chat_id,
            format!("⚠️ Ты уже подписан на <b>{}</b>", parsed.channel_name),
        )
        .parse_mode(ParseMode::Html)
        .await?;
        dialogue.update(AppState::Idle).await?;
        return Ok(());
    }

    // Add subscription
    let sub = db
        .add_channel_sub(
            user_id,
            parsed.platform,
            parsed.channel_id,
            parsed.channel_name.clone(),
            parsed.url.clone(),
        )
        .await?;

    info!(
        user_id = user_id,
        platform = ?sub.platform,
        channel = %sub.channel_name,
        "Added channel subscription"
    );

    let icon = match sub.platform {
        Platform::Twitch => "🟣",
        Platform::Youtube => "🔴",
    };

    bot.send_message(
        chat_id,
        format!(
            "✅ Подписка добавлена!\n\n\
             {} <b>{}</b> — <a href=\"{}\">{}</a>\n\n\
             Я буду уведомлять тебя о новых видео и трансляциях.",
            icon,
            sub.platform,
            sub.url,
            sub.channel_name
        ),
    )
    .parse_mode(ParseMode::Html)
    .await?;

    dialogue.update(AppState::Idle).await?;
    Ok(())
}

/// Handle text in AwaitingSubDeleteNum state - user sends subscription number to delete.
pub async fn handle_sub_delete_num(
    bot: Bot,
    msg: Message,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let chat_id = msg.chat.id;
    let user_id = chat_id.0;

    let text = match msg.text() {
        Some(t) => t.trim(),
        None => {
            bot.send_message(chat_id, "Введи номер подписки для удаления.")
                .await?;
            return Ok(());
        }
    };

    // Check if user wants to cancel
    if text == "назад" || text == "Назад" || text == "/cancel" {
        dialogue.update(AppState::Idle).await?;
        let subs = db.get_user_channel_subs(user_id).await?;
        let msg_text = format_subs_message(&subs);
        let keyboard = channel_subs_keyboard();
        bot.send_message(chat_id, msg_text)
            .parse_mode(ParseMode::Html)
            .reply_markup(keyboard)
            .await?;
        return Ok(());
    }

    // Parse number
    let num: i32 = match text.parse() {
        Ok(n) => n,
        Err(_) => {
            bot.send_message(chat_id, "❌ Введи номер подписки (число).")
                .await?;
            return Ok(());
        }
    };

    // Try to delete
    let deleted = db.delete_channel_sub(user_id, num).await?;

    if deleted {
        info!(user_id = user_id, sub_num = num, "Deleted channel subscription");
        bot.send_message(chat_id, format!("✅ Подписка #{} удалена.", num))
            .await?;
    } else {
        bot.send_message(chat_id, format!("❌ Подписка #{} не найдена.", num))
            .await?;
    }

    dialogue.update(AppState::Idle).await?;

    // Show updated list
    let subs = db.get_user_channel_subs(user_id).await?;
    let msg_text = format_subs_message(&subs);
    let keyboard = channel_subs_keyboard();
    bot.send_message(chat_id, msg_text)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}

// ============================================================================
// Callback Handlers
// ============================================================================

/// Handle "delete subscription" callback.
pub async fn handle_sub_delete_callback(
    bot: Bot,
    q: CallbackQuery,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    bot.answer_callback_query(q.id.clone()).await?;

    let chat_id = match q.message {
        Some(ref m) => m.chat().id,
        None => return Ok(()),
    };
    let user_id = chat_id.0;

    // Check if user has any subscriptions
    let count = db.count_user_channel_subs(user_id).await?;
    if count == 0 {
        bot.send_message(chat_id, "У тебя пока нет подписок для удаления.")
            .await?;
        return Ok(());
    }

    dialogue.update(AppState::AwaitingSubDeleteNum).await?;

    bot.send_message(
        chat_id,
        "Введи номер подписки для удаления (или напиши «назад» для отмены):",
    )
    .await?;

    Ok(())
}

/// Handle "subs" callback (from notification buttons).
pub async fn handle_subs_callback(bot: Bot, q: CallbackQuery, db: Db) -> HandlerResult {
    bot.answer_callback_query(q.id.clone()).await?;

    let chat_id = match q.message {
        Some(ref m) => m.chat().id,
        None => return Ok(()),
    };

    let subs = db.get_user_channel_subs(chat_id.0).await?;
    let text = format_subs_message(&subs);
    let keyboard = channel_subs_keyboard();

    bot.send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}
