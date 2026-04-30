//! Планировщики для работы бота.
//!
//! Включает:
//! - Планировщик отправки напоминаний
//! - Планировщик проверки подписок
//!
//! ## Архитектура планировщика напоминаний
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                   Scheduler Loop (10 sec)                   │
//! ├─────────────────────────────────────────────────────────────┤
//! │  1. claim_due_reminders(100)                                │
//! │     ├─ MongoDB findOneAndUpdate (атомарно)                  │
//! │     └─ status: active/retry → processing                    │
//! │                                                             │
//! │  2. buffer_unordered(20) + Semaphore(20)                   │
//! │     ├─ Параллельная отправка с ограничением                 │
//! │     └─ Rate limiting для Telegram API                       │
//! │                                                             │
//! │  3. Обработка результата:                                   │
//! │     ├─ OK → mark_sent / update_time (recurring)            │
//! │     ├─ Temp error → schedule_retry (exponential backoff)   │
//! │     └─ Permanent error → mark_sent (user blocked)          │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Архитектура планировщика подписок
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │           Subscription Scheduler Loop (1 hour)              │
//! ├─────────────────────────────────────────────────────────────┤
//! │  1. check_expiring_subscriptions()                          │
//! │     ├─ Найти пользователей с подпиской, истекающей за 7 дн  │
//! │     └─ Отправить предупреждение                             │
//! │                                                             │
//! │  2. process_expired_subscriptions()                         │
//! │     ├─ Найти пользователей с истёкшей подпиской             │
//! │     └─ Удалить все их напоминания                           │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Статусы напоминаний
//!
//! | Статус | Описание |
//! |--------|----------|
//! | `active` | Ожидает отправки |
//! | `processing` | Взято в обработку (атомарный захват) |
//! | `retry` | Ожидает retry после временной ошибки |
//! | `sent` | Успешно отправлено |
//! | `failed` | Превышено max retries |
//!
//! ## Retry стратегия
//!
//! Exponential backoff: 30s → 60s → 120s → failed
//!
//! ## Особенности
//!
//! - **Атомарный захват** предотвращает дублирование при overlapping циклов
//! - **Semaphore** ограничивает параллельные запросы к Telegram API
//! - **Recovery** восстанавливает "зависшие" напоминания при старте

pub mod channels;
pub mod subscription;

// Re-export for convenient use
pub use channels::start_channel_scheduler;
pub use subscription::start_subscription_scheduler;

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Datelike, Timelike, Utc};
use futures::stream::{self, StreamExt};
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tokio::sync::Semaphore;
use tracing::{debug, error, info, warn};

use crate::api::db::{Db, Reminder, User};
use crate::utils::timezone::user_local_time;

// ============================================================================
// Конфигурация планировщика
// ============================================================================

/// Интервал между циклами проверки (секунды).
/// Определяет максимальную задержку отправки напоминания.
const SCHEDULER_INTERVAL_SECS: u64 = 10;

/// Максимальное количество напоминаний за один цикл.
/// Ограничивает использование памяти.
const BATCH_SIZE: i64 = 100;

/// Максимальное количество параллельных отправок.
/// Учитывает rate limits Telegram API (~30 msg/sec).
const MAX_CONCURRENT_SENDS: usize = 20;

/// Максимальное количество попыток отправки.
/// После превышения напоминание помечается как `failed`.
const MAX_RETRIES: i32 = 3;

// ============================================================================
// Публичный API
// ============================================================================

/// Запускает планировщик напоминаний как фоновую задачу.
///
/// Планировщик работает бесконечно, пока не завершится tokio runtime.
///
/// ## Порядок работы
///
/// 1. Восстановление "зависших" напоминаний (status = processing)
/// 2. Запуск основного цикла [`scheduler_loop`]
///
/// # Аргументы
///
/// * `bot` - Telegram Bot для отправки сообщений
/// * `db` - Подключение к MongoDB
pub fn start_scheduler(bot: Bot, db: Db) {
    tokio::spawn(async move {
        info!("Starting reminder scheduler");
        
        // Восстанавливаем напоминания, которые были в processing при крахе
        // Это может произойти если бот упал во время отправки
        if let Ok(recovered) = db.recover_stuck_reminders(300).await {
            if recovered > 0 {
                info!("Recovered {} stuck reminders", recovered);
            }
        }
        
        // Запускаем бесконечный цикл обработки
        scheduler_loop(bot, db).await;
    });
}

