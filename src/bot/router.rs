use teloxide::dispatching::UpdateHandler;
use teloxide::dispatching::dialogue::InMemStorage;
use teloxide::prelude::*;

use crate::api::db::Db;
use crate::bot::{states::AppState, filters};
// use crate::utils::db::Db;
// use crate::api::<your_api_here>::Client;

use super::handlers;
// use super::filters;

pub type AppDialogue = Dialogue<AppState, InMemStorage<AppState>>;
pub type HandlerResult = Result<(), anyhow::Error>;

pub async fn build_deps() -> anyhow::Result<DependencyMap> {
    let config = crate::config::Config::from_env();
    let storage = InMemStorage::<AppState>::new();
    let db = Db::connect(&config.mongo_uri, None).await?;
    // Добавляйте сюда зависимости для инъекций в хэндлеры
    Ok(dptree::deps![config, storage, db])
}

pub fn schema() -> UpdateHandler<anyhow::Error> {
    use teloxide::dispatching::UpdateFilterExt;

        let messages = Update::filter_message()
                .enter_dialogue::<Message, InMemStorage<AppState>, AppState>()
                .branch(handlers::commands::router())
                .branch(handlers::text::router());

        let callbacks = Update::filter_callback_query()
                .filter(filters::private_chat_cq)
                .enter_dialogue::<CallbackQuery, InMemStorage<AppState>, AppState>()
                .branch(dptree::endpoint(handlers::callbacks::handle_callback));

    dptree::entry().branch(messages).branch(callbacks)
}
