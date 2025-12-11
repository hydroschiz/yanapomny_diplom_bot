//! Inline клавиатуры для Telegram бота.
//!
//! Модуль организован по группам клавиатур:
//! - [`common`] - Общие клавиатуры (настройки, навигация, UTC)
//! - [`pay`] - Клавиатуры платежей
//! - [`reminder`] - Клавиатуры напоминаний

pub mod common;
pub mod pay;
pub mod reminder;

// Re-exports для удобства использования
pub use common::{back_keyboard, setup_keyboard, utc_keyboard};
pub use pay::{pay_link_keyboard, pay_menu_keyboard, pay_provider_keyboard};
pub use reminder::{
    delete_keyboard, list_delete_keyboard, reminder_confirm_keyboard, reminder_edit_keyboard,
    text_confirm_keyboard,
};

// Примечание: некоторые функции могут показывать warning "never used"
// пока проект в разработке - это нормально, они будут использованы позже
