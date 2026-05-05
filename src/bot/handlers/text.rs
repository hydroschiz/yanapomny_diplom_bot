use std::collections::HashMap;

use chrono::{offset::Offset, TimeZone};
use once_cell::sync::Lazy;
use regex::Regex;
#[cfg(feature = "telegram-legacy")]
use teloxide::prelude::*;
#[cfg(feature = "telegram-legacy")]
use teloxide::types::{ChatKind, ParseMode};

use crate::api::db::Db;
use crate::bot::{
    keyboards::{common::OFFSETS, utc_keyboard},
    router::HandlerResult,
    states::AppState,
};
#[cfg(feature = "telegram-legacy")]
use crate::bot::router::AppDialogue;
use crate::config::Config;
use crate::transport::dialogue_store::DialogueStore;
use crate::transport::text_format::strip_html;
use crate::transport::traits::{BotTransport, TransportKeyboard};
use crate::utils::timezone::user_has_timezone;

static OFFSET_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)utc\s*([+-]?\d{1,2}(:?\s*[:\.]\s*\d{1,2})?)|^([+-]?\d{1,2}(:?\s*[:\.]\s*\d{1,2})?)$",
    )
    .unwrap()
});

static CITY_MAP: Lazy<HashMap<&'static str, &'static str>> = Lazy::new(|| {
    HashMap::from([
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
        ("kazan", "Europe/Moscow"),
        ("казань", "Europe/Moscow"),
        ("omsk", "Asia/Omsk"),
        ("omsk city", "Asia/Omsk"),
        ("vladivostok", "Asia/Vladivostok"),
        ("владивосток", "Asia/Vladivostok"),
        ("irkutsk", "Asia/Irkutsk"),
        ("иркутск", "Asia/Irkutsk"),
        ("samara", "Europe/Samara"),
        ("самара", "Europe/Samara"),
    ])
});

#[cfg(feature = "telegram-legacy")]
pub fn router() -> teloxide::dispatching::UpdateHandler<anyhow::Error> {
    use teloxide::dispatching::UpdateFilterExt;

    dptree::entry().branch(
        Update::filter_message()
            .branch(dptree::case![AppState::AwaitingUtc].endpoint(handle_utc_input))
            .branch(dptree::case![AppState::AwaitingSnoozeButtons].endpoint(handle_snooze_input))
            .branch(dptree::case![AppState::AwaitingAutoSnooze].endpoint(handle_auto_snooze_input)),
    )
}

#[cfg(feature = "telegram-legacy")]
pub async fn handle_group_text(
    bot: Bot,
    msg: Message,
    dialogue: AppDialogue,
    db: Db,
    config: Config,
) -> HandlerResult {
    let text = match msg.text() {
        Some(t) => t.trim(),
        None => return Ok(()),
    };

    let clean_text = match extract_group_mention_text(text, &config.bot_username) {
        Some(text) => text,
        None => return Ok(()),
    };

    let group_id = msg.chat.id.0;

    if let ChatKind::Public(chat) = &msg.chat.kind {
        let title = chat.title.clone().unwrap_or_else(|| "Group".to_string());
        let owner_id = msg.from.as_ref().map(|user| user.id.0 as i64).unwrap_or(msg.chat.id.0);
        let _ = db.ensure_group_record(group_id, title, owner_id).await?;
    }

    let group_user = db.ensure_user(group_id).await?;
    if !user_has_timezone(&group_user) {
        bot.send_message(
            msg.chat.id,
            format!(
                "Сначала установите часовой пояс для этого чата командой /start@{} или /utc",
                config.bot_username
            ),
        )
        .await?;
        return Ok(());
    }

    // Check subscriptions
    // 1. Check group subscription
    let mut is_allowed = db.is_subscription_active(group_id).await?;
    
    // 2. Check sender subscription
    if !is_allowed {
        if let Some(user) = &msg.from {
            if db.is_subscription_active(user.id.0 as i64).await? {
                is_allowed = true;
            }
        }
    }

    // 3. Check owner subscription
    if !is_allowed {
        if let Some(record) = db.find_record(group_id).await? {
            if let Some(owner_id) = record.owner_id {
                 if db.is_subscription_active(owner_id).await? {
                    is_allowed = true;
                 }
            }
        }
    }

    if !is_allowed {
         bot.send_message(msg.chat.id, "⚠️ Подписка не активна. Бот работает в группах, если у группы, отправителя или добавившего администратора есть активная подписка.").await?;
         return Ok(());
    }

    super::reminder::start_reminder_creation_flow(bot, msg.chat.id, clean_text, dialogue).await
}

