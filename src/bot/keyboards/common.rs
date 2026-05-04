//! Общие клавиатуры: настройки, навигация, выбор UTC.

use crate::transport::traits::{TransportButton, TransportKeyboard};

/// Список доступных UTC смещений для выбора.
pub static OFFSETS: &[&str] = &[
    "-12:00", "-11:00", "-10:00", "-09:30", "-09:00", "-08:00", "-07:00", "-06:00", "-05:00",
    "-04:30", "-04:00", "-03:30", "-03:00", "-02:00", "-01:00", "+00:00", "+01:00", "+02:00",
    "+03:00", "+03:30", "+04:00", "+04:30", "+05:00", "+05:30", "+05:45", "+06:00", "+06:30",
    "+07:00", "+08:00", "+08:30", "+08:45", "+09:00", "+09:30", "+10:00", "+10:30", "+11:00",
    "+12:00",
];

/// Клавиатура главного меню настроек (/setup).
///
/// Кнопки:
/// - Время откладывания -> `setup_snooze`
/// - Авто откладывание -> `setup_auto`
/// - Время суток (UTC) -> `setup_utc`
/// - Профиль -> `profile`
pub fn setup_keyboard() -> TransportKeyboard {
    TransportKeyboard::new(vec![
        vec![TransportButton::callback("Время откладывания", "setup_snooze")],
        vec![TransportButton::callback("Авто откладывание", "setup_auto")],
        vec![TransportButton::callback("Время суток (UTC)", "setup_utc")],
        vec![TransportButton::callback("👤 Профиль", "profile")],
    ])
}

/// Клавиатура кнопки "Назад" для возврата в меню настроек.
pub fn back_keyboard() -> TransportKeyboard {
    TransportKeyboard::new(vec![vec![TransportButton::callback("⬅ Назад", "setup_menu")]])
}

/// Навигационная клавиатура только с кнопкой "Профиль".
/// Для разделов, откуда "Назад" ведёт в профиль.
pub fn profile_back_keyboard() -> TransportKeyboard {
    TransportKeyboard::new(vec![vec![
        TransportButton::callback("👤 Профиль", "profile"),
    ]])
}

/// Клавиатура выбора UTC смещения.
///
/// Генерирует сетку кнопок с UTC смещениями (по 4 в ряд)
/// и кнопку "Назад" для отмены.
pub fn utc_keyboard() -> TransportKeyboard {
    let mut rows: Vec<Vec<TransportButton>> = Vec::new();

    // Генерируем кнопки UTC смещений (по 4 в ряд)
    for chunk in OFFSETS.chunks(4) {
        let row = chunk
            .iter()
            .map(|o| {
                let label = format!("UTC{}", o);
                TransportButton::callback(label, format!("utc_set:{}", o))
            })
            .collect();
        rows.push(row);
    }

    // Кнопка отмены
    rows.push(vec![TransportButton::callback("⬅ Назад", "utc_cancel")]);

    TransportKeyboard::new(rows)
}
