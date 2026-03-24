use crate::types::{DetectedSdk, SdkDescriptor};
use std::path::Path;
use std::process::Command;

/// Run an SDK version command and return the captured stdout, trimmed.
/// Uses `cmd.exe /C` on Windows so that bare executable names resolve via PATH.
fn run_version_command(sdk: &SdkDescriptor) -> Option<String> {
    #[cfg(windows)]
    let output = {
        // Build a single command string: "rustc --version"
        let cmd_str = format!("{} {}", sdk.version_command, sdk.version_args.join(" "));
        Command::new("cmd").args(["/C", &cmd_str]).output()
    };

    #[cfg(not(windows))]
    let output = Command::new(sdk.version_command).args(sdk.version_args).output();

    match output {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
            // Some tools (e.g. java -version) print to stderr.
            let raw = if !stdout.is_empty() { stdout } else { stderr };
            if raw.is_empty() { None } else { Some(raw) }
        }
        _ => None,
    }
}

/// Read the pinned version from a version file in `root`.
fn read_pinned_version(root: &Path, sdk: &SdkDescriptor) -> Option<String> {
    let vf = sdk.version_file?;
    let path = root.join(vf);
    if !path.exists() {
        return None;
    }

    let content = std::fs::read_to_string(&path).ok()?;

    if let Some(json_key) = sdk.version_json_key {
        // Drill into a JSON path like "sdk.version"
        let v: serde_json::Value = serde_json::from_str(&content).ok()?;
        let mut cur = &v;
        for segment in json_key.split('.') {
            cur = cur.get(segment)?;
        }
        return cur.as_str().map(|s| s.to_owned());
    }

    // Plain text version file (e.g. .nvmrc, .ruby-version) — first non-empty line.
    content.lines().find(|l| !l.trim().is_empty()).map(|l| l.trim().to_owned())
}

/// Check all SDKs for detected languages and return `DetectedSdk` entries.
pub fn check_sdks(
    root: &Path,
    language_ids: &[String],
    check_sdks: bool,
) -> Vec<DetectedSdk> {
    language_ids
        .iter()
        .filter_map(|id| crate::registry::find_language(id))
        .filter_map(|lang| {
            let sdk = lang.sdk.as_ref()?;
            let installed_version = if check_sdks {
                run_version_command(sdk)
            } else {
                None
            };
            let pinned_version = read_pinned_version(root, sdk);

            Some(DetectedSdk {
                language_id: lang.id.to_owned(),
                sdk_name: sdk.name.to_owned(),
                installed_version,
                pinned_version,
                install_url: sdk.install_url.to_owned(),
            })
        })
        .collect()
}
