//! VK Transport — реализация `BotTransport` для VK API.
//!
//! Использует `vk-bot-api` для взаимодействия с VK Bot API.
//!
//! # Пример
//!
//! ```ignore
//! let transport = VkTransport::new("vk_access_token")?;
//! transport.send_text(user_id, "Привет!").await?;
//! ```

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use vk_bot_api::{
    keyboard::{ButtonColor, Keyboard},
    VkApi,
};

use super::traits::{BotTransport, TransportKeyboard};

/// VK Transport — обёртка над VK API для отправки сообщений.
#[derive(Clone)]
pub struct VkTransport {
    /// API клиент для отправки сообщений.
    api: Arc<VkApi>,
}

impl std::fmt::Debug for VkTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VkTransport").finish()
    }
}

impl VkTransport {
    /// Создать новый VK Transport.
    ///
    /// # Аргументы
    ///
    /// * `access_token` — VK access token бота
    ///
    /// # Ошибки
    ///
    /// Возвращает ошибку, если токен пустой.
    pub fn new(access_token: impl Into<String>) -> Result<Self> {
        let token = access_token.into();
        if token.is_empty() {
            anyhow::bail!("VK access token не может быть пустым");
        }
        let api = VkApi::new(&token)
            .map_err(|e| anyhow::anyhow!("VK API init error: {:?}", e))?;
        Ok(Self {
            api: Arc::new(api),
        })
    }

    /// Создать из переменной окружения `VK_ACCESS_TOKEN`.
    pub fn from_env() -> Result<Self> {
        let token = std::env::var("VK_ACCESS_TOKEN")
            .map_err(|_| anyhow::anyhow!("VK_ACCESS_TOKEN не установлен"))?;
        Self::new(token)
    }

    /// Внутренний API клиент (для advanced операций).
    pub fn api(&self) -> &VkApi {
        &self.api
    }
}

#[async_trait]
impl BotTransport for VkTransport {
    async fn send_text(&self, peer_id: i64, text: &str) -> Result<()> {
        self.api
            .messages_send(
                peer_id,
                text,
                None,  // keyboard
                None,  // attachment
                None,  // sticker_id
                None,  // reply_to
                None,  // forward_messages
                false, // disable_mentions
                false, // dont_parse_links
                Some(get_random_id()),
            )
            .await
            .map_err(|e| anyhow::anyhow!("VK send error: {:?}", e))?;
        Ok(())
    }

    async fn send_with_keyboard(
        &self,
        peer_id: i64,
        text: &str,
        keyboard: &TransportKeyboard,
    ) -> Result<()> {
        // Конвертируем TransportKeyboard в VK Keyboard
        let vk_keyboard = convert_keyboard(keyboard);

        self.api
            .messages_send(
                peer_id,
                text,
                Some(&vk_keyboard),
                None,
                None,
                None,
                None,
                false,
                false,
                Some(get_random_id()),
            )
            .await
            .map_err(|e| anyhow::anyhow!("VK send error: {:?}", e))?;
        Ok(())
    }

    async fn answer_callback(
        &self,
        event_id: &str,
        user_id: i64,
        peer_id: i64,
        text: Option<&str>,
    ) -> Result<()> {
        // VK использует messages.sendMessageEventAnswer для ответов на callback-кнопки
        // text параметр становится event_data в формате JSON для показа toast
        let event_data = text.map(|t| {
            serde_json::json!({
                "type": "show_snackbar",
                "text": t
            }).to_string()
        });

        self.api
            .messages_send_message_event_answer(
                event_id,
                user_id,
                peer_id,
                event_data.as_deref(),
            )
            .await
            .map_err(|e| anyhow::anyhow!("VK callback answer error: {:?}", e))?;
        Ok(())
    }
}

// ============================================================================
// Utility functions
// ============================================================================

