use transport_core::{TransportButton, TransportKeyboard};
use transport_vk::{
    callback_payload, convert_keyboard, convert_keyboard_to_vk_api, is_group_peer, normalize_event,
    random_id, sanitize_keyboard, vk_inline_capabilities, VkIncomingEvent, VkTransport,
};
use vk_bot_api::models::{Event, Message, MessageEvent};

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

#[test]
fn sdk_keyboard_conversion_matches_vk_url_button_rules() {
    let keyboard = TransportKeyboard::new(vec![vec![
        TransportButton::callback("Да", "confirm"),
        TransportButton::url("Открыть", "https://example.com"),
    ]]);

    let json = convert_keyboard_to_vk_api(&keyboard).to_json();

    assert_eq!(
        json["buttons"][0][0]["action"]["payload"]["action"],
        "confirm"
    );
    assert_eq!(json["buttons"][0][0]["color"], "primary");
    assert_eq!(json["buttons"][0][1]["action"]["type"], "open_link");
    assert!(json["buttons"][0][1].get("color").is_none());
}

#[test]
fn callback_payload_extracts_action_command_or_first_string() {
    let mut event = message_event();
    event.payload = Some(std::collections::HashMap::from([(
        "action".to_string(),
        serde_json::json!("profile"),
    )]));
    assert_eq!(callback_payload(&event).as_deref(), Some("profile"));

    event.payload = Some(std::collections::HashMap::from([(
        "command".to_string(),
        serde_json::json!("pay_menu"),
    )]));
    assert_eq!(callback_payload(&event).as_deref(), Some("pay_menu"));

    event.payload = Some(std::collections::HashMap::from([(
        "x".to_string(),
        serde_json::json!("fallback"),
    )]));
    assert_eq!(callback_payload(&event).as_deref(), Some("fallback"));
}

#[test]
fn normalize_event_maps_vk_messages_and_callbacks() {
    let message = Message {
        id: 1,
        from_id: 42,
        text: "hello".to_string(),
        peer_id: 2_000_000_001,
        conversation_message_id: None,
        date: 0,
        attachments: Vec::new(),
        reply_message: None,
        fwd_messages: Vec::new(),
        important: false,
        random_id: None,
        payload: None,
        geo: None,
        action: None,
    };

    let event = normalize_event(&Event::MessageNew(message)).unwrap();
    assert!(matches!(
        event,
        VkIncomingEvent::Message(message) if message.is_group && message.user_id == 42
    ));
    assert!(is_group_peer(2_000_000_001));

    let mut callback = message_event();
    callback.payload = Some(std::collections::HashMap::from([(
        "action".to_string(),
        serde_json::json!("profile"),
    )]));
    let event = normalize_event(&Event::MessageEvent(callback)).unwrap();
    assert!(matches!(
        event,
        VkIncomingEvent::Callback(callback) if callback.payload == "profile"
    ));
}

#[test]
fn vk_transport_validates_token_and_exposes_api() {
    let transport = VkTransport::new("test_token").unwrap();
    assert_eq!(transport.api().token(), "test_token");

    assert!(VkTransport::new("").is_err());
}

#[test]
fn random_id_uses_current_time() {
    let first = random_id();
    let second = random_id();

    assert!(second >= first);
}

fn message_event() -> MessageEvent {
    MessageEvent {
        user_id: 42,
        peer_id: 42,
        event_id: "event".to_string(),
        payload: None,
        conversation_message_id: None,
    }
}
