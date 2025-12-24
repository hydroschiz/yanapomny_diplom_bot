//! Клавиатуры для создания и управления напоминаниями.

use teloxide::types::{InlineKeyboardButton as Btn, InlineKeyboardMarkup};

/// Конвертирует код снуза в читаемый текст для кнопки.
pub fn snooze_code_to_label(code: &str) -> &'static str {
    match code {
        "5minutSnooze" => "5 мин",
        "10minutSnooze" => "10 мин",
        "15minutSnooze" => "15 мин",
        "20minutSnooze" => "20 мин",
        "30minutSnooze" => "30 мин",
        "1hourSnooze" => "1 час",
        "2hourSnooze" => "2 часа",
        "3hourSnooze" => "3 часа",
        "4hourSnooze" => "4 часа",
        "1daySnooze" => "1 день",
        "2daySnooze" => "2 дня",
        "3daySnooze" => "3 дня",
        "7daySnooze" => "7 дней",
        _ => "?",
    }
}

/// Конвертирует код снуза в минуты.
pub fn snooze_code_to_minutes(code: &str) -> Option<i64> {
    match code {
        "5minutSnooze" => Some(5),
        "10minutSnooze" => Some(10),
        "15minutSnooze" => Some(15),
        "20minutSnooze" => Some(20),
        "30minutSnooze" => Some(30),
        "1hourSnooze" => Some(60),
        "2hourSnooze" => Some(120),
        "3hourSnooze" => Some(180),
        "4hourSnooze" => Some(240),
        "1daySnooze" => Some(1440),
        "2daySnooze" => Some(2880),
        "3daySnooze" => Some(4320),
        "7daySnooze" => Some(10080),
        _ => None,
    }
}

/// Клавиатура для отправленного напоминания с кнопками откладывания.
///
/// Использует индивидуальные настройки пользователя.
///
/// Callback data:
/// - `snooze:{rem_id}:{code}` - отложить на указанный интервал
/// - `reminder_done:{rem_id}` - пометить как выполненное
pub fn reminder_snooze_keyboard(rem_id: i32, snooze_buttons: &[String]) -> InlineKeyboardMarkup {
    let mut buttons: Vec<Btn> = snooze_buttons
        .iter()
        .map(|code| {
            let label = snooze_code_to_label(code);
            let callback = format!("snooze:{}:{}", rem_id, code);
            Btn::callback(label, callback)
        })
        .collect();

    // Добавляем кнопку "Готово"
    let done_btn = Btn::callback("✅ Готово", format!("reminder_done:{}", rem_id));

    // Разбиваем на ряды по 3-4 кнопки
    let mut rows: Vec<Vec<Btn>> = Vec::new();
    
    // Кнопки снуза в один ряд (максимум 3)
    if buttons.len() <= 3 {
        rows.push(buttons);
    } else {
        rows.push(buttons.drain(..3).collect());
        if !buttons.is_empty() {
            rows.push(buttons);
        }
    }
    
    // Кнопка "Готово" в отдельный ряд
    rows.push(vec![done_btn]);

    InlineKeyboardMarkup::new(rows)
}

/// Клавиатура после откладывания напоминания.
///
/// Callback data:
/// - `reminder_list` - показать список напоминаний
/// - `reminder_done:{rem_id}` - пометить как выполненное
pub fn reminder_snoozed_keyboard(rem_id: i32) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        Btn::callback("📋 Список напоминаний", "reminder_list"),
        Btn::callback("✅ Готово", format!("reminder_done:{}", rem_id)),
    ]])
}

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
