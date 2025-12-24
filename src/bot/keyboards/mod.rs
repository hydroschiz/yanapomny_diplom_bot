//! Inline клавиатуры для Telegram бота.
//!
//! Модуль организован по группам клавиатур:
//! - [`common`] - Общие клавиатуры (настройки, навигация, UTC)
//! - [`pay`] - Клавиатуры платежей
//! - [`reminder`] - Клавиатуры напоминаний
//! - [`channels`] - Клавиатуры подписок на каналы

pub mod channels;
pub mod common;
pub mod pay;
pub mod profile;
pub mod reminder;

// Re-exports для удобства использования
pub use channels::{channel_subs_keyboard, stream_notification_keyboard};
pub use common::{back_keyboard, profile_back_keyboard, setup_keyboard, utc_keyboard};
pub use pay::{pay_link_keyboard, pay_menu_keyboard, pay_provider_keyboard};
pub use profile::profile_keyboard;
pub use reminder::{
    delete_keyboard, list_delete_keyboard, reminder_confirm_keyboard, reminder_edit_keyboard,
    reminder_snooze_keyboard, reminder_snoozed_keyboard, snooze_code_to_label,
    snooze_code_to_minutes, text_confirm_keyboard,
};

// Примечание: некоторые функции могут показывать warning "never used"
// пока проект в разработке - это нормально, они будут использованы позже
