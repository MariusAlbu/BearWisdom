use crate::types::MonorepoInfo;
use std::path::Path;

/// Detect monorepo / workspace patterns.
///
/// Detects:
/// - npm/pnpm/yarn workspaces (package.json `workspaces` key)
/// - Cargo workspace (Cargo.toml `[workspace]`)
/// - pnpm-workspace.yaml
/// - Turborepo (turbo.json)
/// - Nx (nx.json)
/// - Lerna (lerna.json)
pub fn detect_monorepo(root: &Path) -> Option<MonorepoInfo> {
    // Cargo workspace
    if let Some(info) = detect_cargo_workspace(root) {
        return Some(info);
    }

    // pnpm workspace
    if root.join("pnpm-workspace.yaml").exists() {
        return Some(MonorepoInfo {
            kind: "pnpm-workspace".into(),
            packages: find_workspace_packages(root),
        });
    }

    // Turborepo
    if root.join("turbo.json").exists() {
        return Some(MonorepoInfo {
            kind: "turborepo".into(),
            packages: find_workspace_packages(root),
        });
    }

    // Nx
    if root.join("nx.json").exists() {
        return Some(MonorepoInfo {
            kind: "nx".into(),
            packages: find_nx_packages(root),
        });
    }

    // Lerna
    if root.join("lerna.json").exists() {
        return Some(MonorepoInfo {
            kind: "lerna".into(),
            packages: find_workspace_packages(root),
        });
    }

    // npm/yarn workspaces via package.json
    if let Some(info) = detect_npm_workspace(root) {
        return Some(info);
    }

    None
}

fn detect_cargo_workspace(root: &Path) -> Option<MonorepoInfo> {
    let content = std::fs::read_to_string(root.join("Cargo.toml")).ok()?;
    if !content.contains("[workspace]") {
        return None;
    }
    // Collect members = [...] lines as package paths.
    let packages = content
        .lines()
        .skip_while(|l| !l.trim().starts_with("members"))
        .skip(1)
        .take_while(|l| !l.trim().starts_with(']'))
        .filter_map(|l| {
            let trimmed = l.trim().trim_matches(',').trim_matches('"');
            if trimmed.is_empty() || trimmed.starts_with('#') {
                None
            } else {
                Some(trimmed.to_owned())
            }
        })
        .collect();

    Some(MonorepoInfo { kind: "cargo-workspace".into(), packages })
}

fn detect_npm_workspace(root: &Path) -> Option<MonorepoInfo> {
    let content = std::fs::read_to_string(root.join("package.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    v.get("workspaces")?;
    Some(MonorepoInfo {
        kind: "npm-workspaces".into(),
        packages: find_workspace_packages(root),
    })
}

/// Scan one level of subdirectories looking for package.json / Cargo.toml roots.
fn find_workspace_packages(root: &Path) -> Vec<String> {
    let mut packages = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                let name = entry.file_name().to_string_lossy().into_owned();
                if crate::exclusions::should_exclude(&name) {
                    continue;
                }
                let sub = entry.path();
                if sub.join("package.json").exists() || sub.join("Cargo.toml").exists() {
                    packages.push(name);
                }
            }
        }
    }
    packages.sort();
    packages
}

fn find_nx_packages(root: &Path) -> Vec<String> {
    // Nx projects live in apps/ and libs/ by convention.
    let mut packages = Vec::new();
    for dir in ["apps", "libs", "packages"] {
        let base = root.join(dir);
        if base.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&base) {
                for entry in entries.flatten() {
                    if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        packages.push(format!(
                            "{}/{}",
                            dir,
                            entry.file_name().to_string_lossy()
                        ));
                    }
                }
            }
        }
    }
    packages.sort();
    packages
}
