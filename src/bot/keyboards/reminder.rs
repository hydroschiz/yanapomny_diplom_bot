//! Клавиатуры для создания и управления напоминаниями.

use teloxide::types::{InlineKeyboardButton as Btn, InlineKeyboardMarkup};

/// Клавиатура подтверждения текста ПЕРЕД отправкой в LLM.
///
/// Callback data:
/// - `text_confirm` - подтвердить создание
/// - `text_cancel` - отменить
pub fn text_confirm_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        Btn::callback("✅ Да, создать", "text_confirm"),
        Btn::callback("❌ Отмена", "text_cancel"),
    ]])
}

/// Клавиатура подтверждения распарсенного напоминания.
///
/// Callback data:
/// - `reminder_confirm` - создать напоминание
/// - `reminder_edit` - изменить текст
/// - `reminder_cancel` - отменить
pub fn reminder_confirm_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            Btn::callback("✅ Создать", "reminder_confirm"),
            Btn::callback("✏️ Изменить", "reminder_edit"),
        ],
        vec![Btn::callback("❌ Отменить", "reminder_cancel")],
    ])
}

/// Клавиатура при редактировании текста напоминания.
///
/// Callback data:
/// - `reminder_cancel` - отменить редактирование
pub fn reminder_edit_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![Btn::callback(
        "❌ Отменить",
        "reminder_cancel",
    )]])
}

/// Клавиатура для списка напоминаний (/list).
///
/// Callback data:
/// - `reminder_delete_start` - начать удаление
pub fn list_delete_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![Btn::callback(
        "🗑 Удалить напоминание",
        "reminder_delete_start",
    )]])
}

/// Клавиатура в режиме удаления напоминания.
///
/// Callback data:
/// - `reminder_delete_back` - вернуться к списку
pub fn delete_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![Btn::callback(
        "⬅️ Назад",
        "reminder_delete_back",
    )]])
}
