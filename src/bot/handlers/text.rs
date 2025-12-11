use std::collections::HashMap;

use chrono::{offset::Offset, TimeZone};
use once_cell::sync::Lazy;
use regex::Regex;
use teloxide::prelude::*;
use teloxide::types::ParseMode;

use crate::api::db::Db;
use crate::bot::{
    keyboards::{common::OFFSETS, utc_keyboard},
    router::{AppDialogue, HandlerResult},
    states::AppState,
};

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

pub fn router() -> teloxide::dispatching::UpdateHandler<anyhow::Error> {
    use teloxide::dispatching::UpdateFilterExt;

    dptree::entry().branch(
        Update::filter_message()
            .filter(crate::bot::filters::private_chat_msg)
            .branch(dptree::case![AppState::AwaitingUtc].endpoint(handle_utc_input))
            .branch(dptree::case![AppState::AwaitingSnoozeButtons].endpoint(handle_snooze_input))
            .branch(dptree::case![AppState::AwaitingAutoSnooze].endpoint(handle_auto_snooze_input)),
    )
}

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
        db.update_timezone(chat_id.0, &tz_name).await?;
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

    let hours: i32 = parts.get(0)?.parse().ok()?;
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
