use transport_core::{TransportButton, TransportCapabilities, TransportKeyboard};

pub static OFFSETS: &[&str] = &[
    "-12:00", "-11:00", "-10:00", "-09:30", "-09:00", "-08:00", "-07:00", "-06:00", "-05:00",
    "-04:30", "-04:00", "-03:30", "-03:00", "-02:00", "-01:00", "+00:00", "+01:00", "+02:00",
    "+03:00", "+03:30", "+04:00", "+04:30", "+05:00", "+05:30", "+05:45", "+06:00", "+06:30",
    "+07:00", "+08:00", "+08:30", "+08:45", "+09:00", "+09:30", "+10:00", "+10:30", "+11:00",
    "+12:00",
];

#[derive(Debug, Clone, Copy)]
pub struct KeyboardBuilder {
    capabilities: TransportCapabilities,
}

impl KeyboardBuilder {
    pub const fn new(capabilities: TransportCapabilities) -> Self {
        Self { capabilities }
    }

    pub fn fit(&self, keyboard: TransportKeyboard) -> TransportKeyboard {
        self.finish(keyboard.rows)
    }

    pub fn finish(&self, rows: Vec<Vec<TransportButton>>) -> TransportKeyboard {
        if !self.capabilities.supports_inline_keyboard {
            return TransportKeyboard::empty();
        }

        let max_rows = self.capabilities.max_keyboard_rows.unwrap_or(usize::MAX);
        let max_per_row = self.capabilities.max_buttons_per_row.unwrap_or(usize::MAX);
        let max_total = self.capabilities.max_buttons_total.unwrap_or(usize::MAX);
        let mut total = 0;
        let mut fitted_rows = Vec::new();

        for row in rows
            .into_iter()
            .filter(|row| !row.is_empty())
            .take(max_rows)
        {
            if total >= max_total {
                break;
            }

            let remaining = max_total - total;
            let fitted = row
                .into_iter()
                .take(max_per_row.min(remaining))
                .collect::<Vec<_>>();

            if !fitted.is_empty() {
                total += fitted.len();
                fitted_rows.push(fitted);
            }
        }

        TransportKeyboard::new(fitted_rows)
    }
}

pub fn setup_keyboard(capabilities: TransportCapabilities) -> TransportKeyboard {
    KeyboardBuilder::new(capabilities).finish(vec![
        vec![TransportButton::callback(
            "Время откладывания",
            "setup_snooze",
        )],
        vec![TransportButton::callback("Авто откладывание", "setup_auto")],
        vec![TransportButton::callback("Время суток (UTC)", "setup_utc")],
        vec![TransportButton::callback("👤 Профиль", "profile")],
    ])
}

pub fn back_keyboard(capabilities: TransportCapabilities) -> TransportKeyboard {
    KeyboardBuilder::new(capabilities).finish(vec![vec![TransportButton::callback(
        "⬅ Назад",
        "setup_menu",
    )]])
}

pub fn profile_back_keyboard(capabilities: TransportCapabilities) -> TransportKeyboard {
    KeyboardBuilder::new(capabilities).finish(vec![vec![TransportButton::callback(
        "👤 Профиль",
        "profile",
    )]])
}

pub fn utc_keyboard_page_count(capabilities: TransportCapabilities) -> usize {
    OFFSETS.len().div_ceil(utc_page_size(capabilities))
}

pub fn utc_keyboard(capabilities: TransportCapabilities) -> TransportKeyboard {
    utc_keyboard_page(capabilities, 0)
}

pub fn utc_keyboard_page(capabilities: TransportCapabilities, page: usize) -> TransportKeyboard {
    let page_size = utc_page_size(capabilities);
    let page_count = utc_keyboard_page_count(capabilities);
    let page = page % page_count;
    let start = page * page_size;
    let end = (start + page_size).min(OFFSETS.len());
    let per_row = capabilities.max_buttons_per_row.unwrap_or(4).clamp(1, 4);
    let mut rows = Vec::new();

    for chunk in OFFSETS[start..end].chunks(per_row) {
        rows.push(
            chunk
                .iter()
                .map(|offset| {
                    TransportButton::callback(
                        format!("UTC{}", offset),
                        format!("utc_set:{}", offset),
                    )
                })
                .collect(),
        );
    }

    let previous_page = if page == 0 { page_count - 1 } else { page - 1 };
    let next_page = (page + 1) % page_count;
    rows.push(vec![
        TransportButton::callback("⬅", format!("utc_page:{}", previous_page)),
        TransportButton::callback("Назад", "utc_cancel"),
        TransportButton::callback("➡", format!("utc_page:{}", next_page)),
    ]);

    KeyboardBuilder::new(capabilities).finish(rows)
}

fn utc_page_size(capabilities: TransportCapabilities) -> usize {
    capabilities
        .max_buttons_total
        .map(|max| max.saturating_sub(3).max(1))
        .unwrap_or(7)
}

pub fn profile_keyboard(capabilities: TransportCapabilities) -> TransportKeyboard {
    KeyboardBuilder::new(capabilities).finish(vec![
        vec![
            TransportButton::callback("📋 Список напоминаний", "profile_list"),
            TransportButton::callback("⚙️ Настройки", "profile_setup"),
        ],
        vec![
            TransportButton::callback("📺 Уведомления", "profile_subs"),
            TransportButton::callback("👥 Рефералка", "profile_referral"),
        ],
        vec![TransportButton::callback("💎 Подписка", "profile_pay")],
    ])
}

