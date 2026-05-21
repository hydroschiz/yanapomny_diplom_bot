use application::{ApplicationError, ApplicationResult, StreamPlatformGateway};
use async_trait::async_trait;
use domain::{ChannelSubscription, Platform};
use serde::Deserialize;

const TWITCH_STREAMS_URL: &str = "https://api.twitch.tv/helix/streams";

#[derive(Clone)]
pub struct TwitchGateway {
    client: reqwest::Client,
    client_id: String,
    access_token: String,
}

impl TwitchGateway {
    pub fn new(client_id: impl Into<String>, access_token: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            client_id: client_id.into(),
            access_token: access_token.into(),
        }
    }
}

#[async_trait]
impl StreamPlatformGateway for TwitchGateway {
    async fn latest_content_id(
        &self,
        subscription: &ChannelSubscription,
    ) -> ApplicationResult<Option<String>> {
        if subscription.platform != Platform::Twitch {
            return Ok(None);
        }

        let response = self
            .client
            .get(TWITCH_STREAMS_URL)
            .header("Client-ID", &self.client_id)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .query(&[("user_login", subscription.channel_id.as_str())])
            .send()
            .await
            .map_err(|err| ApplicationError::ExternalService(err.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ApplicationError::ExternalService(format!(
                "Twitch API failed: {status} {body}"
            )));
        }

        let payload: TwitchStreamsResponse = response
            .json()
            .await
            .map_err(|err| ApplicationError::ExternalService(err.to_string()))?;
        Ok(payload.data.into_iter().next().map(|stream| stream.id))
    }
}

#[derive(Debug, Deserialize)]
struct TwitchStreamsResponse {
    data: Vec<TwitchStream>,
}

#[derive(Debug, Deserialize)]
struct TwitchStream {
    id: String,
}
