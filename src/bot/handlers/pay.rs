//! Payment handlers for /pay command and payment callbacks.

use std::sync::Arc;

use teloxide::prelude::*;
use teloxide::types::{InlineKeyboardButton as Btn, InlineKeyboardMarkup, ParseMode};

use crate::api::db::Db;
use crate::api::payments::{PaymentService, get_tariff};
use crate::bot::router::{AppDialogue, HandlerResult};
use crate::bot::states::AppState;

/// Build the main payment menu keyboard.
pub fn pay_menu_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![Btn::callback("3 месяца за 195₽ (65₽/мес)", "pay_select:3")],
        vec![Btn::callback("6 месяцев за 360₽ (60₽/мес)", "pay_select:6")],
        vec![Btn::callback("12 месяцев за 660₽ (55₽/мес)", "pay_select:12")],
        vec![Btn::url("О сервисе", "https://t.me/yanapomnyu".parse().unwrap())],
        vec![Btn::callback("⬅ Назад", "pay_cancel")],
    ])
}

/// Build provider selection keyboard.
pub fn pay_provider_keyboard(months: i32) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![Btn::callback("💳 Карта, SberPay, ЮMoney", format!("pay_yk:{}", months))],
        vec![Btn::callback("⬅ Назад", "pay_cancel")],
    ])
}

/// Build payment link keyboard (after provider selection).
pub fn pay_link_keyboard(url: &str, months: i32) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![Btn::url("💳 Карта, SberPay, ЮMoney", url.parse().unwrap())],
        vec![Btn::callback("🔄 Проверить оплату", format!("pay_check:{}", months))],
        vec![Btn::callback("⬅ Назад", "pay_cancel")],
    ])
}

/// Format subscription status message.
pub fn format_subscription_status(is_active: bool, expiry: Option<&str>) -> String {
    let status = if is_active { "активна ✅" } else { "неактивна ❌" };
    let expiry_line = if is_active {
        expiry.map(|e| format!("\n<b>Действует до:</b> {}", e)).unwrap_or_default()
    } else {
        String::new()
    };

    format!(
        "<b>Оплата подписки — @yanapomnyu_bot</b>\n\n\
         <b>Статус:</b> {}{}\n\n\
         <b>Продлить подписку:</b>",
        status, expiry_line
    )
}

/// Format tariff selection message.
pub fn format_tariff_message(months: i32) -> String {
    let tariff = get_tariff(months);
    let (price, period) = match tariff {
        Some(t) => (t.price, format!("{} мес.", t.months)),
        None => (0, "?".to_string()),
    };

    format!(
        "Вы оплачиваете подписку на <b>{}</b>\n\n\
         💸 <b>К оплате:</b> {} ₽\n\n\
         Оплачивая подписку, вы принимаете условия пользовательского соглашения.\n\n\
         Для оплаты воспользуйтесь кнопками под этим сообщением.",
        period, price
    )
}

/// Handle /pay command - show subscription status and payment options.
pub async fn command_pay(
    bot: Bot,
    msg: Message,
    dialogue: AppDialogue,
    db: Db,
) -> HandlerResult {
    let user_id = msg.chat.id.0;

    // Get subscription status
    let record = db.find_record(user_id).await?;
    let (is_active, expiry) = match &record {
        Some(r) => (r.is_active(), Some(r.expiry_formatted())),
        None => (false, None),
    };

    let text = format_subscription_status(is_active, expiry.as_deref());

    dialogue.update(AppState::Idle).await?;

    bot.send_message(msg.chat.id, text)
        .parse_mode(ParseMode::Html)
        .reply_markup(pay_menu_keyboard())
        .await?;

    Ok(())
}

/// Handle payment-related callbacks.
pub async fn handle_pay_callback(
    bot: Bot,
    cq: CallbackQuery,
    dialogue: AppDialogue,
    _db: Db,
    payment_svc: Arc<PaymentService>,
) -> HandlerResult {
    let data = match cq.data.as_ref() {
        Some(d) => d.clone(),
        None => return Ok(()),
    };
    let chat_id = match &cq.message {
        Some(msg) => msg.chat().id,
        None => return Ok(()),
    };
    let user_id = chat_id.0;

    // pay_cancel - return to /start
    if data == "pay_cancel" {
        dialogue.update(AppState::Idle).await?;
        bot.answer_callback_query(cq.id.clone()).await?;
        bot.send_message(chat_id, "Оплата отменена.")
            .await?;
        return Ok(());
    }

    // pay_select:N - select tariff
    if let Some(rest) = data.strip_prefix("pay_select:") {
        if let Ok(months) = rest.parse::<i32>() {
            bot.answer_callback_query(cq.id.clone()).await?;

            let text = format_tariff_message(months);
            bot.send_message(chat_id, text)
                .parse_mode(ParseMode::Html)
                .reply_markup(pay_provider_keyboard(months))
                .await?;

            dialogue.update(AppState::AwaitingPayment { months }).await?;
            return Ok(());
        }
    }

    // pay_yk:N - YooKassa payment
    if let Some(rest) = data.strip_prefix("pay_yk:") {
        if let Ok(months) = rest.parse::<i32>() {
            bot.answer_callback_query(cq.id.clone()).await?;

            // Initialize or get existing payment
            match payment_svc.init_or_get_last(user_id, months).await {
                Ok(payment) => {
                    let text = format_tariff_message(months);

                    // Send message with payment link
                    bot.send_message(chat_id, text)
                        .parse_mode(ParseMode::Html)
                        .reply_markup(pay_link_keyboard(&payment.confirmation_url, months))
                        .await?;

                    // Send instruction message
                    bot.send_message(
                        chat_id,
                        "🚀 После оплаты вернитесь в чат — мы обработаем платеж автоматически.\n\n\
                         Если не пришло сообщение — нажмите <b>Проверить оплату</b>.",
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;

                    dialogue.update(AppState::AwaitingPayment { months }).await?;
                }
                Err(e) => {
                    tracing::error!(%e, "failed to create payment");
                    bot.send_message(chat_id, "❌ Не удалось создать платёж. Попробуйте позже.")
                        .await?;
                }
            }
            return Ok(());
        }
    }

    // pay_check:N - check payment status
    if let Some(rest) = data.strip_prefix("pay_check:") {
        if let Ok(_months) = rest.parse::<i32>() {
            bot.answer_callback_query(cq.id.clone())
                .text("Проверяем статус платежа...")
                .await?;

            // Get pending payment from cache
            match payment_svc.get_pending_payment(user_id).await {
                Ok(Some(pending)) => {
                    match payment_svc.manual_check(&bot, user_id, &pending.payment_id).await {
                        Ok(msg) => {
                            let is_success = msg.contains("✅");
                            bot.send_message(chat_id, msg).await?;
                            if is_success {
                                dialogue.update(AppState::Idle).await?;
                            }
                        }
                        Err(e) => {
                            tracing::error!(%e, "payment check failed");
                            bot.send_message(chat_id, "❌ Не удалось проверить платёж. Попробуйте позже.")
                                .await?;
                        }
                    }
                }
                Ok(None) => {
                    bot.send_message(chat_id, "Нет активных платежей для проверки. Оформите новый платёж.")
                        .await?;
                }
                Err(e) => {
                    tracing::error!(%e, "failed to get pending payment");
                    bot.send_message(chat_id, "❌ Ошибка при проверке. Попробуйте позже.")
                        .await?;
                }
            }
            return Ok(());
        }
    }

    Ok(())
}