/// Генерирует случайный ID для сообщений VK.
/// VK требует unique random_id для каждого запроса.
fn get_random_id() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// Конвертирует абстрактную клавиатуру в VK Keyboard.
fn convert_keyboard(tk: &TransportKeyboard) -> Keyboard {
    let mut keyboard = Keyboard::new_inline();

    for row in &tk.rows {
        let mut vk_row: Vec<vk_bot_api::keyboard::KeyboardButton> = Vec::new();

        for btn in row {
            match btn {
                super::traits::TransportButton::Callback { label, data } => {
                    let payload = serde_json::json!({ "action": data });
                    let button = create_callback_button(label, &payload, ButtonColor::Primary);
                    vk_row.push(button);
                }
                super::traits::TransportButton::Url { label, url } => {
                    let button = create_link_button(label, url, None, ButtonColor::Secondary);
                    vk_row.push(button);
                }
            }
        }

        if !vk_row.is_empty() {
            keyboard = keyboard.add_row(vk_row);
        }
    }

    keyboard
}

/// Создаёт callback-кнопку для VK.
fn create_callback_button(
    label: &str,
    payload: &serde_json::Value,
    color: ButtonColor,
) -> vk_bot_api::keyboard::KeyboardButton {
    use vk_bot_api::keyboard::ButtonAction;

    vk_bot_api::keyboard::KeyboardButton {
        action: ButtonAction::Callback {
            label: label.to_string(),
            payload: payload.clone(),
        },
        color: Some(color),
    }
}

/// Создаёт link-кнопку для VK.
fn create_link_button(
    label: &str,
    url: &str,
    payload: Option<serde_json::Value>,
    _color: ButtonColor,
) -> vk_bot_api::keyboard::KeyboardButton {
    use vk_bot_api::keyboard::ButtonAction;

    vk_bot_api::keyboard::KeyboardButton {
        action: ButtonAction::OpenLink {
            link: url.to_string(),
            label: label.to_string(),
            payload,
        },
        color: None,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::traits::TransportButton;

    #[test]
    fn test_convert_empty_keyboard() {
        let tk = TransportKeyboard::empty();
        let vk_kb = convert_keyboard(&tk);
        // VK Keyboard создаётся успешно
        let json = vk_kb.to_json();
        assert!(json["buttons"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_convert_keyboard_with_callback_buttons() {
        let tk = TransportKeyboard::new(vec![
            vec![
                TransportButton::callback("Да", "confirm"),
                TransportButton::callback("Нет", "cancel"),
            ],
        ]);
        let vk_kb = convert_keyboard(&tk);
        let json = vk_kb.to_json();
        let buttons = json["buttons"].as_array().unwrap();
        assert_eq!(buttons.len(), 1);
        assert_eq!(buttons[0].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_convert_keyboard_with_url_buttons() {
        let tk = TransportKeyboard::new(vec![
            vec![TransportButton::url("Открыть", "https://example.com")],
        ]);
        let vk_kb = convert_keyboard(&tk);
        let json = vk_kb.to_json();
        let buttons = json["buttons"].as_array().unwrap();
        assert_eq!(buttons.len(), 1);
        assert_eq!(buttons[0].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_convert_keyboard_multiple_rows() {
        let tk = TransportKeyboard::new(vec![
            vec![TransportButton::callback("A", "a"), TransportButton::callback("B", "b")],
            vec![TransportButton::callback("C", "c")],
        ]);
        let vk_kb = convert_keyboard(&tk);
        let json = vk_kb.to_json();
        let buttons = json["buttons"].as_array().unwrap();
        assert_eq!(buttons.len(), 2);
        assert_eq!(buttons[0].as_array().unwrap().len(), 2);
        assert_eq!(buttons[1].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_get_random_id() {
        let id1 = get_random_id();
        let id2 = get_random_id();
        // IDs должны быть разными (разные миллисекунды)
        assert!(id2 >= id1);
    }

    #[test]
    fn test_vk_transport_creation() {
        let transport = VkTransport::new("test_token").unwrap();
        assert_eq!(transport.api().token(), "test_token");
    }

    #[test]
    fn test_vk_transport_empty_token_error() {
        let result = VkTransport::new("");
        assert!(result.is_err());
    }
}
