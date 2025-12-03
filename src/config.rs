use serde::Deserialize;
use std::env;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub admins: Vec<i64>,
    pub mongo_uri: String,
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

        Self { admins, mongo_uri }
    }
}
