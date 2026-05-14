#[cfg(feature = "telegram-legacy")]
use teloxide::{prelude::*, types::ParseMode, utils::command::BotCommands};

use super::text::timezone_offset_string;
use crate::api::db::Db;
#[cfg(feature = "telegram-legacy")]
use crate::bot::router::AppDialogue;
use crate::bot::{
    keyboards::{profile_back_keyboard, setup_keyboard, utc_keyboard, utc_keyboard_page_count},
    router::HandlerResult,
    states::AppState,
};
use crate::transport::dialogue_store::DialogueStore;
use crate::transport::text_format::strip_html;
use crate::transport::traits::{BotTransport, TransportKeyboard};
use crate::utils::timezone::user_has_timezone;

#[cfg(feature = "telegram-legacy")]
#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Доступные команды:")]
pub enum Command {
    #[command(description = "Начать")]
    Start,
    #[command(description = "Дополнительная информация")]
    Help,
    #[command(description = "ИИ-помощник Yan")]
    Yan,
    #[command(description = "Настройка часового пояса")]
    Utc,
    #[command(description = "Настройки")]
    Setup,
    #[command(description = "Оплата")]
    Pay,
    #[command(description = "Список активных напоминаний")]
    List,
    #[command(description = "Уведомления о новых видео")]
    Subs,
    #[command(description = "Профиль и подписка")]
    Profile,
    #[command(description = "Реферальная ссылка")]
    Ref,
    #[command(description = "Создать напоминание: /remind текст")]
    Remind(String),
}

#[cfg(feature = "telegram-legacy")]
pub fn router() -> teloxide::dispatching::UpdateHandler<anyhow::Error> {
    use teloxide::dispatching::UpdateFilterExt;

    dptree::entry().branch(
        Update::filter_message()
            .filter_command::<Command>()
            .branch(dptree::case![Command::Start].endpoint(command_start))
            .branch(dptree::case![Command::Help].endpoint(command_help))
            .branch(dptree::case![Command::Yan].endpoint(command_yan))
            .branch(dptree::case![Command::Utc].endpoint(command_utc))
            .branch(dptree::case![Command::Setup].endpoint(command_setup))
            .branch(dptree::case![Command::Pay].endpoint(super::pay::command_pay))
            .branch(dptree::case![Command::List].endpoint(super::reminder::handle_list_command))
            .branch(dptree::case![Command::Subs].endpoint(super::channels::command_subs))
            .branch(
                dptree::case![Command::Profile].endpoint(super::profile::handle_profile_command),
            )
            .branch(dptree::case![Command::Ref].endpoint(super::referral::command_ref))
            .branch(dptree::case![Command::Remind(text)].endpoint(command_remind)),
    )
}

#[cfg(feature = "telegram-legacy")]
async fn command_remind(
    bot: Bot,
    msg: Message,
    dialogue: AppDialogue,
    db: Db,
    config: crate::config::Config,
    text: String,
) -> HandlerResult {
    let reminder_text = text.trim().to_string();
    if reminder_text.is_empty() {
        bot.send_message(
            msg.chat.id,
            format!(
                "Используйте команду так: /remind@{} через 10 минут написать в чат",
                config.bot_username
            ),
        )
        .await?;
        return Ok(());
    }

    let chat_id = msg.chat.id;
    let chat_key = chat_id.0;

    if let teloxide::types::ChatKind::Public(chat) = &msg.chat.kind {
        let title = chat.title.clone().unwrap_or_else(|| "Group".to_string());
        let owner_id = msg
            .from
            .as_ref()
            .map(|user| user.id.0 as i64)
            .unwrap_or(chat_key);
        let _ = db.ensure_group_record(chat_key, title, owner_id).await?;

        let user = db.ensure_user(chat_key).await?;
        if !user_has_timezone(&user) {
            bot.send_message(
                chat_id,
                format!(
                    "Сначала настройте часовой пояс для этого чата через /start@{} или /utc@{}",
                    config.bot_username, config.bot_username
                ),
            )
            .await?;
            return Ok(());
        }

        let mut is_allowed = db.is_subscription_active(chat_key).await?;
        if !is_allowed {
            if let Some(user) = &msg.from {
                if db.is_subscription_active(user.id.0 as i64).await? {
                    is_allowed = true;
                }
            }
        }
        if !is_allowed {
            if let Some(record) = db.find_record(chat_key).await? {
                if let Some(owner_id) = record.owner_id {
                    if db.is_subscription_active(owner_id).await? {
                        is_allowed = true;
                    }
                }
            }
        }

        if !is_allowed {
            bot.send_message(msg.chat.id, "⚠️ Подписка не активна. Бот работает в группах, если у группы, отправителя или добавившего администратора есть активная подписка.")
                .await?;
            return Ok(());
        }

        return super::reminder::start_reminder_creation_flow(
            bot,
            chat_id,
            reminder_text,
            dialogue,
        )
        .await;
    }

    let user = match db.find_user(chat_key).await? {
        Some(user) => user,
        None => {
            bot.send_message(
                chat_id,
                "Пожалуйста, сначала настройте часовой пояс командой /start",
            )
            .await?;
            return Ok(());
        }
    };

    if !user_has_timezone(&user) {
        bot.send_message(
            chat_id,
            "Пожалуйста, сначала настройте часовой пояс командой /utc",
        )
        .await?;
        return Ok(());
    }

    let record = db.ensure_record(chat_key).await?;
    if !record.is_active() {
        bot.send_message(
            chat_id,
            "⚠️ <b>Подписка не активна</b>\n\nДля создания напоминаний необходима активная подписка.\n\nИспользуйте команду /pay для оформления подписки.",
        )
        .parse_mode(ParseMode::Html)
        .await?;
        return Ok(());
    }

    super::reminder::start_reminder_creation_flow(bot, chat_id, reminder_text, dialogue).await
}

