//! Инициализация и запуск приложения.
//!
//! Этот модуль отвечает за:
//! - Подключение к MongoDB
//! - Инициализацию платёжного сервиса YooKassa
//! - Запуск HTTP сервера для webhooks
//! - Запуск фонового планировщика напоминаний
//! - Запуск Telegram бот dispatcher

use std::sync::Arc;

use axum::Router;
use teloxide::dispatching::UpdateHandler;
use teloxide::dispatching::dialogue::InMemStorage;
use teloxide::prelude::*;
use tracing::info;

use crate::api::db::Db;
use crate::api::payments::PaymentService;
use crate::bot;
use crate::bot::states::AppState;
use crate::config::Config;
use crate::scheduler;

/// Запускает приложение и все его компоненты.
///
/// ## Порядок инициализации
///
/// ```text
/// 1. Config::from_env()     ─── Загрузка конфигурации
/// 2. Bot::from_env()        ─── Создание Telegram бота
/// 3. Db::connect()          ─── Подключение к MongoDB
/// 4. PaymentService         ─── Инициализация YooKassa
/// 5. Axum HTTP server       ─── Фоновый сервер для webhooks
/// 6. Scheduler              ─── Фоновый планировщик напоминаний
/// 7. Telegram Dispatcher    ─── Основной цикл обработки сообщений
/// ```
///
/// ## Dependency Injection
///
/// Зависимости передаются в handlers через `dptree::deps!`:
/// - `Config` - конфигурация приложения
/// - `InMemStorage<AppState>` - хранилище состояний диалогов
/// - `Db` - подключение к MongoDB
/// - `PaymentService` - сервис платежей
///
/// # Errors
///
/// Возвращает ошибку при:
/// - Невалидной конфигурации
/// - Ошибке подключения к MongoDB
/// - Ошибке запуска HTTP сервера
pub async fn run() -> anyhow::Result<()> {
    info!("Starting yanapomnyu_bot...");

    // === 1. Загрузка конфигурации ===
    // Читает переменные окружения: MONGO_URI, IP, PORT, etc.
    let config = Config::from_env();
    
    // Создаёт Telegram бота из TELOXIDE_TOKEN
    let bot = Bot::from_env();

    // Устанавливаем команды меню бота
    set_bot_commands(&bot).await;

    // === 2. Подключение к MongoDB ===
    info!("Connecting to MongoDB...");
    let db = Db::connect(&config.mongo_uri, None).await?;
    info!("MongoDB connected");

    // === 3. Инициализация платёжного сервиса ===
    // PaymentService работает с YooKassa API и Redis кэшем
    info!("Initializing PaymentService...");
    let payment_svc = Arc::new(PaymentService::from_env(db.clone())?);
    info!("PaymentService initialized");

    // === 4. HTTP сервер для webhooks YooKassa ===
    // Axum router обрабатывает POST /yookassa/webhook
    let webhook_router: Router = payment_svc.clone().router(bot.clone());

    // Парсим адрес для bind (IP:PORT из конфигурации)
    let addr: std::net::SocketAddr = format!("{}:{}", config.ip, config.port).parse()?;

    info!(%addr, "Starting HTTP server for YooKassa webhooks");

    // Запускаем HTTP сервер в фоновом task
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, webhook_router).await {
            tracing::error!(error = %e, "axum server failed");
        }
    });

    // === 5. Хранилище состояний диалогов ===
    // InMemStorage хранит AppState для каждого chat_id
    // При рестарте бота состояния теряются (можно заменить на Redis)
    let storage = InMemStorage::<AppState>::new();

    // === 6. Фоновый планировщик напоминаний ===
    // Каждые 10 секунд проверяет due напоминания и отправляет
    info!("Starting reminder scheduler...");
    scheduler::start_scheduler(bot.clone(), db.clone());

    // === 6.1 Планировщик проверки подписок ===
    // Каждый час проверяет подписки:
    // - Отправляет предупреждения за 7 дней до истечения
    // - Удаляет напоминания при истечении подписки
    info!("Starting subscription scheduler...");
    scheduler::start_subscription_scheduler(bot.clone(), db.clone());

    // === 6.2 Планировщик проверки каналов Twitch/YouTube ===
    // Каждые 5 минут проверяет подписанные каналы на новые стримы/видео
    info!("Starting channel scheduler...");
    scheduler::start_channel_scheduler(bot.clone(), db.clone());

    // === 7. Telegram Dispatcher ===
    info!("Starting Telegram bot dispatcher...");

    // Схема роутинга: Commands → Text → Callbacks
    let schema: UpdateHandler<_> = bot::router::schema();

    // Создаём и запускаем dispatcher
    // deps! - инъекция зависимостей в handlers
    Dispatcher::builder(bot, schema)
        .dependencies(dptree::deps![config, storage, db, payment_svc])
        .enable_ctrlc_handler()  // Корректное завершение по Ctrl+C
        .build()
        .dispatch()  // Блокирующий цикл обработки updates
        .await;

    Ok(())
}

/// Устанавливает список команд в меню бота.
async fn set_bot_commands(bot: &Bot) {
    use teloxide::types::BotCommand;

    let commands = vec![
        BotCommand::new("start", "Начать"),
        BotCommand::new("help", "Дополнительная информация"),
        BotCommand::new("utc", "Настройка часового пояса"),
        BotCommand::new("profile", "Профиль и подписка"),
        BotCommand::new("pay", "Оплата"),
        BotCommand::new("setup", "Настройки"),
        BotCommand::new("list", "Список активных напоминаний"),
        BotCommand::new("yan", "ИИ-помощник Yan"),
        BotCommand::new("subs", "Уведомления о новых видео"),
        BotCommand::new("ref", "Реферальная ссылка"),
    ];

    if let Err(e) = bot.set_my_commands(commands).await {
        tracing::warn!(error = %e, "Failed to set bot commands");
    } else {
        info!("Bot commands menu updated");
    }
}
