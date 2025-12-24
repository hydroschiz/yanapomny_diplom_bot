//! Обработчики реферальной программы.

use teloxide::prelude::*;
use teloxide::types::{Message, ParseMode};

use crate::api::db::Db;
use crate::bot::keyboards::profile_back_keyboard;
use crate::bot::router::HandlerResult;

/// Обработчик команды /ref.
pub async fn command_ref(bot: Bot, msg: Message, db: Db) -> HandlerResult {
    let chat_id = msg.chat.id;
    let user_id = chat_id.0;

    send_referral_message(&bot, chat_id, user_id, &db).await
}

/// Отправляет сообщение с реферальной ссылкой.
pub async fn send_referral_message(bot: &Bot, chat_id: ChatId, user_id: i64, db: &Db) -> HandlerResult {
    // Получаем имя бота
    let bot_username = match bot.get_me().await {
        Ok(me) => me.username.clone().unwrap_or_else(|| "yanapomnyu_bot".to_string()),
        Err(_) => std::env::var("BOT_USERNAME").unwrap_or_else(|_| "yanapomnyu_bot".to_string()),
    };

    // Считаем приглашённых друзей
    let invited_count = db.count_referrals_by_referrer(user_id).await.unwrap_or(0);

    // Формируем реферальную ссылку с deep link
    let referral_link = format!("https://t.me/{}?start=ref_{}", bot_username, user_id);

    let message = format!(
        "<b>Ваша ссылка для друзей</b>:\n\
         {}\n\n\
         Если человек, приглашенный по вашей реферальной ссылке, оформит подписку, \
         то вы получите <b>1 месяц подписки бесплатно</b>.\n\n\
         Вы пригласили друзей: <b>{}</b>",
        referral_link,
        invited_count
    );

    bot.send_message(chat_id, message)
        .parse_mode(ParseMode::Html)
        .reply_markup(profile_back_keyboard())
        .await?;

    Ok(())
}