#[cfg(feature = "telegram-legacy")]
async fn command_help(bot: Bot, msg: Message) -> HandlerResult {
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
Добавьте бота в групповой чат или канал и настройте часовой пояс с помощью команды: /start@yanapomnyu_bot

Для напоминаний в группах указывайте имя бота в тексте:
@yanapomnyu_bot через 10 минут на планерку

📞 <b>Вопросы или предложения</b>:

Пишите в чат технической поддержки: @yanapomnyu_support"#;

    bot.send_message(msg.chat.id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(crate::bot::keyboards::profile_back_keyboard())
        .await?;

    Ok(())
}

#[cfg(feature = "telegram-legacy")]
async fn command_yan(bot: Bot, msg: Message) -> HandlerResult {
    let text = r#"Привет! Я — <b>Ян</b>, твой персональный ИИ-помощник 🧠 Я помогу тебе управлять временем, делами и напоминаниями прямо в Telegram.

<b>Вот что я умею</b>:<blockquote>
• Автоматически создавать напоминания по любому тексту — просто напиши, что и когда сделать.
• Подсказывать, как лучше распределить задачи и не перегружать день.
• Давать советы по тайм-менеджменту и концентрации.
• Анализировать твои напоминания и помогать выстроить привычки.
• Работать с голосовыми сообщениями — просто скажи, что запланировать.</blockquote>

<b>Попробуй прямо сейчас</b>:
💬 "Завтра в 9:30 совещание с командой"
или
🎙️ Отправь голосовое сообщение, и я всё сделаю сам!"#;

    bot.send_message(msg.chat.id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(crate::bot::keyboards::profile_back_keyboard())
        .await?;

    Ok(())
}

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

// Клавиатуры setup_keyboard и back_keyboard перенесены в crate::bot::keyboards::common

#[cfg(feature = "telegram-legacy")]
pub async fn command_utc(bot: Bot, msg: Message, dialogue: AppDialogue, db: Db) -> HandlerResult {
    start_utc_flow(bot, msg.chat.id, dialogue, db).await
}

#[cfg(feature = "telegram-legacy")]
pub async fn command_setup(bot: Bot, msg: Message, dialogue: AppDialogue, db: Db) -> HandlerResult {
    dialogue.update(AppState::Idle).await?;
    db.update_user_state(msg.chat.id.0, "waiting_for_message")
        .await?;

    bot.send_message(msg.chat.id, SETUP_PROMPT)
        .parse_mode(ParseMode::Html)
        .reply_markup(setup_keyboard())
        .await?;
    Ok(())
}

#[cfg(feature = "telegram-legacy")]
pub async fn start_utc_flow(
    bot: Bot,
    chat_id: ChatId,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let user = db.ensure_user(chat_id.0).await?;
    db.update_user_state(chat_id.0, "waiting_for_time").await?;
    dialogue.update(AppState::AwaitingUtc).await?;

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

    bot.send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(utc_keyboard())
        .await?;

    Ok(())
}

#[cfg(feature = "telegram-legacy")]
pub async fn command_start(bot: Bot, msg: Message, dialogue: AppDialogue, db: Db) -> HandlerResult {
    let text = r#"<b>YANAPOMNYU</b> — твой личный помощник для организации дел прямо в Telegram!

Создавай напоминания, планируй задачи и получай уведомления без лишних приложений. Внутри тебя ждёт ИИ-помощник <b>Ян</b> — он мгновенно создаст напоминания, подскажет, как улучшить тайм-менеджмент, и поможет быть продуктивнее.

Узнай больше о Яне через команду /yan.

✨ Возможности бота:<blockquote>
• Создание напоминаний на любую дату и время
• Отслеживание всех задач в одном месте
• Автоматические уведомления о важных делах</blockquote>

📺 <b>Получай уведомления о новых видео и трансляциях бесплатно!</b>
Подписка не нужна — просто отправь ссылку через <b>/subs</b> (YouTube или Twitch), и я буду напоминать о новом контенте.

📢 Новости и обновления — в канале @yanapomnyu"#;

    let user_id = msg.chat.id.0;

    // Check if it's a group chat
    use teloxide::types::ChatKind;
    if let ChatKind::Public(chat) = &msg.chat.kind {
        // Create/Update group record
        let title = chat.title.clone().unwrap_or_else(|| "Group".to_string());
        if let Some(from) = &msg.from {
            let _ = db
                .ensure_group_record(user_id, title, from.id.0 as i64)
                .await;
        }

        // Also ensure User struct for preferences
        let user = db.ensure_user(user_id).await?;
        if user.time_zone.is_empty() && (user.utc.is_empty() || user.utc.to_lowercase() == "nil") {
            // For groups, just send message about setting UTC, don't start dialogue flow (state issues)
            // Or we can start it if we assume one admin interacting
            start_utc_flow(bot, msg.chat.id, dialogue, db).await?;
            return Ok(());
        }

        bot.send_message(msg.chat.id, text)
            .parse_mode(ParseMode::Html)
            .await?;

        return Ok(());
    }

    // TODO(vk-migration): реферальные ссылки VK

    bot.send_message(msg.chat.id, text)
        .parse_mode(ParseMode::Html)
        .await?;

    let user = db.ensure_user(user_id).await?;
    let _ = db.ensure_record(user_id).await?;
    if user.time_zone.is_empty() && (user.utc.is_empty() || user.utc.to_lowercase() == "nil") {
        start_utc_flow(bot, msg.chat.id, dialogue, db).await?;
    }

    Ok(())
}

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
