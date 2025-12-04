use serde::Deserialize;
use std::env;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub admins: Vec<i64>,
    pub mongo_uri: String,
    pub redis_url: String,
    pub ip: String,
    pub port: u16,
    pub bot_username: String,
}

impl Config {
    pub fn from_env() -> Self {
        let admins = env::var("ADMINS")
            .unwrap_or_default()
            .split(",")
            .filter_map(|s| s.trim().parse::<i64>().ok())
            .collect();

        let mongo_uri = env::var("MONGO_URI").unwrap_or_else(|_| {
            "mongodb://tgBotUser:tgBotPassword@mongodb1:27017/tgBot?authSource=tgBot".to_string()
        });

        let redis_url = env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".to_string());
        let ip = env::var("IP").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = env::var("PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3001);
        let bot_username =
            env::var("BOT_USERNAME").unwrap_or_else(|_| "yanapomnyu_bot".to_string());

        Self {
            admins,
            mongo_uri,
            redis_url,
            ip,
            port,
            bot_username,
        }
    }
}
