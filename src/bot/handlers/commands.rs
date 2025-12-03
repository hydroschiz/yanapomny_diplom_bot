use teloxide::{prelude::*, types::ParseMode, utils::command::BotCommands};

use super::callbacks::utc_keyboard;
use super::text::timezone_offset_string;
use crate::api::db::Db;
use crate::bot::{
    filters,
    router::{AppDialogue, HandlerResult},
    states::AppState,
};

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Доступные команды:")]
pub enum Command {
    #[command(description = "Старт")]
    Start,
    #[command(description = "Помощь")]
    Help,
    #[command(description = "Настройка часового пояса")]
    Utc,
    #[command(description = "Настройки пользователя")]
    Setup,
}

pub fn router() -> teloxide::dispatching::UpdateHandler<anyhow::Error> {
    use teloxide::dispatching::UpdateFilterExt;

    dptree::entry().branch(
        Update::filter_message()
            .filter_command::<Command>()
            .filter(filters::private_chat_msg)
            .branch(dptree::case![Command::Start].endpoint(command_start))
            .branch(dptree::case![Command::Help].endpoint(command_help))
            .branch(dptree::case![Command::Utc].endpoint(command_utc))
            .branch(dptree::case![Command::Setup].endpoint(command_setup)),
    )
}

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
        .await?;

    Ok(())
}

pub const UTC_PROMPT_MESSAGE: &str = r#"Укажите разницу во времени относительно UTC или просто назовите город, где вы находитесь — ИИ-помощник <b>Ян</b> определит всё сам.

Например:
<b>Москва находится в UTC +3.</b>

Напишите UTC или укажите город, где вы находитесь (данные можно в любое время изменить):"#;

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

pub fn setup_keyboard() -> teloxide::types::InlineKeyboardMarkup {
    use teloxide::types::InlineKeyboardButton as Btn;
    teloxide::types::InlineKeyboardMarkup::new(vec![
        vec![Btn::callback("Время откладывания", "setup_snooze")],
        vec![Btn::callback("Авто откладывание", "setup_auto")],
        vec![Btn::callback("Время суток (UTC)", "setup_utc")],
    ])
}

pub fn back_keyboard() -> teloxide::types::InlineKeyboardMarkup {
    use teloxide::types::InlineKeyboardButton as Btn;
    teloxide::types::InlineKeyboardMarkup::new(vec![vec![Btn::callback("⬅ Назад", "setup_menu")]])
}

async fn command_utc(bot: Bot, msg: Message, dialogue: AppDialogue, db: Db) -> HandlerResult {
    start_utc_flow(bot, msg.chat.id, dialogue, db).await
}

async fn command_setup(bot: Bot, msg: Message, dialogue: AppDialogue, db: Db) -> HandlerResult {
    dialogue.update(AppState::Idle).await?;
    db.update_user_state(msg.chat.id.0, "waiting_for_message").await?;

    bot.send_message(msg.chat.id, SETUP_PROMPT)
        .parse_mode(ParseMode::Html)
        .reply_markup(setup_keyboard())
        .await?;
    Ok(())
}

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
        let offset = timezone_offset_string(&user.time_zone)
            .unwrap_or_else(|| "+00:00".to_string());
        format!("Текущий часовой пояс: <b>{} ({})</b>", user.time_zone, offset)
    } else if user.utc.to_lowercase() != "nil" && !user.utc.is_empty() {
        format!("Текущий часовой пояс: <b>UTC {}</b>", user.utc)
    } else {
        "Текущий часовой пояс: <b>не установлен</b>".to_string()
    };

    let text = format!("{current_tz}\n\n{UTC_PROMPT_MESSAGE}");

    bot.send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(utc_keyboard())
        .await?;

    Ok(())
}

async fn command_start(bot: Bot, msg: Message, dialogue: AppDialogue, db: Db) -> HandlerResult {
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

    bot.send_message(msg.chat.id, text)
        .parse_mode(ParseMode::Html)
        .await?;

    let user = db.ensure_user(msg.chat.id.0).await?;
    if user.time_zone.is_empty() && (user.utc.is_empty() || user.utc.to_lowercase() == "nil") {
        start_utc_flow(bot, msg.chat.id, dialogue, db).await?;
    }

    Ok(())
}
