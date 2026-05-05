//! Планировщик проверки подписок.
//!
//! Отвечает за:
//! - Отправку предупреждений за 7 дней до окончания подписки
//! - Удаление напоминаний пользователей с истекшими подписками
//!
//! ## Архитектура
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │           Subscription Scheduler Loop (1 hour)              │
//! ├─────────────────────────────────────────────────────────────┤
//! │  1. check_expiring_subscriptions()                          │
//! │     ├─ Найти пользователей с подпиской, истекающей за 7 дн  │
//! │     ├─ Отправить предупреждение                             │
//! │     └─ Пометить как предупреждённых                         │
//! │                                                             │
//! │  2. process_expired_subscriptions()                         │
//! │     ├─ Найти пользователей с истёкшей подпиской             │
//! │     ├─ Удалить все их напоминания                           │
//! │     ├─ Отправить уведомление об удалении                    │
//! │     └─ Пометить как обработанных                            │
//! └─────────────────────────────────────────────────────────────┘
//! ```

use std::time::Duration;

use tracing::{debug, error, info, warn};

use crate::api::db::Db;
use crate::transport::text_format::strip_html;
use crate::transport::traits::BotTransport;

// ============================================================================
// Конфигурация
// ============================================================================

/// Интервал проверки подписок (1 час).
const SUBSCRIPTION_CHECK_INTERVAL_SECS: u64 = 3600;

/// За сколько дней до истечения отправлять предупреждение.
const EXPIRY_WARNING_DAYS: i32 = 7;

// ============================================================================
// Публичный API
// ============================================================================

/// Запускает планировщик проверки подписок как фоновую задачу.
///
/// Проверяет подписки каждый час:
/// - Отправляет предупреждения за 7 дней до истечения
/// - Удаляет напоминания при истечении подписки
pub fn start_subscription_scheduler<T>(transport: T, db: Db)
where
    T: BotTransport,
{
    tokio::spawn(async move {
        info!("Starting subscription scheduler");
        subscription_loop(transport, db).await;
    });
}

// ============================================================================
// Основной цикл
// ============================================================================

async fn subscription_loop<T>(transport: T, db: Db)
where
    T: BotTransport,
{
    let mut interval = tokio::time::interval(Duration::from_secs(SUBSCRIPTION_CHECK_INTERVAL_SECS));

    loop {
        interval.tick().await;

        // 1. Проверяем подписки, истекающие через 7 дней
        if let Err(e) = check_expiring_subscriptions(&transport, &db).await {
            error!("Error checking expiring subscriptions: {}", e);
        }

        // 2. Обрабатываем истёкшие подписки
        if let Err(e) = process_expired_subscriptions(&transport, &db).await {
            error!("Error processing expired subscriptions: {}", e);
        }
    }
}

// ============================================================================
// Проверка истекающих подписок
// ============================================================================

/// Находит пользователей с истекающими подписками и отправляет предупреждения.
async fn check_expiring_subscriptions(
    transport: &impl BotTransport,
    db: &Db,
) -> anyhow::Result<()> {
    let expiring_users = db
        .get_users_with_expiring_subscriptions(EXPIRY_WARNING_DAYS)
        .await?;

    if expiring_users.is_empty() {
        debug!("No users with expiring subscriptions");
        return Ok(());
    }

    info!(
        "Found {} users with expiring subscriptions",
        expiring_users.len()
    );

    for record in expiring_users {
        // Подсчитываем количество напоминаний у пользователя
        let reminder_count = db.count_user_reminders(record.chat_id).await.unwrap_or(0);

        // Формируем сообщение
        let warning_message = format_expiry_warning(&record.expiry_formatted(), reminder_count);

        // Отправляем предупреждение
        let warning_message = strip_html(&warning_message);

        match transport.send_text(record.chat_id, &warning_message).await {
            Ok(()) => {
                info!("Sent expiry warning to user {}", record.chat_id);
                // Помечаем, что предупреждение отправлено
                if let Err(e) = db.mark_subscription_warning_sent(record.chat_id).await {
                    warn!(
                        "Failed to mark warning sent for user {}: {}",
                        record.chat_id, e
                    );
                }
            }
            Err(e) => {
                // Если пользователь заблокировал бота - всё равно помечаем
                if is_user_blocked_error(&e.to_string()) {
                    warn!(
                        "User {} blocked bot, marking warning as sent",
                        record.chat_id
                    );
                    let _ = db.mark_subscription_warning_sent(record.chat_id).await;
                } else {
                    warn!(
                        "Failed to send expiry warning to user {}: {}",
                        record.chat_id, e
                    );
                }
            }
        }
    }

    Ok(())
}

