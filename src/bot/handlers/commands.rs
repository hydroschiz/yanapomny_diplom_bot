use super::text::timezone_offset_string;
use crate::api::db::Db;
use crate::bot::{
    keyboards::{profile_back_keyboard, setup_keyboard, utc_keyboard, utc_keyboard_page_count},
    router::HandlerResult,
    states::AppState,
};
use crate::transport::dialogue_store::DialogueStore;
use crate::transport::text_format::strip_html;
use crate::transport::traits::{BotTransport, TransportKeyboard};
use crate::utils::timezone::user_has_timezone;

pub const UTC_PROMPT_MESSAGE: &str = r#"Укажите разницу во времени относительно UTC или просто назовите город, где вы находитесь — ИИ-помощник <b>Ян</b> определит всё сам.

Например:
<b>Москва находится в UTC +3.</b>

Напишите UTC или укажите город, где вы находитесь (данные можно в любое время изменить).
Если нужного смещения нет на кнопках, отправьте его текстом, например UTC+5:45:"#;

pub const UTC_SUCCESS_MESSAGE: &str = r#"Часовой пояс <b>+3:00</b> успешно установлен.

Теперь можно создавать напоминания! <b>Отправь текстовое или голосовое сообщение — и бот всё запомнит</b>.

Примеры запросов: <blockquote>
• через 20 минут позвонить руководителю
• в понедельник в 18 — в поликлинику
• в 13:30 — обед
• завтра в 14 — в налоговую
• 16 сентября в 10:20 — на почту
• 17.04.2025 в 9:15 — поздравить коллегу с днём рождения
• в среду утром — оформить документы
• 9 мая в 19:00 — купить билеты
• каждый день в 18 — домой
• каждую среду в 17:30 — на тренировку
• по будням в 10 — планёрка
• каждое 28 число в 20 — оплатить интернет
• каждое 30 мая — купить подарок на годовщину </blockquote>

⚙️ Чтобы изменить часовой пояс, используйте команду <b>/utc</b>
ℹ️ Дополнительная информация — команда <b>/help</b>"#;

pub const SETUP_PROMPT: &str = r#"<b>Выберите раздел для настройки</b>:

• <b>Время откладывания</b> — время, на которое можно перенести напоминание вручную.
• <b>Авто откладывание</b> — настройка автоматического переноса напоминаний.
• <b>Время суток</b> — укажите часовой пояс (UTC) или отправьте свой город."#;

pub const SNOOZE_PROMPT: &str = r#"<b>Выберите кнопки для откладывания напоминаний</b>
Эти варианты будут показываться при получении напоминания.

По умолчанию: <b>15 мин, 1 час, 3 часа</b>

Введите своё время, которое хотите видеть для откладывания:"#;

pub const AUTO_SNOOZE_PROMPT: &str = r#"Настройте время автоматического откладывания напоминаний

По умолчанию: 15 мин

Введите своё время для автоматического откладывания напоминаний:"#;

// ============================================================================
// Transport-native command handlers for VK router
// ============================================================================

pub async fn command_help_transport<T: BotTransport>(transport: &T, peer_id: i64) -> HandlerResult {
    let text = r#"💬 <b>Создавайте напоминания своими словами!</b>

Вы можете отправлять <b>текстовые</b> или <b>голосовые сообщения</b>, а ИИ-помощник <b>Ян</b> сам распознает, что и когда нужно запланировать.

Примеры:<blockquote>
• через 20 минут позвонить руководителю
• в понедельник в 18 — в поликлинику
• в 13:30 — обед
• завтра в 14 — в налоговую
• 16 сентября в 10:20 — на почту
• 17.04.2017 в 9:15 — поздравить коллегу с днём рождения
• в среду утром — оформить документы
• 9 мая в 19:00 — купить билеты
• каждый день в 18 — домой
• каждую среду в 17:30 — на тренировку
• по будням в 10 — планёрка
• каждое 28 число в 20 — оплатить интернет
• каждое 30 мая — подарок на годовщину</blockquote>

👥 <b>Использование в группах и каналах</b>:
Добавьте бота в групповой чат или канал и настройте часовой пояс с помощью команды /start.

Для напоминаний в группах указывайте имя бота в тексте.

📞 <b>Вопросы или предложения</b>:

Пишите в чат технической поддержки: @yanapomnyu_support"#;

    let keyboard = profile_back_keyboard();
    send_html_with_keyboard(transport, peer_id, text, &keyboard).await
}

pub async fn command_yan_transport<T: BotTransport>(transport: &T, peer_id: i64) -> HandlerResult {
    let text = r#"Привет! Я — <b>Ян</b>, твой персональный ИИ-помощник 🧠 Я помогу тебе управлять временем, делами и напоминаниями.

<b>Вот что я умею</b>:<blockquote>
• Автоматически создавать напоминания по любому тексту — просто напиши, что и когда сделать.
• Подсказывать, как лучше распределить задачи и не перегружать день.
• Давать советы по тайм-менеджменту и концентрации.
• Анализировать твои напоминания и помогать выстроить привычки.</blockquote>

<b>Попробуй прямо сейчас</b>:
💬 "Завтра в 9:30 совещание с командой""#;

    let keyboard = profile_back_keyboard();
    send_html_with_keyboard(transport, peer_id, text, &keyboard).await
}

