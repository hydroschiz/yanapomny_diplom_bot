use chrono::{DateTime, Utc};

use crate::{ChatId, UserId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CommunicationPlatform {
    Vk,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformIdentity {
    pub user_id: UserId,
    pub platform: CommunicationPlatform,
    pub external_id: String,
    pub chat_id: Option<ChatId>,
    pub connected_at: DateTime<Utc>,
}

impl PlatformIdentity {
    pub fn new(
        user_id: UserId,
        platform: CommunicationPlatform,
        external_id: impl Into<String>,
        chat_id: Option<ChatId>,
        connected_at: DateTime<Utc>,
    ) -> Self {
        Self {
            user_id,
            platform,
            external_id: external_id.into(),
            chat_id,
            connected_at,
        }
    }
}
