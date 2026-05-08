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

const UTC_KEYBOARD_PAGE_SIZE: usize = 7;

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
/// Количество страниц UTC-клавиатуры.
pub fn utc_keyboard_page_count() -> usize {
    OFFSETS.len().div_ceil(UTC_KEYBOARD_PAGE_SIZE)
}

/// Генерирует первую страницу клавиатуры выбора UTC смещения.
pub fn utc_keyboard() -> TransportKeyboard {
    utc_keyboard_page(0)
}

/// Генерирует страницу клавиатуры выбора UTC смещения.
pub fn utc_keyboard_page(page: usize) -> TransportKeyboard {
    let mut rows: Vec<Vec<TransportButton>> = Vec::new();
    let page_count = utc_keyboard_page_count();
    let page = page % page_count;
    let start = page * UTC_KEYBOARD_PAGE_SIZE;
    let end = (start + UTC_KEYBOARD_PAGE_SIZE).min(OFFSETS.len());

    // VK inline-клавиатура принимает не больше 10 кнопок суммарно.
    for chunk in OFFSETS[start..end].chunks(4) {
        let row = chunk
            .iter()
            .map(|o| {
                let label = format!("UTC{}", o);
                TransportButton::callback(label, format!("utc_set:{}", o))
            })
            .collect();
        rows.push(row);
    }

    let previous_page = if page == 0 { page_count - 1 } else { page - 1 };
    let next_page = (page + 1) % page_count;
    rows.push(vec![
        TransportButton::callback("⬅", format!("utc_page:{}", previous_page)),
        TransportButton::callback("Назад", "utc_cancel"),
        TransportButton::callback("➡", format!("utc_page:{}", next_page)),
    ]);

    TransportKeyboard::new(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_keyboard_fits_vk_inline_limits() {
        for page in 0..utc_keyboard_page_count() {
            let keyboard = utc_keyboard_page(page);

            assert!(keyboard.rows.len() <= 6);
            assert!(keyboard.rows.iter().all(|row| row.len() <= 5));
            assert!(keyboard.rows.iter().map(Vec::len).sum::<usize>() <= 10);
        }
    }

    #[test]
    fn utc_keyboard_pages_cover_all_offsets() {
        let mut offsets = Vec::new();

        for page in 0..utc_keyboard_page_count() {
            for row in utc_keyboard_page(page).rows {
                for button in row {
                    if let TransportButton::Callback { data, .. } = button {
                        if let Some(offset) = data.strip_prefix("utc_set:") {
                            offsets.push(offset.to_string());
                        }
                    }
                }
            }
        }

        assert_eq!(offsets, OFFSETS);
    }
}
