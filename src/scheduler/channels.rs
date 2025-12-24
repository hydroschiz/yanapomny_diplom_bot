//! Планировщик проверки каналов Twitch/YouTube.
//!
//! Периодически проверяет подписанные каналы на наличие новых стримов/видео
//! и отправляет уведомления пользователям.
//!
//! ## Архитектура
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │           Channel Scheduler Loop (5 min)                    │
//! ├─────────────────────────────────────────────────────────────┤
//! │  1. check_twitch_streams()                                  │
//! │     ├─ Получить список уникальных Twitch каналов            │
//! │     ├─ Проверить статус через Twitch API                    │
//! │     └─ Отправить уведомления о новых стримах                │
//! │                                                             │
//! │  2. check_youtube_videos() (TODO)                           │
//! │     ├─ Получить список YouTube каналов                      │
//! │     └─ Проверить новые видео через YouTube API              │
//! └─────────────────────────────────────────────────────────────┘
//! ```

use std::collections::HashMap;
use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tracing::{debug, error, info, warn};

use crate::api::db::{Db, Platform};
use crate::bot::keyboards::stream_notification_keyboard;

// ============================================================================
// Конфигурация
// ============================================================================

/// Интервал проверки каналов (5 минут).
const CHANNEL_CHECK_INTERVAL_SECS: u64 = 300;

/// Twitch API endpoints.
const TWITCH_STREAMS_URL: &str = "https://api.twitch.tv/helix/streams";
const TWITCH_USERS_URL: &str = "https://api.twitch.tv/helix/users";

// ============================================================================
// Twitch API Types
// ============================================================================

#[derive(Debug, Deserialize)]
struct TwitchStreamsResponse {
    data: Vec<TwitchStream>,
}

#[derive(Debug, Deserialize)]
struct TwitchStream {
    id: String,
    user_id: String,
    user_login: String,
    user_name: String,
    game_name: String,
    title: String,
    viewer_count: i64,
}

#[derive(Debug, Deserialize)]
struct TwitchUsersResponse {
    data: Vec<TwitchUser>,
}

#[derive(Debug, Deserialize)]
struct TwitchUser {
    id: String,
    login: String,
    display_name: String,
}

// ============================================================================
// Публичный API
// ============================================================================

/// Запускает планировщик проверки каналов как фоновую задачу.
pub fn start_channel_scheduler(bot: Bot, db: Db) {
    // Check if Twitch credentials are configured
    let twitch_client_id = std::env::var("TWITCH_CLIENT_ID").ok();
    let twitch_token = std::env::var("TWITCH_ACCESS_TOKEN").ok();
    
    if twitch_client_id.is_none() || twitch_token.is_none() {
        warn!("Twitch API credentials not configured (TWITCH_CLIENT_ID, TWITCH_ACCESS_TOKEN). Channel scheduler disabled.");
        return;
    }

    tokio::spawn(async move {
        info!("Starting channel scheduler");
        channel_loop(bot, db).await;
    });
}

// ============================================================================
// Основной цикл
// ============================================================================

async fn channel_loop(bot: Bot, db: Db) {
    let mut interval = tokio::time::interval(Duration::from_secs(CHANNEL_CHECK_INTERVAL_SECS));
    let client = Client::new();

    loop {
        interval.tick().await;

        // Check Twitch streams
        if let Err(e) = check_twitch_streams(&bot, &db, &client).await {
            error!("Error checking Twitch streams: {}", e);
        }

        // TODO: Add YouTube video checking
        // if let Err(e) = check_youtube_videos(&bot, &db, &client).await {
        //     error!("Error checking YouTube videos: {}", e);
        // }
    }
}

// ============================================================================
// Twitch Stream Checking
// ============================================================================

