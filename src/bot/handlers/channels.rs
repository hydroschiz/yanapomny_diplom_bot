//! Handlers for channel subscriptions (Twitch/YouTube).

use regex::Regex;
use teloxide::prelude::*;
use tracing::info;

use crate::api::db::{ChannelSubscription, Db, Platform};
use crate::bot::keyboards::channel_subs_keyboard;
use crate::bot::router::{AppDialogue, HandlerResult};
use crate::bot::states::AppState;
use crate::transport::adapters::TelegramTransport;
use crate::transport::dialogue_store::DialogueStore;
use crate::transport::text_format::strip_html;
use crate::transport::traits::{BotTransport, TransportKeyboard};

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

    let youtube_handle_re =
        Regex::new(r"(?:https?://)?(?:www\.)?youtube\.com/@([a-zA-Z0-9_-]+)").ok()?;
    if let Some(caps) = youtube_handle_re.captures(url) {
        let handle = caps.get(1)?.as_str();
        return Some(ParsedChannel {
            platform: Platform::Youtube,
            channel_id: format!("@{}", handle),
            channel_name: handle.to_string(),
            url: format!("https://youtube.com/@{}", handle),
        });
    }

    let youtube_channel_re =
        Regex::new(r"(?:https?://)?(?:www\.)?youtube\.com/channel/([a-zA-Z0-9_-]+)").ok()?;
    if let Some(caps) = youtube_channel_re.captures(url) {
        let channel_id = caps.get(1)?.as_str();
        return Some(ParsedChannel {
            platform: Platform::Youtube,
            channel_id: channel_id.to_string(),
            channel_name: channel_id.to_string(),
            url: format!("https://youtube.com/channel/{}", channel_id),
        });
    }

    let youtube_c_re =
        Regex::new(r"(?:https?://)?(?:www\.)?youtube\.com/c/([a-zA-Z0-9_-]+)").ok()?;
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

/// Handle /subs command through transport abstraction.
pub async fn command_subs_transport<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    db: Db,
) -> HandlerResult {
    let subs = db.get_user_channel_subs(user_id).await?;
    let text = format_subs_message(&subs);
    let keyboard = channel_subs_keyboard();

    send_html_with_keyboard(transport, peer_id, &text, &keyboard).await
}

/// Временный Telegram entrypoint до переключения app/router на VK.
pub async fn command_subs(bot: Bot, msg: Message, db: Db) -> HandlerResult {
    let peer_id = msg.chat.id.0;
    let user_id = msg.from.as_ref().map(|user| user.id.0 as i64).unwrap_or(peer_id);
    let transport = TelegramTransport::new(bot);

    command_subs_transport(&transport, peer_id, user_id, db).await
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
                format!(
                    "{}. {} <a href=\"{}\">{}</a>",
                    s.sub_num, icon, s.url, s.channel_name
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        format!("{}\n\n<b>Твои подписки</b>:\n{}", intro, subs_list)
    }
}

// ============================================================================
// Text Handlers for Adding/Deleting Subscriptions
// ============================================================================

/// Handle text with channel URL through transport abstraction.
pub async fn handle_channel_url_transport<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    text: &str,
    store: &DialogueStore,
    db: Db,
) -> HandlerResult {
    handle_channel_url_core(transport, peer_id, user_id, text, Some(store), None, db).await
}

/// Временный Telegram entrypoint до переключения app/router на VK.
pub async fn handle_channel_url(
    bot: Bot,
    msg: Message,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let text = msg.text().unwrap_or("");
    let peer_id = msg.chat.id.0;
    let user_id = msg.from.as_ref().map(|user| user.id.0 as i64).unwrap_or(peer_id);
    let transport = TelegramTransport::new(bot);

    handle_channel_url_core(&transport, peer_id, user_id, text, None, Some(&dialogue), db).await
}

/// Handle text in AwaitingSubDeleteNum state through transport abstraction.
pub async fn handle_sub_delete_num_transport<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    text: &str,
    store: &DialogueStore,
    db: Db,
) -> HandlerResult {
    handle_sub_delete_num_core(transport, peer_id, user_id, text, Some(store), None, db).await
}

/// Временный Telegram entrypoint до переключения app/router на VK.
pub async fn handle_sub_delete_num(
    bot: Bot,
    msg: Message,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let text = msg.text().unwrap_or("");
    let peer_id = msg.chat.id.0;
    let user_id = msg.from.as_ref().map(|user| user.id.0 as i64).unwrap_or(peer_id);
    let transport = TelegramTransport::new(bot);

    handle_sub_delete_num_core(&transport, peer_id, user_id, text, None, Some(&dialogue), db).await
}