// ============================================================================
// Основной цикл
// ============================================================================

/// Основной цикл планировщика.
///
/// Каждые [`SCHEDULER_INTERVAL_SECS`] секунд:
/// 1. Захватывает batch напоминаний
/// 2. Отправляет их параллельно
/// 3. Обновляет статусы в базе
async fn scheduler_loop(bot: Bot, db: Db) {
    // interval.tick() срабатывает сразу, затем каждые N секунд
    let mut interval = tokio::time::interval(Duration::from_secs(SCHEDULER_INTERVAL_SECS));
    
    // Semaphore ограничивает количество одновременных отправок
    let semaphore = Arc::new(Semaphore::new(MAX_CONCURRENT_SENDS));

    loop {
        // Ждём следующего тика (или выполняем сразу при первом вызове)
        interval.tick().await;
        
        // Обрабатываем due напоминания
        if let Err(e) = process_due_reminders(&bot, &db, semaphore.clone()).await {
            error!("Scheduler error: {}", e);
        }
    }
}

/// Обрабатывает due напоминания с атомарным захватом и параллельной отправкой.
///
/// ## Алгоритм
///
/// 1. `claim_due_reminders` - атомарно забирает batch напоминаний
/// 2. `buffer_unordered` - параллельно отправляет с ограничением concurrency
/// 3. Логирует результаты
async fn process_due_reminders(
    bot: &Bot, 
    db: &Db, 
    semaphore: Arc<Semaphore>
) -> anyhow::Result<()> {
    // Атомарно захватываем batch напоминаний (status → processing)
    let claimed_reminders = db.claim_due_reminders(BATCH_SIZE).await?;
    
    // Если нет due напоминаний - выходим
    if claimed_reminders.is_empty() {
        return Ok(());
    }

    info!("Processing {} reminders", claimed_reminders.len());

    // Обрабатываем напоминания параллельно с ограничением concurrency
    // buffer_unordered позволяет обрабатывать результаты по мере готовности
    let results: Vec<_> = stream::iter(claimed_reminders)
        .map(|reminder| {
            // Клонируем для перемещения в async block
            let bot = bot.clone();
            let db = db.clone();
            let sem = semaphore.clone();
            
            async move {
                // Получаем permit от semaphore (ждём если достигнут лимит)
                let _permit = sem.acquire().await.unwrap();
                // Отправляем напоминание
                send_reminder(&bot, &db, reminder).await
            }
        })
        .buffer_unordered(MAX_CONCURRENT_SENDS)  // Параллельное выполнение
        .collect()
        .await;

    // Подсчитываем результаты для логирования
    let success_count = results.iter().filter(|r| r.is_ok()).count();
    let error_count = results.iter().filter(|r| r.is_err()).count();
    
    if error_count > 0 {
        warn!("Sent {}/{} reminders ({} errors)", 
              success_count, 
              success_count + error_count,
              error_count);
    } else if success_count > 0 {
        debug!("Sent {} reminders successfully", success_count);
    }

    Ok(())
}

/// Test-oriented entry point for deterministic due reminder processing.
pub async fn process_due_reminders_once(bot: &Bot, db: &Db) -> anyhow::Result<()> {
    process_due_reminders(bot, db, Arc::new(Semaphore::new(MAX_CONCURRENT_SENDS))).await
}

// ============================================================================
// Отправка напоминания
// ============================================================================

