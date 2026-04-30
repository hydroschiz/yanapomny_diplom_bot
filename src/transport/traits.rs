//! Платформонезависимые трейты и типы транспортного слоя.
//!
//! [`BotTransport`] — главный трейт для отправки сообщений.
//! [`TransportKeyboard`] — абстрактная inline-клавиатура.
//! [`TransportButton`] — кнопка клавиатуры (callback или URL).

use anyhow::Result;
use async_trait::async_trait;

// ============================================================================
// Keyboard types
// ============================================================================

/// Абстрактная кнопка клавиатуры.
///
/// Поддерживает два типа кнопок, общих для Telegram и VK:
/// - `Callback` — кнопка, отправляющая данные обратно боту
/// - `Url` — кнопка, открывающая ссылку
#[derive(Debug, Clone)]
pub enum TransportButton {
    /// Кнопка с callback data (inline keyboard button в Telegram,
    /// callback button в VK).
    Callback {
        /// Текст на кнопке.
        label: String,
        /// Данные, возвращаемые при нажатии (аналог `callback_data` в Telegram).
        data: String,
    },
    /// Кнопка-ссылка (открывает URL).
    Url {
        /// Текст на кнопке.
        label: String,
        /// URL для открытия.
        url: String,
    },
}

impl TransportButton {
    /// Создать callback-кнопку.
    pub fn callback(label: impl Into<String>, data: impl Into<String>) -> Self {
        Self::Callback {
            label: label.into(),
            data: data.into(),
        }
    }

    /// Создать URL-кнопку.
    pub fn url(label: impl Into<String>, url: impl Into<String>) -> Self {
        Self::Url {
            label: label.into(),
            url: url.into(),
        }
    }
}

/// Абстрактная inline-клавиатура.
///
/// Содержит строки кнопок. Каждая строка — `Vec<TransportButton>`.
///
/// # Пример
///
/// ```ignore
/// let kb = TransportKeyboard::new(vec![
///     vec![TransportButton::callback("✅ Да", "confirm")],
///     vec![TransportButton::callback("❌ Нет", "cancel")],
/// ]);
/// ```
#[derive(Debug, Clone, Default)]
pub struct TransportKeyboard {
    /// Строки кнопок (каждый внутренний `Vec` — одна строка).
    pub rows: Vec<Vec<TransportButton>>,
}

impl TransportKeyboard {
    /// Создать клавиатуру из готовых строк.
    pub fn new(rows: Vec<Vec<TransportButton>>) -> Self {
        Self { rows }
    }

    /// Создать пустую клавиатуру.
    pub fn empty() -> Self {
        Self { rows: Vec::new() }
    }

    /// Добавить строку кнопок.
    pub fn add_row(mut self, row: Vec<TransportButton>) -> Self {
        self.rows.push(row);
        self
    }

    /// Проверить, пуста ли клавиатура.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

// ============================================================================
// BotTransport trait
// ============================================================================

/// Абстрактный транспорт для отправки сообщений.
///
/// Все handlers и scheduler используют этот трейт вместо прямого
/// обращения к API конкретной платформы (Telegram / VK).
///
/// Реализации:
/// - Telegram: обёртка над `teloxide::Bot` (текущая)
/// - VK: обёртка над `vk_bot_api::VkApi` (будущая)
#[async_trait]
pub trait BotTransport: Send + Sync + Clone + 'static {
    /// Отправить текстовое сообщение без клавиатуры.
    async fn send_text(&self, peer_id: i64, text: &str) -> Result<()>;

    /// Отправить текстовое сообщение с inline-клавиатурой.
    async fn send_with_keyboard(
        &self,
        peer_id: i64,
        text: &str,
        keyboard: &TransportKeyboard,
    ) -> Result<()>;

    /// Ответить на callback-событие (answer callback query в Telegram,
    /// sendMessageEventAnswer в VK).
    ///
    /// `event_id` — идентификатор callback-события.
    /// `user_id` — ID пользователя, нажавшего кнопку.
    /// `peer_id` — ID чата/диалога.
    /// `text` — опциональный текст уведомления (toast/snackbar).
    async fn answer_callback(
        &self,
        event_id: &str,
        user_id: i64,
        peer_id: i64,
        text: Option<&str>,
    ) -> Result<()>;
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transport_button_callback() {
        let btn = TransportButton::callback("Нажми", "action:1");
        match btn {
            TransportButton::Callback { label, data } => {
                assert_eq!(label, "Нажми");
                assert_eq!(data, "action:1");
            }
            _ => panic!("expected Callback"),
        }
    }

    #[test]
    fn test_transport_button_url() {
        let btn = TransportButton::url("Открыть", "https://example.com");
        match btn {
            TransportButton::Url { label, url } => {
                assert_eq!(label, "Открыть");
                assert_eq!(url, "https://example.com");
            }
            _ => panic!("expected Url"),
        }
    }

    #[test]
    fn test_keyboard_builder() {
        let kb = TransportKeyboard::empty()
            .add_row(vec![TransportButton::callback("A", "a")])
            .add_row(vec![
                TransportButton::callback("B", "b"),
                TransportButton::url("C", "https://c.com"),
            ]);

        assert_eq!(kb.rows.len(), 2);
        assert_eq!(kb.rows[0].len(), 1);
        assert_eq!(kb.rows[1].len(), 2);
        assert!(!kb.is_empty());
    }

    #[test]
    fn test_keyboard_empty() {
        let kb = TransportKeyboard::empty();
        assert!(kb.is_empty());
    }

    #[test]
    fn test_keyboard_new() {
        let kb = TransportKeyboard::new(vec![
            vec![TransportButton::callback("X", "x")],
        ]);
        assert_eq!(kb.rows.len(), 1);
    }
}
