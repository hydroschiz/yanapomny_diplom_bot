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

pub async fn run() -> anyhow::Result<()> {
    info!("Starting yanapomnyu_bot...");

    let config = Config::from_env();
    let bot = Bot::from_env();

    info!("Connecting to MongoDB...");
    let db = Db::connect(&config.mongo_uri, None).await?;
    info!("MongoDB connected");

    info!("Initializing PaymentService...");
    let payment_svc = Arc::new(PaymentService::from_env(db.clone())?);
    info!("PaymentService initialized");

    // Build Axum router for YooKassa webhooks
    let webhook_router: Router = payment_svc.clone().router(bot.clone());

    // Bind address for Axum server
    let addr: std::net::SocketAddr = format!("{}:{}", config.ip, config.port).parse()?;

    info!(%addr, "Starting HTTP server for YooKassa webhooks");

    // Spawn Axum server in background
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, webhook_router).await {
            tracing::error!(error = %e, "axum server failed");
        }
    });

    // Build dependencies for bot
    let storage = InMemStorage::<AppState>::new();

    info!("Starting Telegram bot dispatcher...");

    let schema: UpdateHandler<_> = bot::router::schema();

    // Run Telegram bot dispatcher
    Dispatcher::builder(bot, schema)
        .dependencies(dptree::deps![config, storage, db, payment_svc])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}
