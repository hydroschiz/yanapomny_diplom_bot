use application::DialogState;
use presentation::keyboard::{
    pay_link_keyboard, utc_keyboard_page, utc_keyboard_page_count, OFFSETS,
};
use presentation::{
    parse_command, parse_payload, BotCommand, CallbackPayload, IncomingCallback, IncomingMessage,
    MessageRoute, Notification, Renderer, Router, TimezoneDisplay,
};
use transport_core::{TextFormat, TransportButton, TransportCapabilities};

#[test]
fn command_parser_handles_bot_mentions_and_args() {
    let parsed = parse_command("/Remind@yanapomnyu_bot через 10 минут чай").unwrap();

    assert_eq!(
        parsed.command,
        BotCommand::Remind("через 10 минут чай".to_string())
    );
    assert_eq!(parsed.args, "через 10 минут чай");
}

#[test]
fn router_maps_commands_to_message_routes() {
    let router = Router;
    let cases = [
        ("/start", MessageRoute::Start),
        ("/help", MessageRoute::Help),
        ("/yan", MessageRoute::Yan),
        ("/utc", MessageRoute::ShowUtc),
        ("/setup", MessageRoute::ShowSetup),
        ("/pay", MessageRoute::ShowPay),
        ("/list", MessageRoute::ListReminders),
        ("/subs", MessageRoute::ShowSubscriptions),
        ("/profile", MessageRoute::ShowProfile),
        ("/ref", MessageRoute::ShowReferral),
        (
            "/remind завтра в 9 созвон",
            MessageRoute::CreateReminderFromCommand("завтра в 9 созвон".to_string()),
        ),
    ];

    for (text, expected) in cases {
        let message = IncomingMessage::new(1, 2, text);
        assert_eq!(router.route_message(&message, DialogState::Idle), expected);
    }
}

#[test]
fn router_routes_plain_text_by_dialog_state() {
    let router = Router;
    let message = IncomingMessage::new(1, 2, "Москва");

    assert_eq!(
        router.route_message(&message, DialogState::AwaitingUtc),
        MessageRoute::UtcInput("Москва".to_string())
    );
    assert_eq!(
        router.route_message(&message, DialogState::Idle),
        MessageRoute::ReminderText("Москва".to_string())
    );
}

#[test]
fn payload_parser_handles_callbacks_with_arguments() {
    assert_eq!(parse_payload("utc_page:3"), CallbackPayload::UtcPage(3));
    assert_eq!(
        parse_payload("utc_set:+05:45"),
        CallbackPayload::UtcSet("+05:45".to_string())
    );
    assert_eq!(parse_payload("pay_select:6"), CallbackPayload::PaySelect(6));
    assert_eq!(parse_payload("pay_yk:12"), CallbackPayload::PayYooKassa(12));
    assert_eq!(parse_payload("pay_check:3"), CallbackPayload::PayCheck(3));
    assert_eq!(
        parse_payload("snooze:42:1hourSnooze"),
        CallbackPayload::Snooze {
            reminder_id: 42,
            code: "1hourSnooze".to_string(),
        }
    );
    assert_eq!(
        parse_payload("reminder_done:42"),
        CallbackPayload::ReminderDone(42)
    );
}

#[test]
fn router_uses_payload_parser_for_callbacks() {
    let router = Router;
    let callback = IncomingCallback::new("event", 1, 2, "profile_pay");

    assert_eq!(
        router.route_callback(&callback),
        CallbackPayload::ProfilePay
    );
}

#[test]
fn utc_keyboard_pages_fit_vk_limits_and_cover_offsets() {
    let capabilities = TransportCapabilities::vk_inline();
    let mut offsets = Vec::new();

    for page in 0..utc_keyboard_page_count(capabilities) {
        let keyboard = utc_keyboard_page(capabilities, page);
        assert!(keyboard.fits(capabilities));

        for row in keyboard.rows {
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

#[test]
fn pay_link_keyboard_keeps_url_button_transport_neutral() {
    let keyboard = pay_link_keyboard("https://pay.example", 3, TransportCapabilities::vk_inline());

    assert!(keyboard.fits(TransportCapabilities::vk_inline()));
    assert!(matches!(
        &keyboard.rows[0][0],
        TransportButton::Url { url, .. } if url == "https://pay.example"
    ));
}

#[test]
fn renderer_strips_html_when_transport_does_not_support_html() {
    let content = Renderer.render(
        Notification::UtcPrompt {
            current: TimezoneDisplay::NotSet,
        },
        TransportCapabilities::vk_inline(),
    );

    assert_eq!(content.format, TextFormat::Plain);
    assert!(!content.text.contains("<b>"));
    assert!(content
        .keyboard
        .unwrap()
        .fits(TransportCapabilities::vk_inline()));
}

#[test]
fn renderer_keeps_html_when_transport_supports_html() {
    let content = Renderer.render(
        Notification::PayMenu {
            is_active: true,
            expiry: Some("01.01.2027".to_string()),
        },
        TransportCapabilities::unlimited(),
    );

    assert_eq!(content.format, TextFormat::Html);
    assert!(content.text.contains("<b>Действует до:</b> 01.01.2027"));
}