pub fn extract_group_mention_text(text: &str, bot_username: &str) -> Option<String> {
    let username = bot_username.trim_start_matches('@');

    for (at_index, _) in text.match_indices('@') {
        let after_at = &text[at_index + 1..];
        if after_at.len() < username.len() {
            continue;
        }

        let candidate = &after_at[..username.len()];
        if !candidate.eq_ignore_ascii_case(username) {
            continue;
        }

        let boundary = after_at[username.len()..].chars().next();
        if matches!(boundary, Some(ch) if ch.is_ascii_alphanumeric() || ch == '_') {
            continue;
        }

        let mut suffix = &after_at[username.len()..];
        suffix = suffix.trim_start_matches(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ':' | ';' | '-' | '!' | '?'));

        let prefix = text[..at_index].trim_end();
        let cleaned = if prefix.is_empty() {
            suffix.trim().to_string()
        } else if suffix.trim().is_empty() {
            prefix.to_string()
        } else {
            format!("{} {}", prefix, suffix.trim())
        };

        let cleaned = cleaned.trim().to_string();
        if !cleaned.is_empty() {
            return Some(cleaned);
        }
    }

    None
}

// ============================================================================
// Transport-native text handlers for VK router
// ============================================================================

#[allow(clippy::too_many_arguments)]
pub async fn handle_group_text_transport<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    text: &str,
    group_title: Option<&str>,
    store: &DialogueStore,
    db: Db,
    config: Config,
) -> HandlerResult {
    let clean_text = match extract_group_mention_text(text.trim(), &config.bot_username) {
        Some(text) => text,
        None => return Ok(()),
    };

    let title = group_title
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("VK chat {}", peer_id));
    let _ = db.ensure_group_record(peer_id, title, user_id).await?;

    let group_user = db.ensure_user(peer_id).await?;
    if !user_has_timezone(&group_user) {
        transport
            .send_text(
                peer_id,
                &format!(
                    "Сначала установите часовой пояс для этого чата командой /start@{} или /utc",
                    config.bot_username
                ),
            )
            .await?;
        return Ok(());
    }

    let mut is_allowed = db.is_subscription_active(peer_id).await?;
    if !is_allowed && db.is_subscription_active(user_id).await? {
        is_allowed = true;
    }

    if !is_allowed {
        if let Some(record) = db.find_record(peer_id).await? {
            if let Some(owner_id) = record.owner_id {
                if db.is_subscription_active(owner_id).await? {
                    is_allowed = true;
                }
            }
        }
    }

    if !is_allowed {
        transport
            .send_text(
                peer_id,
                "⚠️ Подписка не активна. Бот работает в группах, если у группы, отправителя или добавившего администратора есть активная подписка.",
            )
            .await?;
        return Ok(());
    }

    super::reminder::start_reminder_creation_flow_transport(
        transport,
        peer_id,
        user_id,
        clean_text,
        store,
    )
    .await
}

pub async fn handle_utc_input_transport<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    text: &str,
    store: &DialogueStore,
    db: Db,
) -> HandlerResult {
    let text = text.trim();

    if let Some(offset) = parse_offset(text) {
        db.ensure_user(peer_id).await?;
        db.update_utc_and_clear_timezone(peer_id, &offset).await?;
        db.update_user_state(peer_id, "waiting_for_message").await?;
        store.update(user_id, AppState::Idle);
        send_html_text(
            transport,
            peer_id,
            &super::commands::UTC_SUCCESS_MESSAGE.replace("+3:00", &offset),
        )
        .await?;
        return Ok(());
    }

    if let Some(tz_name) = resolve_timezone(text) {
        let tz_offset = timezone_offset_string(&tz_name).unwrap_or("+00:00".to_string());

        db.ensure_user(peer_id).await?;
        db.update_timezone(peer_id, &tz_name, &tz_offset).await?;
        db.update_user_state(peer_id, "waiting_for_message").await?;
        store.update(user_id, AppState::Idle);

        send_html_text(
            transport,
            peer_id,
            &super::commands::UTC_SUCCESS_MESSAGE.replace("+3:00", &tz_offset),
        )
        .await?;
        return Ok(());
    }

    let keyboard = utc_keyboard();
    send_html_with_keyboard(
        transport,
        peer_id,
        "Не удалось определить часовой пояс. Укажите в формате UTC+3 или назовите город. Чтобы выйти, нажмите «Назад».",
        &keyboard,
    )
    .await?;

    Ok(())
}

