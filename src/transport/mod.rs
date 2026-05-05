//! Абстрактный транспортный слой.
//!
//! Определяет платформонезависимые трейты и типы для отправки сообщений,
//! управления клавиатурами и состоянием диалога. Позволяет переключаться
//! между платформами (Telegram, VK) без изменения бизнес-логики.
//!
//! ## Подмодули
//!
//! - [`traits`] — Трейт `BotTransport` и абстрактная клавиатура `TransportKeyboard`
//! - [`adapters`] — Legacy Telegram адаптеры (feature `telegram-legacy`)
//! - [`text_format`] — Утилиты форматирования текста (удаление HTML и т.д.)
//! - [`dialogue_store`] — Хранилище состояний FSM
//! - [`vk`] — VK API транспорт (реализация `BotTransport`)

#[cfg(feature = "telegram-legacy")]
pub mod adapters;
pub mod dialogue_store;
pub mod text_format;
pub mod traits;
pub mod vk;
