//! Конфигурация приложения из переменных окружения.
//!
//! Все настройки загружаются из ENV переменных с fallback на значения по умолчанию.
//!
//! ## Переменные окружения
//!
//! | Переменная | Описание | Значение по умолчанию |
//! |------------|----------|----------------------|
//! | `ADMINS` | ID администраторов через запятую | пусто |
//! | `MONGO_URI` | URI подключения к MongoDB | mongodb://...@mongodb1:27017/tgBot |
//! | `REDIS_URL` | URL подключения к Redis | redis://127.0.0.1/ |
//! | `IP` | IP адрес для HTTP сервера | 0.0.0.0 |
//! | `PORT` | Порт HTTP сервера | 3001 |
//! | `VK_ACCESS_TOKEN` | Access token сообщества VK | обязательно |
//! | `VK_GROUP_ID` | ID сообщества VK | обязательно |
//! | `BOT_USERNAME` | Короткое имя бота для упоминаний | yanapomnyu_bot |

use serde::Deserialize;
use std::env;

/// Конфигурация приложения.
///
/// Содержит все настройки, необходимые для работы бота:
/// подключения к базам данных, параметры HTTP сервера и т.д.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Список ID администраторов бота.
    /// Администраторы могут получать уведомления об ошибках.
    pub admins: Vec<i64>,

    /// URI подключения к MongoDB.
    /// Формат: `mongodb://user:password@host:port/database?authSource=...`
    pub mongo_uri: String,

    /// URL подключения к Redis.
    /// Используется для кэширования pending платежей.
    pub redis_url: String,

    /// IP адрес для bind HTTP сервера.
    /// Обычно `0.0.0.0` для приёма соединений со всех интерфейсов.
    pub ip: String,

    /// Порт HTTP сервера для YooKassa webhooks.
    pub port: u16,

    /// VK access token сообщества.
    pub vk_access_token: String,

    /// VK group ID сообщества.
    pub vk_group_id: i64,

    /// Короткое имя бота для упоминаний в групповых сценариях.
    pub bot_username: String,

    /// Включён ли платёжный контур.
    /// Если `false`, reminder-only сценарии должны работать без YooKassa.
    pub payments_enabled: bool,
}

fn parse_bool_env(name: &str) -> Option<bool> {
    let value = env::var(name).ok()?;
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

impl Config {
    /// Загружает конфигурацию из переменных окружения.
    ///
    /// Для отсутствующих переменных используются значения по умолчанию.
    ///
    /// # Пример
    ///
    /// ```ignore
    /// // Установить переменные перед вызовом:
    /// // MONGO_URI=mongodb://localhost:27017/tgBot
    /// // ADMINS=123456789,987654321
    ///
    /// let config = Config::from_env();
    /// println!("MongoDB: {}", config.mongo_uri);
    /// ```
    pub fn from_env() -> Self {
        // Парсим список ID администраторов из строки "123,456,789"
        let admins = env::var("ADMINS")
            .unwrap_or_default()
            .split(",")
            .filter_map(|s| s.trim().parse::<i64>().ok())
            .collect();

        // MongoDB URI - может быть задан напрямую или собран из MONGO_USER/MONGO_PASS
        let mongo_uri = if let Ok(uri) = env::var("MONGO_URI") {
            uri
        } else {
            // Если MONGO_URI не задан, формируем из отдельных переменных
            let user = env::var("MONGO_USER").expect("MONGO_USER must be set (or use MONGO_URI)");
            let pass = env::var("MONGO_PASS").expect("MONGO_PASS must be set (or use MONGO_URI)");
            let host = env::var("MONGO_HOST").unwrap_or_else(|_| "mongodb".to_string());
            let port = env::var("MONGO_PORT").unwrap_or_else(|_| "27017".to_string());
            let db = env::var("MONGO_DB").unwrap_or_else(|_| "tgBot".to_string());
            let auth_source = env::var("MONGO_AUTH_SOURCE").unwrap_or_else(|_| "admin".to_string());

            // URL-encode пароль для безопасной передачи в URI
            let encoded_pass = urlencoding::encode(&pass);

            format!(
                "mongodb://{}:{}@{}:{}/{}?authSource={}",
                user, encoded_pass, host, port, db, auth_source
            )
        };

        // Redis для кэширования pending платежей
        let redis_url = env::var("REDIS_URL").expect("REDIS_URL must be set");

        // Адрес HTTP сервера
        let ip = env::var("IP").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = env::var("PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3001);

        let vk_access_token = env::var("VK_ACCESS_TOKEN").expect("VK_ACCESS_TOKEN must be set");
        let vk_group_id = env::var("VK_GROUP_ID")
            .expect("VK_GROUP_ID must be set")
            .parse::<i64>()
            .expect("VK_GROUP_ID must be an integer");

        // Короткое имя бота для упоминаний в групповых сценариях.
        let bot_username =
            env::var("BOT_USERNAME").unwrap_or_else(|_| "yanapomnyu_bot".to_string());

        let payment_creds_present = env::var("YK_SHOP_ID")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .is_some()
            && env::var("YK_SECRET_KEY")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .is_some();

        let payments_enabled = parse_bool_env("PAYMENTS_ENABLED").unwrap_or(payment_creds_present);

        Self {
            admins,
            mongo_uri,
            redis_url,
            ip,
            port,
            vk_access_token,
            vk_group_id,
            bot_username,
            payments_enabled,
        }
    }
}
