//! Инициализация и запуск VK-приложения.
//!
//! Этот модуль отвечает за:
//! - подключение к MongoDB;
//! - инициализацию платёжного сервиса YooKassa;
//! - запуск HTTP сервера для webhooks в all-in-one режиме;
//! - запуск фоновых планировщиков в all-in-one режиме;
//! - запуск VK long poll бота.

use std::{net::SocketAddr, sync::Arc};

use anyhow::Context;
use axum::Router;
use tokio::net::TcpListener;
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
    let db = connect_db(&config).await?;
    let payment_svc = payment_service(&config, db.clone())?;

    if config.bot_webhook_enabled {
        if payment_svc.is_enabled() {
            spawn_webhook_server(&config, payment_svc.clone(), vk_transport.clone()).await?;
        } else {
            info!("Skipping YooKassa webhook server because payments are disabled");
        }
    } else {
        info!("Skipping embedded YooKassa webhook server because BOT_WEBHOOK_ENABLED=false");
    }

    if config.bot_scheduler_enabled {
        start_all_schedulers(vk_transport.clone(), db.clone());
    } else {
        info!("Skipping embedded schedulers because BOT_SCHEDULER_ENABLED=false");
    }

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
        () = shutdown_signal() => {
        }
    }

    Ok(())
}

/// Запускает standalone scheduler без VK long poll и webhook HTTP сервера.
pub async fn run_scheduler_service() -> anyhow::Result<()> {
    info!("Starting standalone scheduler service...");

    let config = Config::from_env();
    let vk_transport = VkTransport::new(config.vk_access_token.clone())?;
    let db = connect_db(&config).await?;

    start_all_schedulers(vk_transport, db);
    shutdown_signal().await;

    Ok(())
}

/// Запускает standalone YooKassa webhook service без VK long poll и scheduler loops.
pub async fn run_webhook_service() -> anyhow::Result<()> {
    info!("Starting standalone YooKassa webhook service...");

    let config = Config::from_env();
    if !config.payments_enabled {
        anyhow::bail!("PAYMENTS_ENABLED=false; standalone webhook service is disabled");
    }

    let vk_transport = VkTransport::new(config.vk_access_token.clone())?;
    let db = connect_db(&config).await?;
    let payment_svc = Arc::new(PaymentService::from_env(db)?);
    let router = payment_svc.router(vk_transport);
    let (addr, listener) = bind_webhook_listener(&config).await?;

    info!(%addr, "YooKassa webhook service is listening");
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn connect_db(config: &Config) -> anyhow::Result<Db> {
    info!("Connecting to MongoDB...");
    let db = Db::connect(&config.mongo_uri, None).await.map_err(|err| {
        tracing::error!(error = %err, "MongoDB is unavailable during startup");
        err
    })?;
    info!("MongoDB connected");
    Ok(db)
}

fn payment_service(config: &Config, db: Db) -> anyhow::Result<Arc<PaymentService>> {
    if config.payments_enabled {
        info!("Initializing PaymentService...");
        let service = Arc::new(PaymentService::from_env(db)?);
        info!("PaymentService initialized");
        Ok(service)
    } else {
        warn!("PaymentService disabled by configuration; reminder-only mode is active");
        Ok(Arc::new(PaymentService::disabled(db)))
    }
}

async fn spawn_webhook_server(
    config: &Config,
    payment_svc: Arc<PaymentService>,
    vk_transport: VkTransport,
) -> anyhow::Result<()> {
    let webhook_router: Router = payment_svc.router(vk_transport);
    let (addr, listener) = bind_webhook_listener(config).await?;

    info!(%addr, "Starting HTTP server for YooKassa webhooks");
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, webhook_router).await {
            tracing::error!(error = %e, "axum server failed");
        }
    });

    Ok(())
}

async fn bind_webhook_listener(config: &Config) -> anyhow::Result<(SocketAddr, TcpListener)> {
    let addr: SocketAddr = format!("{}:{}", config.ip, config.port)
        .parse()
        .context("invalid webhook bind address")?;
    let listener = TcpListener::bind(addr).await?;
    Ok((addr, listener))
}

fn start_all_schedulers<T>(transport: T, db: Db)
where
    T: crate::transport::traits::BotTransport,
{
    info!("Starting reminder scheduler...");
    scheduler::start_scheduler(transport.clone(), db.clone());

    info!("Starting subscription scheduler...");
    scheduler::start_subscription_scheduler(transport.clone(), db.clone());

    info!("Starting channel scheduler...");
    scheduler::start_channel_scheduler(transport, db);
}

async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        tracing::error!(error = %err, "failed to listen for shutdown signal");
    }
    info!("Shutdown signal received");
}
