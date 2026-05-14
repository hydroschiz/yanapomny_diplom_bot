use std::fmt;

use chrono::{DateTime, Utc};

use crate::UserId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    Twitch,
    Youtube,
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Twitch => f.write_str("Twitch"),
            Self::Youtube => f.write_str("YouTube"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelSubscription {
    pub user_id: UserId,
    pub platform: Platform,
    pub channel_id: String,
    pub channel_name: String,
    pub url: String,
    pub sub_num: i32,
    pub created_at: DateTime<Utc>,
    pub last_content_id: Option<String>,
    pub is_live: bool,
}

impl ChannelSubscription {
    pub fn new(
        user_id: UserId,
        platform: Platform,
        channel_id: impl Into<String>,
        channel_name: impl Into<String>,
        url: impl Into<String>,
        sub_num: i32,
        created_at: DateTime<Utc>,
    ) -> Self {
        Self {
            user_id,
            platform,
            channel_id: channel_id.into(),
            channel_name: channel_name.into(),
            url: url.into(),
            sub_num,
            created_at,
            last_content_id: None,
            is_live: false,
        }
    }

    pub fn mark_content_seen(&mut self, content_id: impl Into<String>) {
        self.last_content_id = Some(content_id.into());
    }

    pub fn set_live(&mut self, is_live: bool) {
        self.is_live = is_live;
    }
}