/// Отправляет одно напоминание с логикой retry.
///
/// ## Результаты отправки
///
/// | Результат | Действие |
/// |-----------|----------|
/// | OK | Обновляем время (recurring) или помечаем sent |
/// | Permanent error | Помечаем sent (user blocked bot) |
/// | Temp error, retry < max | Планируем retry |
/// | Temp error, retry >= max | Помечаем failed |
async fn send_reminder(bot: &Bot, db: &Db, reminder: Reminder) -> anyhow::Result<()> {
    use crate::bot::keyboards::reminder_snooze_keyboard;

    let chat_id = ChatId(reminder.chat_id);
    let rem_id = reminder.rem_id.unwrap_or(0);

    // Получаем настройки пользователя для кнопок откладывания
    let user = db.ensure_user(reminder.chat_id).await?;
    
    // Форматируем время в часовом поясе пользователя
    let time_display = format_reminder_time_for_user(&reminder.time, &user);

    // Формируем сообщение в новом формате
    let message = format!(
        "▹ {}\n\n\
         📅 <b>{}</b>\n\n\
         💬 Чтобы перенести или завершить уведомление, нажми ниже:",
        html_escape(&reminder.text),
        time_display
    );

    // Создаём клавиатуру с кнопками откладывания
    let keyboard = reminder_snooze_keyboard(rem_id, &user.snooze_buttons);

    // Отправляем через Telegram API
    match bot.send_message(chat_id, &message)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await 
    {
        Ok(_sent_msg) => {
            debug!("Sent reminder #{} to user {}", rem_id, reminder.chat_id);
            // Обрабатываем успешную отправку
            handle_successful_send(db, &reminder).await?;
        }
        Err(e) => {
            let error_str = e.to_string();
            
            // Проверяем, является ли ошибка постоянной (retry бессмысленен)
            if is_permanent_error(&error_str) {
                warn!("Permanent failure for reminder #{}: {}", rem_id, error_str);
                // Помечаем как failed чтобы не пытаться снова и сохранить статус ошибки
                db.mark_reminder_failed(rem_id).await?;
            } 
            // Проверяем, можно ли ещё делать retry
            else if reminder.retry_count < MAX_RETRIES {
                warn!("Temporary failure for reminder #{}, scheduling retry: {}", 
                      rem_id, error_str);
                // Планируем retry с exponential backoff
                db.schedule_retry(rem_id, reminder.retry_count).await?;
            } 
            // Превышено максимальное количество retry
            else {
                error!("Max retries exceeded for reminder #{}, marking as failed", rem_id);
                db.mark_reminder_failed(rem_id).await?;
            }
        }
    }

    Ok(())
}

/// Форматирует время напоминания для отображения.
/// Формат: "21.10 (вторник, 13:31)"
fn format_reminder_time(time: &DateTime<Utc>, utc_offset: &str) -> String {
    use chrono::FixedOffset;

    // Парсим смещение пользователя
    let offset_secs = parse_utc_offset(utc_offset).unwrap_or(0);
    let offset = FixedOffset::east_opt(offset_secs).unwrap_or(FixedOffset::east_opt(0).unwrap());
    
    // Конвертируем в локальное время пользователя
    let local_time = time.with_timezone(&offset);
    
    // Названия дней недели на русском
    let weekday = match local_time.weekday() {
        chrono::Weekday::Mon => "понедельник",
        chrono::Weekday::Tue => "вторник",
        chrono::Weekday::Wed => "среда",
        chrono::Weekday::Thu => "четверг",
        chrono::Weekday::Fri => "пятница",
        chrono::Weekday::Sat => "суббота",
        chrono::Weekday::Sun => "воскресенье",
    };
    
    format!(
        "{:02}.{:02} ({}, {:02}:{:02})",
        local_time.day(),
        local_time.month(),
        weekday,
        local_time.hour(),
        local_time.minute()
    )
}

pub fn format_reminder_time_for_user(time: &DateTime<Utc>, user: &User) -> String {
    let local_time = user_local_time(user, *time);

    let weekday = match local_time.weekday() {
        chrono::Weekday::Mon => "понедельник",
        chrono::Weekday::Tue => "вторник",
        chrono::Weekday::Wed => "среда",
        chrono::Weekday::Thu => "четверг",
        chrono::Weekday::Fri => "пятница",
        chrono::Weekday::Sat => "суббота",
        chrono::Weekday::Sun => "воскресенье",
    };

    format!(
        "{:02}.{:02} ({}, {:02}:{:02})",
        local_time.day(),
        local_time.month(),
        weekday,
        local_time.hour(),
        local_time.minute()
    )
}

