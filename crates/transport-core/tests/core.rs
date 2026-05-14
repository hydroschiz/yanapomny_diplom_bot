use transport_core::{strip_html, TransportButton, TransportCapabilities, TransportKeyboard};

#[test]
fn keyboard_builders_preserve_rows_and_buttons() {
    let keyboard = TransportKeyboard::empty()
        .add_row(vec![TransportButton::callback("A", "a")])
        .add_row(vec![
            TransportButton::callback("B", "b"),
            TransportButton::url("C", "https://c.example"),
        ]);

    assert_eq!(keyboard.row_count(), 2);
    assert_eq!(keyboard.button_count(), 3);
    assert!(!keyboard.is_empty());
}

#[test]
fn keyboard_checks_capabilities() {
    let keyboard = TransportKeyboard::new(vec![
        vec![TransportButton::callback("1", "1")],
        vec![TransportButton::callback("2", "2")],
    ]);
    let capabilities = TransportCapabilities {
        max_keyboard_rows: Some(1),
        ..TransportCapabilities::unlimited()
    };

    assert!(!keyboard.fits(capabilities));
    assert!(keyboard.fits(TransportCapabilities::unlimited()));
}

#[test]
fn callback_and_url_buttons_keep_payloads() {
    let callback = TransportButton::callback("Нажми", "action:1");
    let url = TransportButton::url("Открыть", "https://example.com");

    assert_eq!(callback.label(), "Нажми");
    assert_eq!(url.label(), "Открыть");

    assert!(matches!(callback, TransportButton::Callback { .. }));
    assert!(matches!(url, TransportButton::Url { .. }));
}

#[test]
fn strip_html_keeps_readable_text() {
    assert_eq!(strip_html("<b>Привет</b>"), "Привет");
    assert_eq!(strip_html("Tom &amp; Jerry"), "Tom & Jerry");
    assert_eq!(
        strip_html(r#"<a href="https://example.com">Ссылка</a>"#),
        "Ссылка (https://example.com)"
    );
}
