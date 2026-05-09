use super::spec_for_body;

#[test]
fn spec_for_body_returns_ads_for_adb() {
    assert_eq!(
        spec_for_body("src/bmp280.adb"),
        Some("src/bmp280.ads".to_string())
    );
}

#[test]
fn spec_for_body_handles_unix_path() {
    assert_eq!(
        spec_for_body("drivers/sensors/bmp280.adb"),
        Some("drivers/sensors/bmp280.ads".to_string())
    );
}

#[test]
fn spec_for_body_handles_windows_separators() {
    assert_eq!(
        spec_for_body("drivers\\sensors\\bmp280.adb"),
        Some("drivers/sensors/bmp280.ads".to_string())
    );
}

#[test]
fn spec_for_body_returns_none_for_ads() {
    assert_eq!(spec_for_body("src/bmp280.ads"), None);
}

#[test]
fn spec_for_body_returns_none_for_other_extension() {
    assert_eq!(spec_for_body("src/main.rs"), None);
    assert_eq!(spec_for_body("src/foo.py"), None);
}

#[test]
fn spec_for_body_bare_filename() {
    assert_eq!(spec_for_body("bmp280.adb"), Some("bmp280.ads".to_string()));
}
