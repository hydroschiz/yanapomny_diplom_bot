//! Инициализация и запуск VK-приложения.
//!
//! Этот модуль отвечает за:
//! - подключение к MongoDB;
//! - инициализацию платёжного сервиса YooKassa;
//! - запуск HTTP сервера для webhooks;
//! - запуск фоновых планировщиков;
//! - запуск VK long poll бота.

use std::sync::Arc;

use axum::Router;
use tracing::{info, warn};
use vk_bot_api::bot::VkBot;

use crate::api::db::Db;
use crate::api::payments::PaymentService;
use crate::bot::router::AppHandler;
use crate::config::Config;
use crate::scheduler;
use crate::transport::dialogue_store::DialogueStore;
use crate::transport::vk::VkTransport;

/// Запускает приложение и все его компоненты.
pub async fn run() -> anyhow::Result<()> {
    info!("Starting yanapomnyu_bot on VK...");

    let config = Config::from_env();
    let vk_transport = VkTransport::new(config.vk_access_token.clone())?;

    info!("Connecting to MongoDB...");
    let db = match Db::connect(&config.mongo_uri, None).await {
        Ok(db) => db,
        Err(err) => {
            tracing::error!(error = %err, "MongoDB is unavailable during startup");
            return Err(err);
        }
    };
    info!("MongoDB connected");

    let payment_svc = if config.payments_enabled {
        info!("Initializing PaymentService...");
        let service = Arc::new(PaymentService::from_env(db.clone())?);
        info!("PaymentService initialized");
        service
    } else {
        warn!("PaymentService disabled by configuration; reminder-only mode is active");
        Arc::new(PaymentService::disabled(db.clone()))
    };

    if payment_svc.is_enabled() {
        let webhook_router: Router = payment_svc.clone().router(vk_transport.clone());
        let addr: std::net::SocketAddr = format!("{}:{}", config.ip, config.port).parse()?;

        info!(%addr, "Starting HTTP server for YooKassa webhooks");
        let listener = tokio::net::TcpListener::bind(addr).await?;
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, webhook_router).await {
                tracing::error!(error = %e, "axum server failed");
            }
        });
    } else {
        info!("Skipping YooKassa webhook server because payments are disabled");
    }

    info!("Starting reminder scheduler...");
    scheduler::start_scheduler(vk_transport.clone(), db.clone());

    info!("Starting subscription scheduler...");
    scheduler::start_subscription_scheduler(vk_transport.clone(), db.clone());

    info!("Starting channel scheduler...");
    scheduler::start_channel_scheduler(vk_transport.clone(), db.clone());

    let dialogue_store = DialogueStore::new();
    let handler = AppHandler::new(
        vk_transport,
        db,
        payment_svc,
        dialogue_store,
        config.clone(),
    );

    let mut vk_bot = VkBot::builder()
        .token(config.vk_access_token)
        .group_id(config.vk_group_id)
        .build()?;
    vk_bot.add_handler(handler);

    info!(group_id = config.vk_group_id, "Starting VK bot long poll");
    tokio::select! {
        result = vk_bot.run() => {
            result?;
        }
        signal = tokio::signal::ctrl_c() => {
            signal?;
            info!("Shutdown signal received");
        }
    }

    Ok(())
}