pub async fn handle_snooze_input_transport<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    text: &str,
    store: &DialogueStore,
    db: Db,
) -> HandlerResult {
    let durations = parse_duration_list(text);
    if durations.is_empty() {
        transport.send_text(peer_id, "Не понял время. Пример: \"15 мин, 1 час\". Доступно: 5,10,15,20,30 мин; 1,2,3,4 часа; 1,2,3,7 дней.").await?;
        return Ok(());
    }

    let mut codes = Vec::new();
    for m in durations {
        if let Some(code) = snooze_code(m) {
            codes.push(code);
        } else {
            transport.send_text(peer_id, "Некорректное время. Доступно: 5,10,15,20,30 мин; 1,2,3,4 часа; 1,2,3,7 дней.").await?;
            return Ok(());
        }
    }

    db.update_snooze_buttons(peer_id, codes.clone()).await?;
    db.update_user_state(peer_id, "waiting_for_message").await?;
    store.update(user_id, AppState::Idle);

    let human = codes
        .iter()
        .filter_map(|c| human_readable_snooze(c))
        .collect::<Vec<_>>()
        .join(", ");

    send_html_text(
        transport,
        peer_id,
        &format!("Сохранено. Кнопки откладывания: <b>{}</b>", human),
    )
    .await?;
    Ok(())
}

pub async fn handle_auto_snooze_input_transport<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    text: &str,
    store: &DialogueStore,
    db: Db,
) -> HandlerResult {
    let durations = parse_duration_list(text);
    if durations.len() != 1 {
        transport
            .send_text(
                peer_id,
                "Введите одно значение, например \"15 мин\". Доступно: 5,10,15,20 мин.",
            )
            .await?;
        return Ok(());
    }

    let code = match durations[0] {
        5 => Some("5minutAutoSnooze"),
        10 => Some("10minutAutoSnooze"),
        15 => Some("15minutAutoSnooze"),
        20 => Some("20minutAutoSnooze"),
        _ => None,
    };
    let code = if let Some(c) = code {
        c
    } else {
        transport
            .send_text(peer_id, "Некорректное время. Доступно: 5,10,15,20 мин.")
            .await?;
        return Ok(());
    };

    db.update_auto_delay(peer_id, code.to_string()).await?;
    db.update_user_state(peer_id, "waiting_for_message").await?;
    store.update(user_id, AppState::Idle);

    send_html_text(
        transport,
        peer_id,
        &format!(
            "Сохранено. Авто откладывание: <b>{}</b>",
            human_readable_auto(code)
        ),
    )
    .await?;
    Ok(())
}

#[cfg(feature = "telegram-legacy")]
pub async fn handle_utc_input(
    bot: Bot,
    msg: Message,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let chat_id = msg.chat.id;
    let text = if let Some(t) = msg.text() {
        t.trim()
    } else {
        ""
    };

    // Try offsets first.
    if let Some(offset) = parse_offset(text) {
        db.ensure_user(chat_id.0).await?;
        db.update_utc_and_clear_timezone(chat_id.0, &offset).await?;
        db.update_user_state(chat_id.0, "waiting_for_message")
            .await?;
        dialogue.update(AppState::Idle).await?;
        bot.send_message(
            chat_id,
            super::commands::UTC_SUCCESS_MESSAGE.replace("+3:00", &offset),
        )
        .parse_mode(ParseMode::Html)
        .await?;
        return Ok(());
    }

    // Try city/IANA timezone resolution.
    if let Some(tz_name) = resolve_timezone(text) {
        let tz_offset = timezone_offset_string(&tz_name).unwrap_or("+00:00".to_string());

        db.ensure_user(chat_id.0).await?;
        db.update_timezone(chat_id.0, &tz_name, &tz_offset).await?;
        db.update_user_state(chat_id.0, "waiting_for_message")
            .await?;
        dialogue.update(AppState::Idle).await?;

        bot.send_message(
            chat_id,
            super::commands::UTC_SUCCESS_MESSAGE.replace("+3:00", &tz_offset),
        )
        .parse_mode(ParseMode::Html)
        .await?;
        return Ok(());
    }

    // Unknown input -> send prompt again.
    bot.send_message(
        chat_id,
        "Не удалось определить часовой пояс. Укажите в формате UTC+3 или назовите город. Чтобы выйти, нажмите «Назад».",
    )
    .reply_markup(utc_keyboard())
    .parse_mode(ParseMode::Html)
    .await?;

    Ok(())
}

