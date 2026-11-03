use featherplay_integrations::parse_hotkey;

#[test]
fn parse_hotkey_valid() {
    assert!(parse_hotkey("Ctrl+Alt+Space").is_ok());
    assert!(parse_hotkey("Ctrl+Shift+Right").is_ok());
}

#[test]
fn parse_hotkey_rejects_bare_key() {
    assert!(parse_hotkey("Space").is_err());
}
