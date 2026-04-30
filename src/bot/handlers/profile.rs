//! Обработчики команды /profile и профиля пользователя.

use teloxide::prelude::*;
use teloxide::types::{Message, ParseMode};

use crate::api::db::Db;
use crate::bot::keyboards::profile_keyboard;
use crate::bot::router::HandlerResult;
use crate::scheduler::format_full_reminder_time_for_user;

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

/// Обработчик команды /profile.
pub async fn handle_profile_command(bot: Bot, msg: Message, db: Db) -> HandlerResult {
    let chat_id = msg.chat.id;
    let user_id = chat_id.0;

    // Получаем данные пользователя
    let user = db.ensure_user(user_id).await?;
    let record = db.ensure_record(user_id).await?;

    // Собираем статистику
    let active_count = db.count_active_reminders(user_id).await.unwrap_or(0);
    let this_month = db.count_reminders_this_month(user_id).await.unwrap_or(0);
    let last_month = db.count_reminders_last_month(user_id).await.unwrap_or(0);
    
    // Считаем прирост
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

    // Статус подписки
    let subscription_status = if record.is_active() {
        format!("активна до {}", record.expiry_formatted())
    } else if record.free_state == Some(1) {
        "пробный период".to_string()
    } else {
        "не активна".to_string()
    };

    // Количество подписок на каналы
    let channel_subs_count = db.get_user_channel_subs(user_id).await
        .map(|subs| subs.len())
        .unwrap_or(0);

    // Случайный совет
    let tip_index = (user_id as usize) % TIPS.len();
    let tip = TIPS[tip_index];

    // Последнее напоминание
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

    // Никнейм (или ID)
    let nickname = msg.from
        .as_ref()
        .and_then(|u| u.username.clone())
        .map(|u| format!("@{}", u))
        .unwrap_or_else(|| format!("#{}", user_id));

    // Формируем сообщение профиля
    let message = format!(
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
    );

    bot.send_message(chat_id, message)
        .parse_mode(ParseMode::Html)
        .reply_markup(profile_keyboard())
        .await?;

    Ok(())
}

/// Обрезает текст до указанной длины.
fn truncate_text(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        text.to_string()
    } else {
        format!("{}...", &text[..max_len])
    }
}
