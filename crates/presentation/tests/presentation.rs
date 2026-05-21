use application::DialogState;
use presentation::keyboard::{
    pay_link_keyboard, utc_keyboard_page, utc_keyboard_page_count, OFFSETS,
};
use presentation::{
    parse_channel_url, parse_command, parse_payload, BotCommand, CallbackPayload, CallbackRoute,
    ChannelPlatform, ConversationState, IncomingCallback, IncomingEvent, IncomingGroupEvent,
    IncomingMessage, MessageRoute, Notification, OutgoingCallbackAnswer, Renderer, RouteContext,
    Router, TimezoneDisplay,
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
fn incoming_event_covers_message_callback_and_group_events() {
    let message = IncomingEvent::Message(IncomingMessage::new(1, 2, "text"));
    let callback = IncomingEvent::Callback(IncomingCallback::new("event", 1, 2, "profile"));
    let group = IncomingEvent::Group(IncomingGroupEvent::new(
        2_000_000_001,
        Some(2),
        Some("chat".to_string()),
    ));

    assert!(matches!(message, IncomingEvent::Message(_)));
    assert!(matches!(callback, IncomingEvent::Callback(_)));
    assert!(matches!(group, IncomingEvent::Group(_)));
}

#[test]
fn router_matrix_covers_extended_text_states() {
    let router = Router;
    let message = IncomingMessage::new(1, 2, "42");

    let cases = [
        (
            ConversationState::AwaitingReminderEdit,
            MessageRoute::ReminderEditText("42".to_string()),
        ),
        (
            ConversationState::AwaitingReminderDeletion,
            MessageRoute::ReminderDeletionInput("42".to_string()),
        ),
        (
            ConversationState::AwaitingSubDeleteNum,
            MessageRoute::ChannelDeletionInput("42".to_string()),
        ),
        (ConversationState::AwaitingPayment, MessageRoute::Ignored),
    ];

    for (state, expected) in cases {
        assert_eq!(router.route_message_state(&message, state), expected);
    }
}

#[test]
fn router_extracts_group_mentions_and_ignores_unmentioned_group_text() {
    let router = Router;
    let mentioned =
        IncomingMessage::new(2_000_000_001, 2, "команда @yanapomnyu_bot завтра в 9").group("chat");
    let unmentioned = IncomingMessage::new(2_000_000_001, 2, "завтра в 9").group("chat");

    assert_eq!(
        router.route_message_with_context(
            &mentioned,
            ConversationState::Idle,
            RouteContext::for_bot("yanapomnyu_bot"),
        ),
        MessageRoute::GroupReminderText("команда завтра в 9".to_string())
    );
    assert_eq!(
        router.route_message_with_context(
            &unmentioned,
            ConversationState::Idle,
            RouteContext::for_bot("yanapomnyu_bot"),
        ),
        MessageRoute::Ignored
    );
}

#[test]
fn router_routes_channel_urls_as_subscription_intents() {
    let router = Router;
    let message = IncomingMessage::new(1, 2, "https://www.twitch.tv/Streamer_Name");

    assert_eq!(
        router.route_message_state(&message, ConversationState::Idle),
        MessageRoute::ChannelSubscriptionUrl(parse_channel_url("twitch.tv/Streamer_Name").unwrap())
    );

    let parsed = parse_channel_url("https://youtube.com/@yanapomnyu").unwrap();
    assert_eq!(parsed.platform, ChannelPlatform::Youtube);
    assert_eq!(parsed.channel_id, "@yanapomnyu");
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
fn router_maps_callbacks_to_transport_neutral_actions() {
    let router = Router;
    let cases = [
        ("profile_pay", CallbackRoute::ShowPayMenu),
        ("profile_list", CallbackRoute::ListReminders),
        ("setup_snooze", CallbackRoute::StartSnoozeSetup),
        ("pay_yk:12", CallbackRoute::StartYooKassaPayment(12)),
        ("reminder_done:42", CallbackRoute::CompleteReminder(42)),
    ];

    for (payload, expected) in cases {
        let callback = IncomingCallback::new("event", 1, 2, payload);
        assert_eq!(router.route_callback_action(&callback), expected);
    }
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

#[test]
fn renderer_returns_transport_neutral_output_dtos() {
    let response = Renderer
        .render_response(10, Notification::Help, TransportCapabilities::vk_inline())
        .with_callback_answer(OutgoingCallbackAnswer::new(
            "event",
            20,
            10,
            Some("ok".to_string()),
        ));

    assert_eq!(response.messages.len(), 1);
    assert_eq!(response.messages[0].peer_id, 10);
    assert_eq!(response.callback_answer.unwrap().event_id, "event");
}