#[cfg(feature = "telegram-legacy")]
pub async fn handle_snooze_input(
    bot: Bot,
    msg: Message,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let chat_id = msg.chat.id;
    let text = msg.text().unwrap_or("").to_string();
    let durations = parse_duration_list(&text);
    if durations.is_empty() {
        bot.send_message(chat_id, "Не понял время. Пример: \"15 мин, 1 час\". Доступно: 5,10,15,20,30 мин; 1,2,3,4 часа; 1,2,3,7 дней.")
            .await?;
        return Ok(());
    }

    let mut codes = Vec::new();
    for m in durations {
        if let Some(code) = snooze_code(m) {
            codes.push(code);
        } else {
            bot.send_message(chat_id, "Некорректное время. Доступно: 5,10,15,20,30 мин; 1,2,3,4 часа; 1,2,3,7 дней.")
                .await?;
            return Ok(());
        }
    }

    db.update_snooze_buttons(chat_id.0, codes.clone()).await?;
    db.update_user_state(chat_id.0, "waiting_for_message").await?;
    dialogue.update(AppState::Idle).await?;

    let human = codes
        .iter()
        .filter_map(|c| human_readable_snooze(c))
        .collect::<Vec<_>>()
        .join(", ");
    bot.send_message(
        chat_id,
        format!("Сохранено. Кнопки откладывания: <b>{}</b>", human),
    )
    .parse_mode(ParseMode::Html)
    .await?;
    Ok(())
}

#[cfg(feature = "telegram-legacy")]
pub async fn handle_auto_snooze_input(
    bot: Bot,
    msg: Message,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let chat_id = msg.chat.id;
    let text = msg.text().unwrap_or("").to_string();
    let durations = parse_duration_list(&text);
    if durations.len() != 1 {
        bot.send_message(
            chat_id,
            "Введите одно значение, например \"15 мин\". Доступно: 5,10,15,20 мин.",
        )
        .await?;
        return Ok(());
    }
    let minutes = durations[0];
    let code = match minutes {
        5 => Some("5minutAutoSnooze"),
        10 => Some("10minutAutoSnooze"),
        15 => Some("15minutAutoSnooze"),
        20 => Some("20minutAutoSnooze"),
        _ => None,
    };
    let code = if let Some(c) = code {
        c
    } else {
        bot.send_message(chat_id, "Некорректное время. Доступно: 5,10,15,20 мин.").await?;
        return Ok(());
    };

    db.update_auto_delay(chat_id.0, code.to_string()).await?;
    db.update_user_state(chat_id.0, "waiting_for_message").await?;
    dialogue.update(AppState::Idle).await?;

    bot.send_message(
        chat_id,
        format!(
            "Сохранено. Авто откладывание: <b>{}</b>",
            human_readable_auto(code)
        ),
    )
    .parse_mode(ParseMode::Html)
    .await?;
    Ok(())
}

pub fn parse_offset(input: &str) -> Option<String> {
    let cleaned = input.trim();
    if let Some(caps) = OFFSET_RE.captures(cleaned) {
        let raw = caps
            .get(1)
            .or_else(|| caps.get(3))
            .map(|m| m.as_str().trim())?;
        normalize_offset(raw)
    } else {
        None
    }
}

pub fn normalize_offset(raw: &str) -> Option<String> {
    let mut s = raw.trim().to_uppercase();
    s = s.replace("UTC", "").replace(' ', "");
    s = s.replace(',', ".");
    if !s.starts_with('+') && !s.starts_with('-') {
        s = format!("+{}", s);
    }

    let parts: Vec<&str> = s
        .trim_start_matches('+')
        .trim_start_matches('-')
        .split([':', '.'])
        .collect();
    let sign = if raw.trim().starts_with('-') { -1 } else { 1 };

    let hours: i32 = parts.first()?.parse().ok()?;
    let minutes: i32 = if let Some(m) = parts.get(1) {
        m.parse().unwrap_or(0)
    } else {
        0
    };
    if hours.abs() > 14 || minutes >= 60 {
        return None;
    }

    let total_minutes = sign * (hours * 60 + minutes);
    let hrs = total_minutes / 60;
    let mins = total_minutes.abs() % 60;
    let formatted = format!("{:+03}:{:02}", hrs, mins);
    OFFSETS
        .iter()
        .find(|o| o.trim() == formatted)
        .map(|_| formatted)
}