async fn check_twitch_streams(bot: &Bot, db: &Db, client: &Client) -> anyhow::Result<()> {
    let twitch_client_id = match std::env::var("TWITCH_CLIENT_ID") {
        Ok(id) => id,
        Err(_) => return Ok(()),
    };
    let twitch_token = match std::env::var("TWITCH_ACCESS_TOKEN") {
        Ok(token) => token,
        Err(_) => return Ok(()),
    };

    // Get all Twitch subscriptions
    let all_subs = db.get_all_channel_subs().await?;
    let twitch_subs: Vec<_> = all_subs
        .iter()
        .filter(|s| s.platform == Platform::Twitch)
        .collect();

    if twitch_subs.is_empty() {
        debug!("No Twitch subscriptions to check");
        return Ok(());
    }

    // Get unique channel IDs (usernames)
    let unique_channels: Vec<String> = twitch_subs
        .iter()
        .map(|s| s.channel_id.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    debug!("Checking {} Twitch channels", unique_channels.len());

    // Build channel -> was_live map from DB
    let mut was_live: HashMap<String, bool> = HashMap::new();
    for sub in &twitch_subs {
        was_live.entry(sub.channel_id.clone()).or_insert(sub.is_live);
    }

    // Query Twitch API for stream status (batch by 100)
    for chunk in unique_channels.chunks(100) {
        let query_params: Vec<(&str, &str)> = chunk
            .iter()
            .map(|ch| ("user_login", ch.as_str()))
            .collect();

        let response = client
            .get(TWITCH_STREAMS_URL)
            .header("Client-ID", &twitch_client_id)
            .header("Authorization", format!("Bearer {}", twitch_token))
            .query(&query_params)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            warn!("Twitch API error: {} - {}", status, text);
            continue;
        }

        let streams_response: TwitchStreamsResponse = response.json().await?;
        
        // Create a map of currently live channels
        let live_now: HashMap<String, &TwitchStream> = streams_response
            .data
            .iter()
            .map(|s| (s.user_login.to_lowercase(), s))
            .collect();

        // Check each channel in this chunk
        for channel_id in chunk {
            let channel_lower = channel_id.to_lowercase();
            let is_live_now = live_now.contains_key(&channel_lower);
            let was_live_before = *was_live.get(&channel_lower).unwrap_or(&false);

            // Channel just went live
            if is_live_now && !was_live_before {
                if let Some(stream) = live_now.get(&channel_lower) {
                    // Get all subscribers for this channel
                    let subscribers = db.get_channel_subscribers(Platform::Twitch, channel_id).await?;
                    let subscribers_count = subscribers.len();
                    
                    // Send notifications
                    for user_id in subscribers {
                        send_stream_notification(bot, user_id, stream).await;
                    }

                    info!(
                        channel = %stream.user_name,
                        title = %stream.title,
                        viewers = stream.viewer_count,
                        "Twitch stream started, notified {} users",
                        subscribers_count
                    );
                }
            }

            // Update DB with current live status
            db.update_channel_content(
                Platform::Twitch,
                channel_id,
                live_now.get(&channel_lower).map(|s| s.id.clone()),
                is_live_now,
            )
            .await?;
        }
    }

    Ok(())
}

async fn send_stream_notification(bot: &Bot, user_id: i64, stream: &TwitchStream) {
    let message = format!(
        "<b>{}</b> — начал трансляцию 🎮\n\n\
         <b>{}</b> — <b>{}</b>\n\
         https://twitch.tv/{}\n\n\
         🎮 {} • 👁 {}",
        stream.user_name,
        stream.user_name,
        stream.title,
        stream.user_login,
        stream.game_name,
        stream.viewer_count
    );

    let keyboard = stream_notification_keyboard();

    if let Err(e) = bot
        .send_message(ChatId(user_id), message)
        .parse_mode(ParseMode::Html)
        .reply_markup(keyboard)
        .await
    {
        if is_user_blocked_error(&e.to_string()) {
            warn!("User {} blocked bot", user_id);
        } else {
            warn!("Failed to send stream notification to {}: {}", user_id, e);
        }
    }
}

fn is_user_blocked_error(error: &str) -> bool {
    error.contains("blocked")
        || error.contains("chat not found")
        || error.contains("user is deactivated")
        || error.contains("bot was kicked")
}

// ============================================================================
// YouTube Video Checking (TODO)
// ============================================================================

// YouTube API integration will require:
// 1. YouTube Data API v3 key
// 2. Checking channel's uploaded videos playlist
// 3. Comparing with last known video ID
// 4. Sending notifications for new videos

// async fn check_youtube_videos(bot: &Bot, db: &Db, client: &Client) -> anyhow::Result<()> {
//     // TODO: Implement YouTube API integration
//     Ok(())
// }
