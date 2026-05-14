use transport_core::{TransportButton, TransportKeyboard};
use transport_vk::{convert_keyboard, sanitize_keyboard, vk_inline_capabilities};

#[test]
fn vk_capabilities_match_known_inline_limits() {
    let capabilities = vk_inline_capabilities();

    assert!(!capabilities.supports_html);
    assert_eq!(capabilities.max_keyboard_rows, Some(10));
    assert_eq!(capabilities.max_buttons_total, Some(10));
    assert!(!capabilities.url_buttons_support_color);
}

#[test]
fn sanitize_keyboard_enforces_total_button_limit() {
    let rows = (0..12)
        .map(|idx| {
            vec![TransportButton::callback(
                format!("B{idx}"),
                idx.to_string(),
            )]
        })
        .collect();
    let keyboard = TransportKeyboard::new(rows);

    let sanitized = sanitize_keyboard(&keyboard);

    assert_eq!(sanitized.row_count(), 10);
    assert_eq!(sanitized.button_count(), 10);
    assert!(sanitized.fits(vk_inline_capabilities()));
}

#[test]
fn convert_keyboard_maps_callback_payload() {
    let keyboard = TransportKeyboard::new(vec![vec![TransportButton::callback("Да", "confirm")]]);

    let json = convert_keyboard(&keyboard).to_json();

    assert_eq!(json["inline"], true);
    assert_eq!(json["buttons"][0][0]["action"]["type"], "callback");
    assert_eq!(
        json["buttons"][0][0]["action"]["payload"]["action"],
        "confirm"
    );
    assert_eq!(json["buttons"][0][0]["color"], "primary");
}

#[test]
fn convert_keyboard_does_not_add_color_to_url_buttons() {
    let keyboard = TransportKeyboard::new(vec![vec![TransportButton::url(
        "Открыть",
        "https://example.com",
    )]]);

    let json = convert_keyboard(&keyboard).to_json();

    assert_eq!(json["buttons"][0][0]["action"]["type"], "open_link");
    assert_eq!(
        json["buttons"][0][0]["action"]["link"],
        "https://example.com"
    );
    assert!(json["buttons"][0][0].get("color").is_none());
}