// ============================================================================
// Обработка истёкших подписок
// ============================================================================

/// Находит пользователей с истёкшими подписками и удаляет их напоминания.
async fn process_expired_subscriptions(
    transport: &impl BotTransport,
    db: &Db,
) -> anyhow::Result<()> {
    let expired_users = db.get_expired_subscriptions().await?;

    if expired_users.is_empty() {
        debug!("No users with expired subscriptions");
        return Ok(());
    }

    info!(
        "Found {} users with expired subscriptions",
        expired_users.len()
    );

    for record in expired_users {
        // Подсчитываем количество напоминаний перед удалением
        let reminder_count = db.count_user_reminders(record.chat_id).await.unwrap_or(0);

        if reminder_count == 0 {
            // Нет напоминаний - просто помечаем как обработанного
            let _ = db.delete_all_user_reminders(record.chat_id).await;
            continue;
        }

        // Удаляем все напоминания
        let deleted_count = db.delete_all_user_reminders(record.chat_id).await?;

        info!(
            "Deleted {} reminders for user {} (subscription expired)",
            deleted_count, record.chat_id
        );

        // Отправляем уведомление об удалении
        let deletion_message = format_deletion_notice(deleted_count);

        let deletion_message = strip_html(&deletion_message);

        match transport.send_text(record.chat_id, &deletion_message).await {
            Ok(()) => {
                info!("Sent deletion notice to user {}", record.chat_id);
            }
            Err(e) => {
                // Не критично если не удалось отправить уведомление
                warn!(
                    "Failed to send deletion notice to user {}: {}",
                    record.chat_id, e
                );
            }
        }
    }

    Ok(())
}

// ============================================================================
// Вспомогательные функции
// ============================================================================

/// Форматирует сообщение-предупреждение об истечении подписки.
fn format_expiry_warning(expiry_date: &str, reminder_count: i64) -> String {
    let reminder_text = if reminder_count == 0 {
        String::new()
    } else {
        let word = pluralize_reminders(reminder_count);
        format!(
            "\n\n📝 У вас <b>{}</b> {}. После истечения подписки все напоминания будут <b>удалены</b>.",
            reminder_count, word
        )
    };

    format!(
        "⚠️ <b>Внимание! Подписка скоро закончится</b>\n\n\
        Ваша подписка истекает <b>{}</b>.{}\n\n\
        Чтобы сохранить доступ к созданию напоминаний и не потерять существующие, \
        продлите подписку командой /pay",
        expiry_date, reminder_text
    )
}

/// Форматирует сообщение об удалении напоминаний.
fn format_deletion_notice(deleted_count: i64) -> String {
    let word = pluralize_reminders(deleted_count);

    format!(
        "❌ <b>Подписка истекла</b>\n\n\
        Ваша подписка закончилась. <b>{}</b> {} было удалено.\n\n\
        Для продолжения использования бота и создания новых напоминаний \
        оформите подписку командой /pay",
        deleted_count, word
    )
}

/// Склонение слова "напоминание" в зависимости от числа.
fn pluralize_reminders(count: i64) -> &'static str {
    let abs = count.abs();
    let last_two = abs % 100;
    let last_one = abs % 10;

    if (11..=19).contains(&last_two) {
        "напоминаний"
    } else if last_one == 1 {
        "напоминание"
    } else if (2..=4).contains(&last_one) {
        "напоминания"
    } else {
        "напоминаний"
    }
}

/// Проверяет, является ли ошибка блокировкой бота пользователем.
fn is_user_blocked_error(error: &str) -> bool {
    error.contains("blocked")
        || error.contains("chat not found")
        || error.contains("user is deactivated")
        || error.contains("bot was kicked")
}
