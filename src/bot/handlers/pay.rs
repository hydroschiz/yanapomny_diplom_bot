//! Payment handlers for /pay command and payment callbacks.

use std::sync::Arc;

use async_trait::async_trait;

use crate::api::db::Db;
use crate::api::payments::{get_tariff, PaymentService};
use crate::bot::keyboards::{pay_link_keyboard, pay_menu_keyboard, pay_provider_keyboard};
use crate::bot::router::HandlerResult;
use crate::bot::states::AppState;
use crate::transport::dialogue_store::DialogueStore;
use crate::transport::text_format::strip_html;
use crate::transport::traits::{BotTransport, TransportKeyboard};

#[async_trait]
trait PayStateStore {
    async fn update_state(&self, user_id: i64, state: AppState) -> HandlerResult;
}

#[async_trait]
impl PayStateStore for DialogueStore {
    async fn update_state(&self, user_id: i64, state: AppState) -> HandlerResult {
        self.update(user_id, state);
        Ok(())
    }
}

/// Format subscription status message.
pub fn format_subscription_status(is_active: bool, expiry: Option<&str>) -> String {
    let status = if is_active {
        "активна ✅"
    } else {
        "неактивна ❌"
    };
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
        store,
        db,
        payment_svc,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn handle_pay_callback_core<T, S>(
    transport: &T,
    event_id: &str,
    user_id: i64,
    peer_id: i64,
    data: &str,
    store: &S,
    db: Db,
    payment_svc: Arc<PaymentService>,
) -> HandlerResult
where
    T: BotTransport,
    S: PayStateStore + Sync,
{
    if !payment_svc.is_enabled() && data != "pay_cancel" {
        update_state(store, user_id, AppState::Idle).await?;
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
        update_state(store, user_id, AppState::Idle).await?;
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
        update_state(store, user_id, AppState::Idle).await?;

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

            update_state(store, user_id, AppState::AwaitingPayment { months }).await?;
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

                    update_state(store, user_id, AppState::AwaitingPayment { months }).await?;
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
                .answer_callback(
                    event_id,
                    user_id,
                    peer_id,
                    Some("Проверяем статус платежа..."),
                )
                .await?;

            match payment_svc.get_pending_payment(user_id).await {
                Ok(Some(pending)) => {
                    match payment_svc
                        .manual_check(transport, user_id, &pending.payment_id)
                        .await
                    {
                        Ok(msg) => {
                            let is_success = msg.contains('✅');
                            transport.send_text(peer_id, &msg).await?;
                            if is_success {
                                update_state(store, user_id, AppState::Idle).await?;
                            }
                        }
                        Err(e) => {
                            tracing::error!(%e, "payment check failed");
                            transport
                                .send_text(
                                    peer_id,
                                    "❌ Не удалось проверить платёж. Попробуйте позже.",
                                )
                                .await?;
                        }
                    }
                }
                Ok(None) => {
                    transport
                        .send_text(
                            peer_id,
                            "Нет активных платежей для проверки. Оформите новый платёж.",
                        )
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

async fn update_state<S: PayStateStore + Sync>(
    store: &S,
    user_id: i64,
    state: AppState,
) -> HandlerResult {
    store.update_state(user_id, state).await
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
