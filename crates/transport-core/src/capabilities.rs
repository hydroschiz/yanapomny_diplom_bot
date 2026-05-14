#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransportCapabilities {
    pub supports_html: bool,
    pub supports_inline_keyboard: bool,
    pub max_keyboard_rows: Option<usize>,
    pub max_buttons_per_row: Option<usize>,
    pub max_buttons_total: Option<usize>,
    pub url_buttons_support_color: bool,
}

impl TransportCapabilities {
    pub const fn unlimited() -> Self {
        Self {
            supports_html: true,
            supports_inline_keyboard: true,
            max_keyboard_rows: None,
            max_buttons_per_row: None,
            max_buttons_total: None,
            url_buttons_support_color: true,
        }
    }

    pub const fn vk_inline() -> Self {
        Self {
            supports_html: false,
            supports_inline_keyboard: true,
            max_keyboard_rows: Some(10),
            max_buttons_per_row: Some(5),
            max_buttons_total: Some(10),
            url_buttons_support_color: false,
        }
    }
}

impl Default for TransportCapabilities {
    fn default() -> Self {
        Self::unlimited()
    }
}
