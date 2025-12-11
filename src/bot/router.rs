use std::sync::Arc;

use teloxide::dispatching::UpdateHandler;
use teloxide::dispatching::dialogue::InMemStorage;
use teloxide::prelude::*;

use crate::api::db::Db;
use crate::api::payments::PaymentService;
use crate::bot::{states::AppState, filters};

use super::handlers;

pub type AppDialogue = Dialogue<AppState, InMemStorage<AppState>>;
pub type HandlerResult = Result<(), anyhow::Error>;

pub async fn build_deps() -> anyhow::Result<DependencyMap> {
    let config = crate::config::Config::from_env();
    let storage = InMemStorage::<AppState>::new();
    let db = Db::connect(&config.mongo_uri, None).await?;

    // Initialize PaymentService if YK_SHOP_ID is set
    let payment_svc: Arc<PaymentService> = Arc::new(
        PaymentService::from_env(db.clone())
            .expect("Failed to initialize PaymentService. Check YK_SHOP_ID and YK_SECRET_KEY env vars.")
    );

    Ok(dptree::deps![config, storage, db, payment_svc])
}

pub fn schema() -> UpdateHandler<anyhow::Error> {
    use teloxide::dispatching::UpdateFilterExt;

    let messages = Update::filter_message()
        .enter_dialogue::<Message, InMemStorage<AppState>, AppState>()
        .branch(handlers::commands::router())
        .branch(handlers::text::router())
        // Handle reminder editing state
        .branch(
            dptree::case![AppState::AwaitingReminderEdit { pending }]
                .filter(filters::private_chat_msg)
                .endpoint(handlers::reminder::handle_reminder_edit_text),
        )
        // Handle reminder deletion state
        .branch(
            dptree::case![AppState::AwaitingReminderDeletion]
                .filter(filters::private_chat_msg)
                .endpoint(handlers::reminder::handle_deletion_input),
        )
        // Handle idle state - any text treated as reminder (lowest priority)
        .branch(
            dptree::case![AppState::Idle]
                .filter(filters::private_chat_msg)
                .endpoint(handlers::reminder::handle_idle_text),
        );

    let callbacks = Update::filter_callback_query()
        .filter(filters::private_chat_cq)
        .enter_dialogue::<CallbackQuery, InMemStorage<AppState>, AppState>()
        .branch(dptree::endpoint(handlers::callbacks::handle_callback));

    dptree::entry().branch(messages).branch(callbacks)
}
