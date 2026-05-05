//! Payment handlers for /pay command and payment callbacks.

use std::sync::Arc;

use anyhow::Context;
use chrono::Utc;
use teloxide::prelude::*;

use crate::api::db::{Db, PaymentTransaction};
use crate::api::payments::{get_tariff, PaymentService};
use crate::bot::keyboards::{pay_link_keyboard, pay_menu_keyboard, pay_provider_keyboard};
use crate::bot::router::{AppDialogue, HandlerResult};
use crate::bot::states::AppState;
use crate::transport::adapters::TelegramTransport;
use crate::transport::dialogue_store::DialogueStore;
use crate::transport::text_format::strip_html;
use crate::transport::traits::{BotTransport, TransportKeyboard};

/// Format subscription status message.
pub fn format_subscription_status(is_active: bool, expiry: Option<&str>) -> String {
    let status = if is_active { "активна ✅" } else { "неактивна ❌" };
    let expiry_line = if is_active {
        expiry
            .map(|e| format!("\n📅 <b>Действует до:</b> {}", e))
            .unwrap_or_default()
    } else {
        String::new()
    };

    format!(
        "👛 <b>Выберите срок, на который хотите оформить подписку</b>\n\n\
         📧 <b>Статус:</b> {}{}\n\n\
         <i>Совет:</i> <b>выбирайте более длительную подписку</b>, чтобы снизить стоимость одного месяца.",
        status, expiry_line
    )
}

/// Format tariff selection message.
pub fn format_tariff_message(months: i32) -> String {
    let tariff = get_tariff(months);
    let (price, period, price_per_month) = match tariff {
        Some(t) => (t.price, format!("{} месяца", t.months), t.price_per_month),
        None => (0, "?".to_string(), 0),
    };

    format!(
        "Вы оплачиваете <b>подписку на {}</b>\n\n\
         💸 <b>К оплате:</b> {}₽ ({}₽ за мес)\n\n\
         Оплачивая подписку, вы <a href=\"https://telegra.ph/Polzovatelskoe-soglashenie-i-publichnaya-oferta-12-03\">принимаете условия пользовательского соглашения</a>.\n\n\
         Для оплаты воспользуйтесь кнопками под этим сообщением.",
        period, price, price_per_month
    )
}

/// Handle /pay command through transport abstraction.
pub async fn command_pay_transport<T: BotTransport>(
    transport: &T,
    peer_id: i64,
    user_id: i64,
    store: &DialogueStore,
    db: Db,
    payment_svc: Arc<PaymentService>,
) -> HandlerResult {
    if !payment_svc.is_enabled() {
        store.update(user_id, AppState::Idle);
        transport
            .send_text(
                peer_id,
                "⚠️ Платёжный контур сейчас отключён. Базовые сценарии напоминаний работают в reminder-only режиме.",
            )
            .await?;
        return Ok(());
    }

    let record = db.find_record(user_id).await?;
    let (is_active, expiry) = match &record {
        Some(r) => (r.is_active(), Some(r.expiry_formatted())),
        None => (false, None),
    };

    let text = format_subscription_status(is_active, expiry.as_deref());
    let keyboard = pay_menu_keyboard();
    store.update(user_id, AppState::Idle);

    send_html_with_keyboard(transport, peer_id, &text, &keyboard).await
}

/// Временный Telegram entrypoint до переключения app/router на VK.
pub async fn command_pay(
    bot: Bot,
    msg: Message,
    dialogue: AppDialogue,
    db: Db,
    payment_svc: Arc<PaymentService>,
) -> HandlerResult {
    let peer_id = msg.chat.id.0;
    let user_id = msg.from.as_ref().map(|user| user.id.0 as i64).unwrap_or(peer_id);
    let transport = TelegramTransport::new(bot);

    if !payment_svc.is_enabled() {
        dialogue.update(AppState::Idle).await?;
        transport
            .send_text(
                peer_id,
                "⚠️ Платёжный контур сейчас отключён. Базовые сценарии напоминаний работают в reminder-only режиме.",
            )
            .await?;
        return Ok(());
    }

    let record = db.find_record(user_id).await?;
    let (is_active, expiry) = match &record {
        Some(r) => (r.is_active(), Some(r.expiry_formatted())),
        None => (false, None),
    };

    let text = format_subscription_status(is_active, expiry.as_deref());
    let keyboard = pay_menu_keyboard();
    dialogue.update(AppState::Idle).await?;

    send_html_with_keyboard(&transport, peer_id, &text, &keyboard).await
}

/// Handle payment-related callbacks through transport abstraction.
#[allow(clippy::too_many_arguments)]
pub async fn handle_pay_callback_transport<T: BotTransport>(
    transport: &T,
    event_id: &str,
    user_id: i64,
    peer_id: i64,
    payload: &str,
    store: &DialogueStore,
    db: Db,
    payment_svc: Arc<PaymentService>,
) -> HandlerResult {
    handle_pay_callback_core(
        transport,
        event_id,
        user_id,
        peer_id,
        payload,
        Some(store),
        None,
        db,
        payment_svc,
    )
    .await
}