pub fn pay_menu_keyboard(capabilities: TransportCapabilities) -> TransportKeyboard {
    KeyboardBuilder::new(capabilities).finish(vec![
        vec![TransportButton::callback(
            "3 месяца за 195₽ (65₽/мес)",
            "pay_select:3",
        )],
        vec![TransportButton::callback(
            "6 месяцев за 360₽ (60₽/мес)",
            "pay_select:6",
        )],
        vec![TransportButton::callback(
            "12 месяцев за 660₽ (55₽/мес)",
            "pay_select:12",
        )],
        vec![TransportButton::url("О сервисе", "https://t.me/yanapomnyu")],
        vec![TransportButton::callback("👤 Профиль", "profile")],
    ])
}

pub fn pay_provider_keyboard(
    months: i32,
    capabilities: TransportCapabilities,
) -> TransportKeyboard {
    KeyboardBuilder::new(capabilities).finish(vec![
        vec![TransportButton::callback(
            "💳 Карта, SberPay, ЮMoney",
            format!("pay_yk:{}", months),
        )],
        vec![
            TransportButton::callback("⬅ Назад", "pay_menu"),
            TransportButton::callback("👤 Профиль", "profile"),
        ],
    ])
}

pub fn pay_link_keyboard(
    url: &str,
    payment_id: &str,
    capabilities: TransportCapabilities,
) -> TransportKeyboard {
    KeyboardBuilder::new(capabilities).finish(vec![
        vec![TransportButton::url("💳 Карта, SberPay, ЮMoney", url)],
        vec![TransportButton::callback(
            "🔄 Проверить оплату",
            format!("pay_check:{}", payment_id),
        )],
        vec![
            TransportButton::callback("⬅ Назад", "pay_menu"),
            TransportButton::callback("👤 Профиль", "profile"),
        ],
    ])
}

pub fn channel_subs_keyboard(capabilities: TransportCapabilities) -> TransportKeyboard {
    KeyboardBuilder::new(capabilities).finish(vec![
        vec![
            TransportButton::callback("🗑 Удалить", "sub_delete"),
            TransportButton::callback("◀️ Назад", "back_main"),
        ],
        vec![TransportButton::callback("👤 Профиль", "profile")],
    ])
}

pub fn stream_notification_keyboard(capabilities: TransportCapabilities) -> TransportKeyboard {
    KeyboardBuilder::new(capabilities).finish(vec![vec![
        TransportButton::callback("👤 Профиль", "profile"),
        TransportButton::callback("📺 Подписки", "subs"),
    ]])
}

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

pub fn reminder_snooze_keyboard(
    rem_id: i32,
    snooze_buttons: &[String],
    capabilities: TransportCapabilities,
) -> TransportKeyboard {
    let mut buttons = snooze_buttons
        .iter()
        .map(|code| {
            TransportButton::callback(
                snooze_code_to_label(code),
                format!("snooze:{}:{}", rem_id, code),
            )
        })
        .collect::<Vec<_>>();
    let done_button = TransportButton::callback("✅ Готово", format!("reminder_done:{}", rem_id));
    let mut rows = Vec::new();

    if buttons.len() <= 3 {
        rows.push(buttons);
    } else {
        rows.push(buttons.drain(..3).collect());
        if !buttons.is_empty() {
            rows.push(buttons);
        }
    }

    rows.push(vec![done_button]);
    KeyboardBuilder::new(capabilities).finish(rows)
}

pub fn reminder_snoozed_keyboard(
    rem_id: i32,
    capabilities: TransportCapabilities,
) -> TransportKeyboard {
    KeyboardBuilder::new(capabilities).finish(vec![vec![
        TransportButton::callback("📋 Список напоминаний", "reminder_list"),
        TransportButton::callback("✅ Готово", format!("reminder_done:{}", rem_id)),
    ]])
}

pub fn text_confirm_keyboard(capabilities: TransportCapabilities) -> TransportKeyboard {
    KeyboardBuilder::new(capabilities).finish(vec![vec![
        TransportButton::callback("✅ Да, создать", "text_confirm"),
        TransportButton::callback("❌ Отмена", "text_cancel"),
    ]])
}

pub fn reminder_confirm_keyboard(capabilities: TransportCapabilities) -> TransportKeyboard {
    KeyboardBuilder::new(capabilities).finish(vec![
        vec![
            TransportButton::callback("✅ Создать", "reminder_confirm"),
            TransportButton::callback("✏️ Изменить", "reminder_edit"),
        ],
        vec![TransportButton::callback("❌ Отменить", "reminder_cancel")],
    ])
}

pub fn reminder_edit_keyboard(capabilities: TransportCapabilities) -> TransportKeyboard {
    KeyboardBuilder::new(capabilities).finish(vec![vec![TransportButton::callback(
        "❌ Отменить",
        "reminder_cancel",
    )]])
}

pub fn list_delete_keyboard(capabilities: TransportCapabilities) -> TransportKeyboard {
    KeyboardBuilder::new(capabilities).finish(vec![vec![
        TransportButton::callback("🗑 Удалить", "reminder_delete_start"),
        TransportButton::callback("✏️ Изменить", "reminder_edit_start"),
    ]])
}

pub fn delete_keyboard(capabilities: TransportCapabilities) -> TransportKeyboard {
    KeyboardBuilder::new(capabilities).finish(vec![vec![TransportButton::callback(
        "⬅️ Назад",
        "reminder_delete_back",
    )]])
}
