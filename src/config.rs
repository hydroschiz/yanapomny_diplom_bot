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
//! | `BOT_USERNAME` | Username бота в Telegram | yanapomnyu_bot |

use serde::Deserialize;
use std::env;

/// Конфигурация приложения.
///
/// Содержит все настройки, необходимые для работы бота:
/// подключения к базам данных, параметры HTTP сервера и т.д.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// Список Telegram ID администраторов бота.
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
    
    /// Username бота в Telegram (без @).
    pub bot_username: String,
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

        // MongoDB URI с полными credentials для production
        let mongo_uri = env::var("MONGO_URI").unwrap_or_else(|_| {
            "mongodb://tgBotUser:tgBotPassword@mongodb1:27017/tgBot?authSource=tgBot".to_string()
        });

        // Redis для кэширования pending платежей
        let redis_url = env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1/".to_string());
        
        // Адрес HTTP сервера
        let ip = env::var("IP").unwrap_or_else(|_| "0.0.0.0".to_string());
        let port = env::var("PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(3001);
        
        // Username бота (для генерации ссылок t.me/username)
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