/// Временный Telegram callback entrypoint до переключения app/router на VK.
pub async fn handle_pay_callback(
    bot: Bot,
    cq: CallbackQuery,
    dialogue: AppDialogue,
    db: Db,
    payment_svc: Arc<PaymentService>,
) -> HandlerResult {
    let data = match cq.data.as_ref() {
        Some(d) => d.clone(),
        None => return Ok(()),
    };
    let peer_id = match &cq.message {
        Some(msg) => msg.chat().id.0,
        None => return Ok(()),
    };
    let user_id = cq.from.id.0 as i64;
    let transport = TelegramTransport::new(bot);

    handle_pay_callback_core(
        &transport,
        &cq.id.0,
        user_id,
        peer_id,
        &data,
        None,
        Some(&dialogue),
        db,
        payment_svc,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn handle_pay_callback_core<T: BotTransport>(
    transport: &T,
    event_id: &str,
    user_id: i64,
    peer_id: i64,
    data: &str,
    store: Option<&DialogueStore>,
    dialogue: Option<&AppDialogue>,
    db: Db,
    payment_svc: Arc<PaymentService>,
) -> HandlerResult {
    if !payment_svc.is_enabled() && data != "pay_cancel" {
        update_state(store, dialogue, user_id, AppState::Idle).await?;
        transport
            .answer_callback(event_id, user_id, peer_id, None)
            .await?;
        transport
            .send_text(
                peer_id,
                "⚠️ Платёжный контур сейчас отключён. Напоминания и список доступны без оплаты.",
            )
            .await?;
        return Ok(());
    }

    if data == "pay_cancel" {
        update_state(store, dialogue, user_id, AppState::Idle).await?;
        transport
            .answer_callback(event_id, user_id, peer_id, None)
            .await?;
        transport.send_text(peer_id, "Оплата отменена.").await?;
        return Ok(());
    }

    if data == "pay_menu" {
        transport
            .answer_callback(event_id, user_id, peer_id, None)
            .await?;
        update_state(store, dialogue, user_id, AppState::Idle).await?;

        let record = db.ensure_record(user_id).await?;
        let status = if record.is_active() {
            format!(
                "💎 Подписка активна до: <b>{}</b>\n\n",
                record.next_payment_date.format("%d.%m.%Y")
            )
        } else {
            String::new()
        };
        let message = format!("{}Выберите тариф подписки:", status);
        let keyboard = pay_menu_keyboard();

        send_html_with_keyboard(transport, peer_id, &message, &keyboard).await?;
        return Ok(());
    }

    if let Some(rest) = data.strip_prefix("pay_select:") {
        if let Ok(months) = rest.parse::<i32>() {
            transport
                .answer_callback(event_id, user_id, peer_id, None)
                .await?;

            let text = format_tariff_message(months);
            let keyboard = pay_provider_keyboard(months);
            send_html_with_keyboard(transport, peer_id, &text, &keyboard).await?;

            update_state(store, dialogue, user_id, AppState::AwaitingPayment { months }).await?;
            return Ok(());
        }
    }

    if let Some(rest) = data.strip_prefix("pay_yk:") {
        if let Ok(months) = rest.parse::<i32>() {
            transport
                .answer_callback(event_id, user_id, peer_id, None)
                .await?;

            match payment_svc.init_or_get_last(user_id, months).await {
                Ok(payment) => {
                    let text = format_tariff_message(months);
                    let keyboard = pay_link_keyboard(&payment.confirmation_url, months);

                    send_html_with_keyboard(transport, peer_id, &text, &keyboard).await?;
                    send_html_text(
                        transport,
                        peer_id,
                        "🚀 После оплаты вернитесь в чат — мы обработаем платеж автоматически.\n\n\
                         Если не пришло сообщение — нажмите <b>Проверить оплату</b>.",
                    )
                    .await?;

                    update_state(store, dialogue, user_id, AppState::AwaitingPayment { months })
                        .await?;
                }
                Err(e) => {
                    tracing::error!(%e, "failed to create payment");
                    transport
                        .send_text(peer_id, "❌ Не удалось создать платёж. Попробуйте позже.")
                        .await?;
                }
            }
            return Ok(());
        }
    }

    if let Some(rest) = data.strip_prefix("pay_check:") {
        if rest.parse::<i32>().is_ok() {
            transport
                .answer_callback(event_id, user_id, peer_id, Some("Проверяем статус платежа..."))
                .await?;

            match payment_svc.get_pending_payment(user_id).await {
                Ok(Some(pending)) => {
                    match manual_check_transport(&payment_svc, transport, user_id, &pending.payment_id).await {
                        Ok(msg) => {
                            let is_success = msg.contains('✅');
                            transport.send_text(peer_id, &msg).await?;
                            if is_success {
                                update_state(store, dialogue, user_id, AppState::Idle).await?;
                            }
                        }
                        Err(e) => {
                            tracing::error!(%e, "payment check failed");
                            transport
                                .send_text(peer_id, "❌ Не удалось проверить платёж. Попробуйте позже.")
                                .await?;
                        }
                    }
                }
                Ok(None) => {
                    transport
                        .send_text(peer_id, "Нет активных платежей для проверки. Оформите новый платёж.")
                        .await?;
                }
                Err(e) => {
                    tracing::error!(%e, "failed to get pending payment");
                    transport
                        .send_text(peer_id, "❌ Ошибка при проверке. Попробуйте позже.")
                        .await?;
                }
            }
            return Ok(());
        }
    }

    Ok(())
}

async fn manual_check_transport<T: BotTransport>(
    payment_svc: &PaymentService,
    transport: &T,
    user_id: i64,
    payment_id: &str,
) -> anyhow::Result<String> {
    let payment = payment_svc
        .yk_api
        .as_ref()
        .context("payment service is disabled")?
        .get(payment_id)
        .await?;
    let cache = payment_svc.cache.as_ref().context("payment service is disabled")?;

    let status_str = serde_json::to_value(payment.status)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_default();

    let months = payment
        .metadata
        .as_ref()
        .and_then(|m| m.get("months"))
        .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
        .map(|m| m as i32)
        .unwrap_or(3);

    match status_str.as_str() {
        "succeeded" => {
            fulfill_payment_transport(payment_svc, transport, payment_id, user_id, months).await?;
            let _ = cache.delete_by_payment(payment_id).await;
            Ok("✅ Платёж найден и подтверждён! Подписка активирована.".to_string())
        }
        "canceled" => {
            let _ = cache.delete_by_payment(payment_id).await;
            Ok("❌ Платёж отменён. Оформите новый платёж.".to_string())
        }
        "pending" => Ok("⏳ Платёж в обработке. Подождите или попробуйте позже.".to_string()),
        other => Ok(format!(
            "Статус платежа: {}. Подождите или попробуйте позже.",
            other
        )),
    }
}

async fn fulfill_payment_transport<T: BotTransport>(
    payment_svc: &PaymentService,
    transport: &T,
    payment_id: &str,
    user_id: i64,
    months: i32,
) -> anyhow::Result<()> {
    let cache = payment_svc.cache.as_ref().context("payment service is disabled")?;

    if !cache.try_acquire_fulfill_lock(payment_id).await.unwrap_or(true) {
        return Ok(());
    }

    if payment_svc.db.is_transaction_fulfilled(payment_id).await? {
        let _ = cache.release_fulfill_lock(payment_id).await;
        return Ok(());
    }

    let new_expiry = payment_svc.db.extend_subscription(user_id, months).await?;

    if let Err(e) = payment_svc.db.reset_expiry_flags(user_id).await {
        tracing::warn!(user_id = user_id, error = %e, "failed to reset expiry flags");
    }

    let tariff = get_tariff(months);
    let tx = PaymentTransaction {
        payment_id: payment_id.to_string(),
        user_id,
        amount: tariff.map(|t| t.price as f64).unwrap_or(0.0),
        currency: "RUB".to_string(),
        status: "succeeded".to_string(),
        months: Some(months),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        fulfilled: Some(true),
        fulfilled_at: Some(Utc::now()),
        idempotence_key: None,
        provider: Some("yookassa".to_string()),
    };
    let _ = payment_svc.db.save_transaction(&tx).await;
    let _ = cache.release_fulfill_lock(payment_id).await;

    let expiry_str = new_expiry.format("%Y-%m-%d %H:%M:%S UTC").to_string();
    let message = format!(
        "🎉 <b>Спасибо за покупку подписки</b>!\n\n\
         📧 <b>Статус</b>: активна\n\
         📅 <b>Действует до</b>: {}\n\n\
         Если что-то непонятно — воспользуйтесь командой: /help — полезная информация и поддержка 💬",
        expiry_str
    );
    send_html_text(transport, user_id, &message).await?;

    if let Ok(Some(referrer_id)) = payment_svc.db.consume_referral_reward(user_id).await {
        match payment_svc.db.extend_subscription(referrer_id, 1).await {
            Ok(referrer_expiry) => {
                let referrer_expiry_str = referrer_expiry.format("%d.%m.%Y").to_string();
                let referrer_message = format!(
                    "🎁 <b>Бонус по реферальной программе!</b>\n\n\
                     Ваш друг оформил подписку, и вы получили <b>+1 месяц</b> бесплатно!\n\n\
                     📅 Подписка активна до: <b>{}</b>",
                    referrer_expiry_str
                );
                send_html_text(transport, referrer_id, &referrer_message).await?;
                tracing::info!(referrer_id = referrer_id, invited_id = user_id, "referral reward granted: +1 month");
            }
            Err(e) => {
                tracing::warn!(referrer_id = referrer_id, invited_id = user_id, error = %e, "failed to grant referral reward");
            }
        }
    }

    tracing::info!(payment_id = %payment_id, user_id = user_id, months = months, "payment fulfilled");

    Ok(())
}

async fn update_state(
    store: Option<&DialogueStore>,
    dialogue: Option<&AppDialogue>,
    user_id: i64,
    state: AppState,
) -> HandlerResult {
    if let Some(store) = store {
        store.update(user_id, state);
    } else if let Some(dialogue) = dialogue {
        dialogue.update(state).await?;
    }

    Ok(())
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
