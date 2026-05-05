//! Обработчики команды /profile и профиля пользователя.

#[cfg(feature = "telegram-legacy")]
use teloxide::prelude::*;
#[cfg(feature = "telegram-legacy")]
use teloxide::types::Message;

use crate::api::db::Db;
use crate::bot::keyboards::profile_keyboard;
use crate::bot::router::HandlerResult;
use crate::scheduler::format_full_reminder_time_for_user;
#[cfg(feature = "telegram-legacy")]
use crate::transport::adapters::TelegramTransport;
use crate::transport::text_format::strip_html;
use crate::transport::traits::{BotTransport, TransportKeyboard};

/// Советы от Яна для профиля.
const TIPS: &[&str] = &[
    "Попробуй объединять похожие задачи в блоки — так мозгу проще концентрироваться, и ты сэкономишь до 20% времени ⏱",
    "Самые важные дела лучше планировать на утро, когда концентрация максимальна 🌅",
    "Не забывай делать перерывы — 5 минут отдыха каждый час повышают продуктивность 🧘",
    "Разбивай большие задачи на мелкие шаги — так проще начать и не откладывать 📝",
    "Используй правило 2 минут: если дело занимает меньше 2 минут — сделай сейчас ⚡",
    "Планируй следующий день вечером — утром будет легче начать 🌙",
    "Отключай уведомления во время важных задач — это сохранит фокус 🔕",
];

/// Обработчик команды /profile через абстрактный транспорт.
pub async fn handle_profile_command_transport<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    nickname: Option<&str>,
    db: Db,
) -> HandlerResult {
    let message = build_profile_message(user_id, nickname, db).await?;
    let keyboard = profile_keyboard();

    send_html_with_keyboard(transport, peer_id, &message, &keyboard).await
}

#[cfg(feature = "telegram-legacy")]
/// Временный Telegram entrypoint до переключения app/router на VK.
pub async fn handle_profile_command(bot: Bot, msg: Message, db: Db) -> HandlerResult {
    let peer_id = msg.chat.id.0;
    let user_id = msg.from.as_ref().map(|user| user.id.0 as i64).unwrap_or(peer_id);
    let nickname = msg
        .from
        .as_ref()
        .and_then(|user| user.username.as_deref())
        .map(|username| format!("@{}", username));
    let transport = TelegramTransport::new(bot);

    handle_profile_command_transport(&transport, peer_id, user_id, nickname.as_deref(), db).await
}

async fn build_profile_message(
    user_id: i64,
    nickname: Option<&str>,
    db: Db,
) -> anyhow::Result<String> {
    let user = db.ensure_user(user_id).await?;
    let record = db.ensure_record(user_id).await?;

    let active_count = db.count_active_reminders(user_id).await.unwrap_or(0);
    let this_month = db.count_reminders_this_month(user_id).await.unwrap_or(0);
    let last_month = db.count_reminders_last_month(user_id).await.unwrap_or(0);

    let growth = if last_month > 0 {
        let diff = this_month as f64 - last_month as f64;
        let percent = (diff / last_month as f64) * 100.0;
        if percent >= 0.0 {
            format!("+{:.0}%", percent)
        } else {
            format!("{:.0}%", percent)
        }
    } else if this_month > 0 {
        "+100%".to_string()
    } else {
        "0%".to_string()
    };

    let subscription_status = if record.is_active() {
        format!("активна до {}", record.expiry_formatted())
    } else if record.free_state == Some(1) {
        "пробный период".to_string()
    } else {
        "не активна".to_string()
    };

    let channel_subs_count = db
        .get_user_channel_subs(user_id)
        .await
        .map(|subs| subs.len())
        .unwrap_or(0);

    let tip_index = (user_id as usize) % TIPS.len();
    let tip = TIPS[tip_index];

    let last_reminder = db.get_last_reminder(user_id).await.ok().flatten();
    let last_reminder_text = if let Some(rem) = last_reminder {
        let time_display = format_full_reminder_time_for_user(&rem.time, &user);
        format!(
            "📌 Ближайшее напоминание: \"{}\" — {}\n\n",
            truncate_text(&rem.text, 40),
            time_display
        )
    } else {
        String::new()
    };

    let nickname = nickname
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("#{}", user_id));

    Ok(format!(
        "👤 Профиль {}\n\n\
         📅 Напоминаний активно: <b>{}</b>\n\
         📈 Всего создано в этом месяце: <b>{}</b>\n\
         📊 Прирост к прошлому месяцу: <b>{}</b>\n\
         💎 Подписка: <b>{}</b>\n\
         📺 Отслеживание каналов: <b>{}</b>\n\n\
         💡 Совет от Яна: {}\n\n\
         {}\
         📢 Новости и обновления — в канале @yanapomnyu\n\
         ⭐ Интеграция с VPN: @ya_vpnbot",
        nickname,
        active_count,
        this_month,
        growth,
        subscription_status,
        channel_subs_count,
        tip,
        last_reminder_text,
    ))
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

/// Обрезает текст до указанной длины.
fn truncate_text(text: &str, max_len: usize) -> String {
    let mut chars = text.chars();
    let truncated: String = chars.by_ref().take(max_len).collect();
    if chars.next().is_none() {
        text.to_string()
    } else {
        format!("{}...", truncated)
    }
}
