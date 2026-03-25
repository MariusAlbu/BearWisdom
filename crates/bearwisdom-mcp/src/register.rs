use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tracing::info;

/// Get the path to `.mcp.json` at the project root.
fn settings_path(project_root: &Path) -> PathBuf {
    project_root.join(".mcp.json")
}

/// Register this MCP server in `.mcp.json`.
///
/// Performs a JSON merge — only touches `mcpServers.bearwisdom`, preserving
/// all other keys and existing MCP servers.
pub fn register(project_root: &Path) -> Result<()> {
    let settings_file = settings_path(project_root);
    let binary = std::env::current_exe()
        .context("Failed to resolve binary path")?
        .canonicalize()
        .context("Failed to canonicalize binary path")?;
    let project = project_root
        .canonicalize()
        .context("Failed to canonicalize project path")?;

    // Read existing settings or start with empty object
    let mut settings: serde_json::Value = if settings_file.exists() {
        let content = std::fs::read_to_string(&settings_file)
            .context("Failed to read settings.local.json")?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Ensure mcpServers object exists
    if settings.get("mcpServers").is_none() {
        settings["mcpServers"] = serde_json::json!({});
    }

    // Use forward slashes for the paths (cross-platform compat in JSON)
    // Strip Windows extended-length path prefix (\\?\) from canonicalize()
    let binary_str = binary
        .to_string_lossy()
        .strip_prefix(r"\\?\")
        .unwrap_or(&binary.to_string_lossy())
        .replace('\\', "/");
    let project_str = project
        .to_string_lossy()
        .strip_prefix(r"\\?\")
        .unwrap_or(&project.to_string_lossy())
        .replace('\\', "/");

    // Merge our entry
    settings["mcpServers"]["bearwisdom"] = serde_json::json!({
        "type": "stdio",
        "command": binary_str,
        "args": ["--project", project_str]
    });

    // Write back with pretty-printing
    let output =
        serde_json::to_string_pretty(&settings).context("Failed to serialize settings")?;
    std::fs::write(&settings_file, output).context("Failed to write settings.local.json")?;

    info!("Registered MCP server in {}", settings_file.display());
    eprintln!(
        "Registered bearwisdom MCP server in {}",
        settings_file.display()
    );
    eprintln!("  command: {binary_str}");
    eprintln!("  project: {project_str}");

    Ok(())
}

/// Unregister this MCP server from `.mcp.json`.
///
/// Removes only the `mcpServers.bearwisdom` key, preserving everything else.
pub fn unregister(project_root: &Path) -> Result<()> {
    let settings_file = settings_path(project_root);

    if !settings_file.exists() {
        eprintln!("No settings.local.json found — nothing to unregister");
        return Ok(());
    }

    let content = std::fs::read_to_string(&settings_file)
        .context("Failed to read settings.local.json")?;
    let mut settings: serde_json::Value =
        serde_json::from_str(&content).context("Failed to parse settings.local.json")?;

    // Remove our entry
    if let Some(servers) = settings
        .get_mut("mcpServers")
        .and_then(|v| v.as_object_mut())
    {
        if servers.remove("bearwisdom").is_some() {
            let output = serde_json::to_string_pretty(&settings)
                .context("Failed to serialize settings")?;
            std::fs::write(&settings_file, output)
                .context("Failed to write settings.local.json")?;
            info!("Unregistered MCP server from {}", settings_file.display());
            eprintln!(
                "Unregistered bearwisdom from {}",
                settings_file.display()
            );
        } else {
            eprintln!("bearwisdom was not registered — nothing to remove");
        }
    } else {
        eprintln!("No mcpServers section found — nothing to unregister");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
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
}
