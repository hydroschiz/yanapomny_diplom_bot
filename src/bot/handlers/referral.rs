//! Обработчики реферальной программы.

use crate::api::db::Db;
use crate::bot::keyboards::profile_back_keyboard;
use crate::bot::router::HandlerResult;
use crate::transport::traits::{BotTransport, TransportKeyboard};

const REFERRAL_UNAVAILABLE: &str = "Реферальная программа временно недоступна";

/// Обработчик команды /ref через абстрактный транспорт.
pub async fn command_ref_transport<T: BotTransport>(transport: &T, peer_id: i64) -> HandlerResult {
    send_unavailable(transport, peer_id).await
}

/// Отправляет сообщение о временной недоступности реферальной программы.
pub async fn send_referral_message<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    _user_id: i64,
    _db: &Db,
) -> HandlerResult {
    send_unavailable(transport, peer_id).await
}

async fn send_unavailable<T: BotTransport>(transport: &T, peer_id: i64) -> HandlerResult {
    // TODO(vk-migration): реферальные ссылки VK
    let keyboard = profile_back_keyboard();
    send_with_keyboard(transport, peer_id, REFERRAL_UNAVAILABLE, &keyboard).await
}

async fn send_with_keyboard<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    text: &str,
    keyboard: &TransportKeyboard,
) -> HandlerResult {
    transport
        .send_with_keyboard(peer_id, text, keyboard)
        .await?;
    Ok(())
}