/// Форматирует полную дату для сообщения об откладывании.
/// Формат: "ОКТЯБРЬ 2025г. 21.10 (вторник, 16:42)"
pub fn format_full_reminder_time(time: &DateTime<Utc>, utc_offset: &str) -> String {
    use chrono::FixedOffset;

    let offset_secs = parse_utc_offset(utc_offset).unwrap_or(0);
    let offset = FixedOffset::east_opt(offset_secs).unwrap_or(FixedOffset::east_opt(0).unwrap());
    let local_time = time.with_timezone(&offset);
    
    let month_name = match local_time.month() {
        1 => "ЯНВАРЬ",
        2 => "ФЕВРАЛЬ",
        3 => "МАРТ",
        4 => "АПРЕЛЬ",
        5 => "МАЙ",
        6 => "ИЮНЬ",
        7 => "ИЮЛЬ",
        8 => "АВГУСТ",
        9 => "СЕНТЯБРЬ",
        10 => "ОКТЯБРЬ",
        11 => "НОЯБРЬ",
        12 => "ДЕКАБРЬ",
        _ => "???",
    };
    
    let weekday = match local_time.weekday() {
        chrono::Weekday::Mon => "понедельник",
        chrono::Weekday::Tue => "вторник",
        chrono::Weekday::Wed => "среда",
        chrono::Weekday::Thu => "четверг",
        chrono::Weekday::Fri => "пятница",
        chrono::Weekday::Sat => "суббота",
        chrono::Weekday::Sun => "воскресенье",
    };
    
    format!(
        "{} {}г. <b>{:02}.{:02} ({}, {:02}:{:02})</b>",
        month_name,
        local_time.year(),
        local_time.day(),
        local_time.month(),
        weekday,
        local_time.hour(),
        local_time.minute()
    )
}

pub fn format_full_reminder_time_for_user(time: &DateTime<Utc>, user: &User) -> String {
    let local_time = user_local_time(user, *time);

    let month_name = match local_time.month() {
        1 => "ЯНВАРЬ",
        2 => "ФЕВРАЛЬ",
        3 => "МАРТ",
        4 => "АПРЕЛЬ",
        5 => "МАЙ",
        6 => "ИЮНЬ",
        7 => "ИЮЛЬ",
        8 => "АВГУСТ",
        9 => "СЕНТЯБРЬ",
        10 => "ОКТЯБРЬ",
        11 => "НОЯБРЬ",
        12 => "ДЕКАБРЬ",
        _ => "???",
    };

    let weekday = match local_time.weekday() {
        chrono::Weekday::Mon => "понедельник",
        chrono::Weekday::Tue => "вторник",
        chrono::Weekday::Wed => "среда",
        chrono::Weekday::Thu => "четверг",
        chrono::Weekday::Fri => "пятница",
        chrono::Weekday::Sat => "суббота",
        chrono::Weekday::Sun => "воскресенье",
    };

    format!(
        "{} {}г. <b>{:02}.{:02} ({}, {:02}:{:02})</b>",
        month_name,
        local_time.year(),
        local_time.day(),
        local_time.month(),
        weekday,
        local_time.hour(),
        local_time.minute()
    )
}

/// Парсит строку UTC offset в секунды.
fn parse_utc_offset(utc_str: &str) -> Option<i32> {
    if utc_str == "nil" || utc_str.is_empty() {
        return Some(0);
    }
    
    // Format: "+3:00" or "-5:30" or "7" or "+7"
    let s = utc_str.trim();
    
    let (sign, rest) = if s.starts_with('-') {
        (-1, &s[1..])
    } else if s.starts_with('+') {
        (1, &s[1..])
    } else {
        (1, s)
    };
    
    let parts: Vec<&str> = rest.split(':').collect();
    let hours: i32 = parts.first()?.parse().ok()?;
    let minutes: i32 = parts.get(1).and_then(|m| m.parse().ok()).unwrap_or(0);
    
    Some(sign * (hours * 3600 + minutes * 60))
}

/// Обрабатывает успешную отправку напоминания.
///
/// Для повторяющихся напоминаний - вычисляет следующее время.
/// Для разовых - помечает как отправленное.
async fn handle_successful_send(db: &Db, reminder: &Reminder) -> anyhow::Result<()> {
    let rem_id = reminder.rem_id.unwrap_or(0);
    
    // Проверяем, является ли напоминание повторяющимся
    if !reminder.delay.is_empty() {
        // Повторяющееся напоминание - вычисляем следующее время
        if let Some(next_time) = calculate_next_from_delay(&reminder.delay, reminder.time) {
            debug!("Updating reminder #{} next time to {}", rem_id, next_time);
            // Обновляем время и сбрасываем статус на active
            db.update_reminder_time(rem_id, next_time).await?;
        } else {
            // Не удалось вычислить следующее время - помечаем как sent
            db.mark_reminder_sent(rem_id).await?;
        }
    } else {
        // Разовое напоминание - просто помечаем как отправленное
        db.mark_reminder_sent(rem_id).await?;
    }
    
    Ok(())
}