fn resolve_timezone(input: &str) -> Option<String> {
    let trimmed = input.trim();
    // Exact IANA name
    if trimmed.contains('/') {
        return Some(trimmed.replace(' ', ""));
    }

    let lower = trimmed.to_lowercase();
    if let Some(tz) = CITY_MAP.get(lower.as_str()) {
        return Some((*tz).to_string());
    }

    None
}

pub fn timezone_offset_string(tz_name: &str) -> Option<String> {
    let tz: chrono_tz::Tz = tz_name.parse().ok()?;
    let offset = tz.offset_from_utc_datetime(&chrono::Utc::now().naive_utc());
    let seconds = offset.fix().local_minus_utc();
    let sign = if seconds >= 0 { '+' } else { '-' };
    let secs = seconds.abs();
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    Some(format!("{sign}{hours:02}:{minutes:02}"))
}

pub const UTC_PROMPT_FALLBACK: &str = "Не удалось определить часовой пояс. Укажите в формате UTC+3 или назовите город. Чтобы выйти, нажмите «Назад».";

fn parse_duration_list(input: &str) -> Vec<i32> {
    static DURATION_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)(\d+)\s*(минут|мин|m|час|ч|h|день|дн|дня|сут)").unwrap());

    let mut out = Vec::new();
    for caps in DURATION_RE.captures_iter(input) {
        let num: i32 = caps
            .get(1)
            .and_then(|m| m.as_str().parse().ok())
            .unwrap_or(0);
        let unit = caps
            .get(2)
            .map(|m| m.as_str().to_lowercase())
            .unwrap_or_default();
        let minutes = if unit.starts_with('м') || unit.starts_with('m') {
            num
        } else if unit.starts_with('ч') || unit.starts_with('h') {
            num * 60
        } else {
            num * 1440
        };
        if minutes > 0 {
            out.push(minutes);
        }
    }
    out
}

fn snooze_code(minutes: i32) -> Option<String> {
    match minutes {
        5 => Some("5minutSnooze".into()),
        10 => Some("10minutSnooze".into()),
        15 => Some("15minutSnooze".into()),
        20 => Some("20minutSnooze".into()),
        30 => Some("30minutSnooze".into()),
        60 => Some("1hourSnooze".into()),
        120 => Some("2hourSnooze".into()),
        180 => Some("3hourSnooze".into()),
        240 => Some("4hourSnooze".into()),
        1440 => Some("1daySnooze".into()),
        2880 => Some("2daySnooze".into()),
        4320 => Some("3daySnooze".into()),
        10080 => Some("7daySnooze".into()),
        _ => None,
    }
}

pub fn human_readable_snooze(code: &str) -> Option<&'static str> {
    match code {
        "5minutSnooze" => Some("5 мин"),
        "10minutSnooze" => Some("10 мин"),
        "15minutSnooze" => Some("15 мин"),
        "20minutSnooze" => Some("20 мин"),
        "30minutSnooze" => Some("30 мин"),
        "1hourSnooze" => Some("1 час"),
        "2hourSnooze" => Some("2 часа"),
        "3hourSnooze" => Some("3 часа"),
        "4hourSnooze" => Some("4 часа"),
        "1daySnooze" => Some("1 день"),
        "2daySnooze" => Some("2 дня"),
        "3daySnooze" => Some("3 дня"),
        "7daySnooze" => Some("7 дней"),
        _ => None,
    }
}

pub fn human_readable_auto(code: &str) -> String {
    match code {
        "5minutAutoSnooze" => "5 мин".to_string(),
        "10minutAutoSnooze" => "10 мин".to_string(),
        "15minutAutoSnooze" => "15 мин".to_string(),
        "20minutAutoSnooze" => "20 мин".to_string(),
        _ => code.to_string(),
    }
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
