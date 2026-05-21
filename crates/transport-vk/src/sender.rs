use std::sync::Arc;

use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use transport_core::{BotTransport, TransportKeyboard};
use vk_bot_api::VkApi;

use crate::keyboard::convert_keyboard_to_vk_api;

#[derive(Clone)]
pub struct VkTransport {
    api: Arc<VkApi>,
}

impl std::fmt::Debug for VkTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VkTransport").finish()
    }
}

impl VkTransport {
    pub fn new(access_token: impl Into<String>) -> Result<Self> {
        let token = access_token.into();
        if token.is_empty() {
            bail!("VK access token cannot be empty");
        }

        let api = VkApi::new(&token).map_err(|error| anyhow!("VK API init error: {error:?}"))?;
        Ok(Self { api: Arc::new(api) })
    }

    pub fn from_env() -> Result<Self> {
        let token =
            std::env::var("VK_ACCESS_TOKEN").map_err(|_| anyhow!("VK_ACCESS_TOKEN is not set"))?;
        Self::new(token)
    }

    pub fn api(&self) -> &VkApi {
        &self.api
    }
}

#[async_trait]
impl BotTransport for VkTransport {
    fn capabilities(&self) -> transport_core::TransportCapabilities {
        transport_core::TransportCapabilities::vk_inline()
    }

    async fn send_text(&self, peer_id: i64, text: &str) -> Result<()> {
        self.api
            .messages_send(
                peer_id,
                text,
                None,
                None,
                None,
                None,
                None,
                false,
                false,
                Some(random_id()),
            )
            .await
            .map_err(|error| anyhow!("VK send error: {error:?}"))?;
        Ok(())
    }

    async fn send_with_keyboard(
        &self,
        peer_id: i64,
        text: &str,
        keyboard: &TransportKeyboard,
    ) -> Result<()> {
        let keyboard = convert_keyboard_to_vk_api(keyboard);
        self.api
            .messages_send(
                peer_id,
                text,
                Some(&keyboard),
                None,
                None,
                None,
                None,
                false,
                false,
                Some(random_id()),
            )
            .await
            .map_err(|error| anyhow!("VK send error: {error:?}"))?;
        Ok(())
    }

    async fn answer_callback(
        &self,
        event_id: &str,
        user_id: i64,
        peer_id: i64,
        text: Option<&str>,
    ) -> Result<()> {
        let event_data = text.map(|text| {
            serde_json::json!({
                "type": "show_snackbar",
                "text": text,
            })
            .to_string()
        });

        self.api
            .messages_send_message_event_answer(event_id, user_id, peer_id, event_data.as_deref())
            .await
            .map_err(|error| anyhow!("VK callback answer error: {error:?}"))?;
        Ok(())
    }
}

pub fn random_id() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