pub async fn command_utc_transport<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    store: &DialogueStore,
    db: Db,
) -> HandlerResult {
    start_utc_flow_transport(transport, peer_id, user_id, store, db).await
}

pub async fn command_setup_transport<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    store: &DialogueStore,
    db: Db,
) -> HandlerResult {
    store.update(user_id, AppState::Idle);
    db.update_user_state(peer_id, "waiting_for_message").await?;

    let keyboard = setup_keyboard();
    send_html_with_keyboard(transport, peer_id, SETUP_PROMPT, &keyboard).await
}

pub async fn start_utc_flow_transport<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    store: &DialogueStore,
    db: Db,
) -> HandlerResult {
    let user = db.ensure_user(peer_id).await?;
    db.update_user_state(peer_id, "waiting_for_time").await?;
    store.update(user_id, AppState::AwaitingUtc);

    let current_tz = if !user.time_zone.is_empty() {
        let offset =
            timezone_offset_string(&user.time_zone).unwrap_or_else(|| "+00:00".to_string());
        format!(
            "Текущий часовой пояс: <b>{} ({})</b>",
            user.time_zone, offset
        )
    } else if user.utc.to_lowercase() != "nil" && !user.utc.is_empty() {
        format!("Текущий часовой пояс: <b>UTC {}</b>", user.utc)
    } else {
        "Текущий часовой пояс: <b>не установлен</b>".to_string()
    };

    let text = format!(
        "{current_tz}\n\n{UTC_PROMPT_MESSAGE}\n\nСтраница 1/{}",
        utc_keyboard_page_count()
    );
    let keyboard = utc_keyboard();
    send_html_with_keyboard(transport, peer_id, &text, &keyboard).await
}

#[allow(clippy::too_many_arguments)]
pub async fn command_start_transport<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    is_group: bool,
    group_title: Option<&str>,
    store: &DialogueStore,
    db: Db,
) -> HandlerResult {
    let text = r#"<b>YANAPOMNYU</b> — твой личный помощник для организации дел!

Создавай напоминания, планируй задачи и получай уведомления без лишних приложений. Внутри тебя ждёт ИИ-помощник <b>Ян</b> — он мгновенно создаст напоминания, подскажет, как улучшить тайм-менеджмент, и поможет быть продуктивнее.

Узнай больше о Яне через команду /yan.

✨ Возможности бота:<blockquote>
• Создание напоминаний на любую дату и время
• Отслеживание всех задач в одном месте
• Автоматические уведомления о важных делах</blockquote>

📺 <b>Получай уведомления о новых видео и трансляциях бесплатно!</b>
Подписка не нужна — просто отправь ссылку через <b>/subs</b> (YouTube или Twitch), и я буду напоминать о новом контенте.

📢 Новости и обновления — в канале @yanapomnyu"#;

    if is_group {
        let title = group_title
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("VK chat {}", peer_id));
        let _ = db.ensure_group_record(peer_id, title, user_id).await;

        let user = db.ensure_user(peer_id).await?;
        if user.time_zone.is_empty() && (user.utc.is_empty() || user.utc.to_lowercase() == "nil") {
            start_utc_flow_transport(transport, peer_id, user_id, store, db).await?;
            return Ok(());
        }

        send_html_text(transport, peer_id, text).await?;
        return Ok(());
    }

    // TODO(vk-migration): реферальные ссылки VK
    send_html_text(transport, peer_id, text).await?;

    let user = db.ensure_user(peer_id).await?;
    let _ = db.ensure_record(peer_id).await?;
    if user.time_zone.is_empty() && (user.utc.is_empty() || user.utc.to_lowercase() == "nil") {
        start_utc_flow_transport(transport, peer_id, user_id, store, db).await?;
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn command_remind_transport<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    is_group: bool,
    group_title: Option<&str>,
    text: String,
    store: &DialogueStore,
    db: Db,
    config: crate::config::Config,
) -> HandlerResult {
    let reminder_text = text.trim().to_string();
    if reminder_text.is_empty() {
        transport
            .send_text(
                peer_id,
                &format!(
                    "Используйте команду так: /remind@{} через 10 минут написать в чат",
                    config.bot_username
                ),
            )
            .await?;
        return Ok(());
    }

    if is_group {
        let title = group_title
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("VK chat {}", peer_id));
        let _ = db.ensure_group_record(peer_id, title, user_id).await?;

        let user = db.ensure_user(peer_id).await?;
        if !user_has_timezone(&user) {
            transport
                .send_text(
                    peer_id,
                    &format!(
                        "Сначала настройте часовой пояс для этого чата через /start@{} или /utc@{}",
                        config.bot_username, config.bot_username
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

        return super::reminder::start_reminder_creation_flow_transport(
            transport,
            peer_id,
            user_id,
            reminder_text,
            store,
        )
        .await;
    }

    let user = match db.find_user(peer_id).await? {
        Some(user) => user,
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
        send_html_text(
            transport,
            peer_id,
            "⚠️ <b>Подписка не активна</b>\n\nДля создания напоминаний необходима активная подписка.\n\nИспользуйте команду /pay для оформления подписки.",
        )
        .await?;
        return Ok(());
    }

    super::reminder::start_reminder_creation_flow_transport(
        transport,
        peer_id,
        user_id,
        reminder_text,
        store,
    )
    .await
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
