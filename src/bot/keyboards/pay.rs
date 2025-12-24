//! Клавиатуры для платежей и подписок.

use teloxide::types::{InlineKeyboardButton as Btn, InlineKeyboardMarkup};

/// Главное меню оплаты с выбором тарифа.
///
/// Callback data:
/// - `pay_select:3` / `pay_select:6` / `pay_select:12` - выбор тарифа
/// - `pay_cancel` - отмена
pub fn pay_menu_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![Btn::callback("3 месяца за 195₽ (65₽/мес)", "pay_select:3")],
        vec![Btn::callback("6 месяцев за 360₽ (60₽/мес)", "pay_select:6")],
        vec![Btn::callback("12 месяцев за 660₽ (55₽/мес)", "pay_select:12")],
        vec![Btn::url(
            "О сервисе",
            "https://t.me/yanapomnyu".parse().unwrap(),
        )],
        vec![Btn::callback("👤 Профиль", "profile")],
    ])
}

/// Клавиатура выбора платёжного провайдера.
///
/// # Аргументы
/// * `months` - количество месяцев тарифа для callback data
pub fn pay_provider_keyboard(months: i32) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![Btn::callback(
            "💳 Карта, SberPay, ЮMoney",
            format!("pay_yk:{}", months),
        )],
        vec![Btn::callback("⬅ Назад", "pay_menu"), Btn::callback("👤 Профиль", "profile")],
    ])
}

/// Клавиатура со ссылкой на оплату.
///
/// # Аргументы
/// * `url` - URL для перехода к оплате
/// * `months` - количество месяцев для проверки статуса
pub fn pay_link_keyboard(url: &str, months: i32) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![Btn::url("💳 Карта, SberPay, ЮMoney", url.parse().unwrap())],
        vec![Btn::callback(
            "🔄 Проверить оплату",
            format!("pay_check:{}", months),
        )],
        vec![Btn::callback("⬅ Назад", "pay_menu"), Btn::callback("👤 Профиль", "profile")],
    ])
}