// ============================================================================
// Вспомогательные функции
// ============================================================================

/// Проверяет, является ли ошибка постоянной (нет смысла в retry).
///
/// Возвращает `true` для ошибок типа:
/// - Пользователь заблокировал бота
/// - Чат не найден
/// - Пользователь деактивирован
/// - Бот был кикнут из группы
fn is_permanent_error(error: &str) -> bool {
    error.contains("blocked") || 
    error.contains("chat not found") ||
    error.contains("user is deactivated") ||
    error.contains("bot was kicked") ||
    error.contains("have no rights")
}

/// Вычисляет следующее время для повторяющегося напоминания.
///
/// Поддерживает legacy форматы delay из старой версии бота:
/// - `day` - каждый день
/// - `week` - каждую неделю
/// - `month` - каждый месяц
/// - `year` - каждый год
/// - `weekday` - по будням (пн-пт)
/// - `weekend` - по выходным (сб-вс)
///
/// # Возвращает
///
/// `Some(DateTime)` - следующее время срабатывания
/// `None` - неизвестный формат delay
fn calculate_next_from_delay(
    delay: &str,
    current_time: chrono::DateTime<Utc>,
) -> Option<chrono::DateTime<Utc>> {
    use chrono::Duration;

    let next = match delay {
        // Простые интервалы
        "day" => current_time + Duration::days(1),
        "week" => current_time + Duration::weeks(1),
        "month" => add_months(current_time, 1),
        "year" => add_months(current_time, 12),
        
        // По будням - ищем следующий рабочий день
        "weekday" => {
            let mut next = current_time + Duration::days(1);
            while !is_weekday(next.weekday()) {
                next = next + Duration::days(1);
            }
            next
        }
        
        // По выходным - ищем следующий выходной
        "weekend" => {
            let mut next = current_time + Duration::days(1);
            while !is_weekend(next.weekday()) {
                next = next + Duration::days(1);
            }
            next
        }
        
        // Неизвестный формат
        _ => return None,
    };

    Some(next)
}

/// Добавляет месяцы к дате с корректной обработкой переполнения дней.
///
/// Например: 31 января + 1 месяц = 28/29 февраля
fn add_months(dt: chrono::DateTime<Utc>, months: i32) -> chrono::DateTime<Utc> {
    use chrono::{Datelike, NaiveDate};

    let date = dt.date_naive();
    let mut year = date.year();
    let mut month = date.month() as i32 + months;

    // Нормализуем месяц (может стать > 12 или < 1)
    while month > 12 {
        month -= 12;
        year += 1;
    }
    while month < 1 {
        month += 12;
        year -= 1;
    }

    // Ограничиваем день максимумом для нового месяца
    let day = date.day().min(days_in_month(year, month as u32));
    let new_date = NaiveDate::from_ymd_opt(year, month as u32, day).unwrap_or(date);
    let new_dt = new_date.and_time(dt.time());

    chrono::DateTime::<Utc>::from_naive_utc_and_offset(new_dt, Utc)
}

/// Возвращает количество дней в месяце.
fn days_in_month(year: i32, month: u32) -> u32 {
    use chrono::NaiveDate;
    
    // Вычисляем как разницу между первыми днями текущего и следующего месяца
    if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    }
    .unwrap()
    .signed_duration_since(NaiveDate::from_ymd_opt(year, month, 1).unwrap())
    .num_days() as u32
}

/// Проверяет, является ли день будним (понедельник-пятница).
fn is_weekday(wd: chrono::Weekday) -> bool {
    matches!(
        wd,
        chrono::Weekday::Mon
            | chrono::Weekday::Tue
            | chrono::Weekday::Wed
            | chrono::Weekday::Thu
            | chrono::Weekday::Fri
    )
}

/// Проверяет, является ли день выходным (суббота-воскресенье).
fn is_weekend(wd: chrono::Weekday) -> bool {
    matches!(wd, chrono::Weekday::Sat | chrono::Weekday::Sun)
}

/// Экранирует специальные символы для HTML.
///
/// Заменяет: `&` → `&amp;`, `<` → `&lt;`, `>` → `&gt;`
pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
