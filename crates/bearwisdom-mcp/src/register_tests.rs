use super::*;
use tempfile::TempDir;

#[test]
fn test_register_creates_settings_file() {
    let dir = TempDir::new().unwrap();
    register(dir.path()).unwrap();

    let settings_file = dir.path().join(".mcp.json");
    assert!(settings_file.exists());

    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&settings_file).unwrap()).unwrap();
    assert!(content["mcpServers"]["bearwisdom"].is_object());
    assert_eq!(content["mcpServers"]["bearwisdom"]["type"], "stdio");
}

#[test]
fn test_register_preserves_existing_keys() {
    let dir = TempDir::new().unwrap();

    let existing = serde_json::json!({
        "mcpServers": {
            "other-server": { "type": "stdio", "command": "other" }
        },
        "someOtherSetting": true
    });
    std::fs::write(
        dir.path().join(".mcp.json"),
        serde_json::to_string_pretty(&existing).unwrap(),
    )
    .unwrap();

    register(dir.path()).unwrap();

    let content: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join(".mcp.json")).unwrap(),
    )
    .unwrap();

    assert!(content["mcpServers"]["bearwisdom"].is_object());
    assert!(content["mcpServers"]["other-server"].is_object());
    assert_eq!(content["someOtherSetting"], true);
}

#[test]
fn test_register_is_idempotent() {
    let dir = TempDir::new().unwrap();
    register(dir.path()).unwrap();
    register(dir.path()).unwrap();

    let settings_file = dir.path().join(".mcp.json");
    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&settings_file).unwrap()).unwrap();
    assert!(content["mcpServers"]["bearwisdom"].is_object());
}

#[test]
fn test_unregister_removes_entry() {
    let dir = TempDir::new().unwrap();
    register(dir.path()).unwrap();
    unregister(dir.path()).unwrap();

    let settings_file = dir.path().join(".mcp.json");
    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&settings_file).unwrap()).unwrap();
    assert!(content["mcpServers"].get("bearwisdom").is_none());
}

#[test]
fn test_unregister_preserves_other_servers() {
    let dir = TempDir::new().unwrap();

    let existing = serde_json::json!({
        "mcpServers": {
            "bearwisdom": { "type": "stdio", "command": "bw-mcp" },
            "other-server": { "type": "stdio", "command": "other" }
        }
    });
    std::fs::write(
        dir.path().join(".mcp.json"),
        serde_json::to_string_pretty(&existing).unwrap(),
    )
    .unwrap();

    unregister(dir.path()).unwrap();

    let content: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join(".mcp.json")).unwrap(),
    )
    .unwrap();

    assert!(content["mcpServers"].get("bearwisdom").is_none());
    assert!(content["mcpServers"]["other-server"].is_object());
}

#[test]
fn test_unregister_no_file() {
    let dir = TempDir::new().unwrap();
    unregister(dir.path()).unwrap();
}
