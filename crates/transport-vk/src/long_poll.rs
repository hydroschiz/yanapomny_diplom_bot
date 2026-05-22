use anyhow::{anyhow, Result};
use async_trait::async_trait;
use vk_bot_api::{
    api::VkApi,
    bot::VkBot,
    error::{VkError, VkResult},
    handler::MessageHandler,
    models::Event,
};

use crate::{normalize_event, VkIncomingEvent};

#[async_trait]
pub trait VkEventHandler: Clone + Send + Sync + 'static {
    async fn handle_vk_event(&self, event: VkIncomingEvent) -> Result<()>;
}

pub async fn run_long_poll<H>(
    access_token: impl Into<String>,
    group_id: i64,
    handler: H,
) -> Result<()>
where
    H: VkEventHandler,
{
    let mut bot = VkBot::builder()
        .token(access_token)
        .group_id(group_id)
        .build()
        .map_err(|error| anyhow!("VK bot init error: {error:?}"))?;
    bot.add_handler(VkSdkHandler { handler });
    bot.run()
        .await
        .map_err(|error| anyhow!("VK bot run error: {error:?}"))
}

#[derive(Clone)]
struct VkSdkHandler<H> {
    handler: H,
}

#[async_trait]
impl<H> MessageHandler for VkSdkHandler<H>
where
    H: VkEventHandler,
{
    async fn handle(&self, event: &Event, _api: &VkApi) -> VkResult<()> {
        let Some(event) = normalize_event(event) else {
            return Ok(());
        };

        self.handler
            .handle_vk_event(event)
            .await
            .map_err(|error| VkError::Custom(error.to_string()))
    }
}