#[allow(clippy::too_many_arguments)]
async fn handle_channel_url_core<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    text: &str,
    store: Option<&DialogueStore>,
    dialogue: Option<&AppDialogue>,
    db: Db,
) -> HandlerResult {
    let text = text.trim();
    if text.is_empty() {
        transport
            .send_text(peer_id, "Пожалуйста, отправь ссылку на канал Twitch или YouTube.")
            .await?;
        return Ok(());
    }

    if text.eq_ignore_ascii_case("назад") || text == "/cancel" {
        update_state(store, dialogue, user_id, AppState::Idle).await?;
        transport.send_text(peer_id, "Отменено.").await?;
        return Ok(());
    }

    let parsed = match parse_channel_url(text) {
        Some(p) => p,
        None => {
            send_html_text(
                transport,
                peer_id,
                "❌ Не удалось распознать ссылку.\n\n\
                 Поддерживаемые форматы:\n\
                 • <code>https://twitch.tv/username</code>\n\
                 • <code>https://youtube.com/@handle</code>\n\
                 • <code>https://youtube.com/channel/UCxxxx</code>",
            )
            .await?;
            return Ok(());
        }
    };

    if db
        .is_channel_subscribed(user_id, parsed.platform, &parsed.channel_id)
        .await?
    {
        send_html_text(
            transport,
            peer_id,
            &format!("⚠️ Ты уже подписан на <b>{}</b>", parsed.channel_name),
        )
        .await?;
        update_state(store, dialogue, user_id, AppState::Idle).await?;
        return Ok(());
    }

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

    send_html_text(
        transport,
        peer_id,
        &format!(
            "✅ Подписка добавлена!\n\n\
             {} <b>{}</b> — <a href=\"{}\">{}</a>\n\n\
             Я буду уведомлять тебя о новых видео и трансляциях.",
            icon, sub.platform, sub.url, sub.channel_name
        ),
    )
    .await?;

    update_state(store, dialogue, user_id, AppState::Idle).await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_sub_delete_num_core<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    text: &str,
    store: Option<&DialogueStore>,
    dialogue: Option<&AppDialogue>,
    db: Db,
) -> HandlerResult {
    let text = text.trim();
    if text.is_empty() {
        transport
            .send_text(peer_id, "Введи номер подписки для удаления.")
            .await?;
        return Ok(());
    }

    if text.eq_ignore_ascii_case("назад") || text == "/cancel" {
        update_state(store, dialogue, user_id, AppState::Idle).await?;
        send_subs_list(transport, peer_id, user_id, db).await?;
        return Ok(());
    }

    let num: i32 = match text.parse() {
        Ok(n) => n,
        Err(_) => {
            transport
                .send_text(peer_id, "❌ Введи номер подписки (число).")
                .await?;
            return Ok(());
        }
    };

    let deleted = db.delete_channel_sub(user_id, num).await?;

    if deleted {
        info!(user_id = user_id, sub_num = num, "Deleted channel subscription");
        transport
            .send_text(peer_id, &format!("✅ Подписка #{} удалена.", num))
            .await?;
    } else {
        transport
            .send_text(peer_id, &format!("❌ Подписка #{} не найдена.", num))
            .await?;
    }

    update_state(store, dialogue, user_id, AppState::Idle).await?;
    send_subs_list(transport, peer_id, user_id, db).await?;

    Ok(())
}

// ============================================================================
// Callback Handlers
// ============================================================================

/// Handle "delete subscription" callback through transport abstraction.
pub async fn handle_sub_delete_callback_transport<T: BotTransport>(
    transport: &T,
    event_id: &str,
    user_id: i64,
    peer_id: i64,
    store: &DialogueStore,
    db: Db,
) -> HandlerResult {
    handle_sub_delete_callback_core(transport, event_id, user_id, peer_id, Some(store), None, db)
        .await
}

/// Временный Telegram callback entrypoint до переключения app/router на VK.
pub async fn handle_sub_delete_callback(
    bot: Bot,
    q: CallbackQuery,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let peer_id = match &q.message {
        Some(msg) => msg.chat().id.0,
        None => return Ok(()),
    };
    let user_id = q.from.id.0 as i64;
    let transport = TelegramTransport::new(bot);

    handle_sub_delete_callback_core(
        &transport,
        &q.id.0,
        user_id,
        peer_id,
        None,
        Some(&dialogue),
        db,
    )
    .await
}

/// Handle "subs" callback through transport abstraction.
pub async fn handle_subs_callback_transport<T: BotTransport>(
    transport: &T,
    event_id: &str,
    user_id: i64,
    peer_id: i64,
    db: Db,
) -> HandlerResult {
    transport
        .answer_callback(event_id, user_id, peer_id, None)
        .await?;
    send_subs_list(transport, peer_id, user_id, db).await
}

/// Временный Telegram callback entrypoint до переключения app/router на VK.
pub async fn handle_subs_callback(bot: Bot, q: CallbackQuery, db: Db) -> HandlerResult {
    let peer_id = match &q.message {
        Some(msg) => msg.chat().id.0,
        None => return Ok(()),
    };
    let user_id = q.from.id.0 as i64;
    let transport = TelegramTransport::new(bot);

    handle_subs_callback_transport(&transport, &q.id.0, user_id, peer_id, db).await
}

#[allow(clippy::too_many_arguments)]
async fn handle_sub_delete_callback_core<T: BotTransport>(
    transport: &T,
    event_id: &str,
    user_id: i64,
    peer_id: i64,
    store: Option<&DialogueStore>,
    dialogue: Option<&AppDialogue>,
    db: Db,
) -> HandlerResult {
    transport
        .answer_callback(event_id, user_id, peer_id, None)
        .await?;

    let count = db.count_user_channel_subs(user_id).await?;
    if count == 0 {
        transport
            .send_text(peer_id, "У тебя пока нет подписок для удаления.")
            .await?;
        return Ok(());
    }

    update_state(store, dialogue, user_id, AppState::AwaitingSubDeleteNum).await?;
    transport
        .send_text(
            peer_id,
            "Введи номер подписки для удаления (или напиши «назад» для отмены):",
        )
        .await?;

    Ok(())
}

async fn send_subs_list<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    db: Db,
) -> HandlerResult {
    let subs = db.get_user_channel_subs(user_id).await?;
    let text = format_subs_message(&subs);
    let keyboard = channel_subs_keyboard();

    send_html_with_keyboard(transport, peer_id, &text, &keyboard).await
}

async fn update_state(
    store: Option<&DialogueStore>,
    dialogue: Option<&AppDialogue>,
    user_id: i64,
    state: AppState,
) -> HandlerResult {
    if let Some(store) = store {
        store.update(user_id, state);
    } else if let Some(dialogue) = dialogue {
        dialogue.update(state).await?;
    }

    Ok(())
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
