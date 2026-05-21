use serde_json::{json, Value};
use transport_core::{TransportButton, TransportCapabilities, TransportKeyboard};
use vk_bot_api::keyboard::{ButtonAction, ButtonColor, Keyboard as VkApiKeyboard, KeyboardButton};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VkKeyboard {
    pub inline: bool,
    pub buttons: Vec<Vec<VkButton>>,
}

impl VkKeyboard {
    pub fn to_json(&self) -> Value {
        json!({
            "inline": self.inline,
            "buttons": self.buttons.iter().map(|row| {
                row.iter().map(VkButton::to_json).collect::<Vec<_>>()
            }).collect::<Vec<_>>()
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VkButton {
    Callback {
        label: String,
        payload: String,
        color: VkButtonColor,
    },
    OpenLink {
        label: String,
        link: String,
    },
}

impl VkButton {
    pub fn to_json(&self) -> Value {
        match self {
            Self::Callback {
                label,
                payload,
                color,
            } => json!({
                "action": {
                    "type": "callback",
                    "label": label,
                    "payload": { "action": payload }
                },
                "color": color.as_vk_str()
            }),
            Self::OpenLink { label, link } => json!({
                "action": {
                    "type": "open_link",
                    "label": label,
                    "link": link
                }
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VkButtonColor {
    Primary,
    Secondary,
    Positive,
    Negative,
}

impl VkButtonColor {
    const fn as_vk_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Secondary => "secondary",
            Self::Positive => "positive",
            Self::Negative => "negative",
        }
    }
}

pub fn vk_inline_capabilities() -> TransportCapabilities {
    TransportCapabilities::vk_inline()
}

pub fn sanitize_keyboard(keyboard: &TransportKeyboard) -> TransportKeyboard {
    let capabilities = vk_inline_capabilities();
    let max_rows = capabilities.max_keyboard_rows.unwrap_or(usize::MAX);
    let max_per_row = capabilities.max_buttons_per_row.unwrap_or(usize::MAX);
    let max_total = capabilities.max_buttons_total.unwrap_or(usize::MAX);

    let mut total = 0;
    let mut rows = Vec::new();

    for row in keyboard
        .rows
        .iter()
        .filter(|row| !row.is_empty())
        .take(max_rows)
    {
        if total >= max_total {
            break;
        }

        let remaining = max_total - total;
        let buttons = row
            .iter()
            .take(max_per_row.min(remaining))
            .cloned()
            .collect::<Vec<_>>();

        if !buttons.is_empty() {
            total += buttons.len();
            rows.push(buttons);
        }
    }

    TransportKeyboard::new(rows)
}

pub fn convert_keyboard(keyboard: &TransportKeyboard) -> VkKeyboard {
    let keyboard = sanitize_keyboard(keyboard);
    let buttons = keyboard
        .rows
        .iter()
        .map(|row| row.iter().map(convert_button).collect())
        .collect();

    VkKeyboard {
        inline: true,
        buttons,
    }
}

pub fn convert_keyboard_to_vk_api(keyboard: &TransportKeyboard) -> VkApiKeyboard {
    let keyboard = sanitize_keyboard(keyboard);
    let mut vk_keyboard = VkApiKeyboard::new_inline();

    for row in &keyboard.rows {
        let vk_row = row.iter().map(convert_button_to_vk_api).collect::<Vec<_>>();
        if !vk_row.is_empty() {
            vk_keyboard = vk_keyboard.add_row(vk_row);
        }
    }

    vk_keyboard
}

fn convert_button(button: &TransportButton) -> VkButton {
    match button {
        TransportButton::Callback { label, data } => VkButton::Callback {
            label: label.clone(),
            payload: data.clone(),
            color: VkButtonColor::Primary,
        },
        TransportButton::Url { label, url } => VkButton::OpenLink {
            label: label.clone(),
            link: url.clone(),
        },
    }
}

fn convert_button_to_vk_api(button: &TransportButton) -> KeyboardButton {
    match button {
        TransportButton::Callback { label, data } => KeyboardButton {
            action: ButtonAction::Callback {
                label: label.clone(),
                payload: json!({ "action": data }),
            },
            color: Some(ButtonColor::Primary),
        },
        TransportButton::Url { label, url } => KeyboardButton {
            action: ButtonAction::OpenLink {
                link: url.clone(),
                label: label.clone(),
                payload: None,
            },
            color: None,
        },
    }
}
