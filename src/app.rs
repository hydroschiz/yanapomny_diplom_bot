use crate::bot;
use teloxide::dispatching::UpdateHandler;
use teloxide::prelude::*;

pub async fn run() -> anyhow::Result<()> {
    let bot = Bot::from_env();
    let schema: UpdateHandler<_> = bot::router::schema();
    let deps = bot::router::build_deps().await?;

    Dispatcher::builder(bot, schema)
        .dependencies(deps)
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}
