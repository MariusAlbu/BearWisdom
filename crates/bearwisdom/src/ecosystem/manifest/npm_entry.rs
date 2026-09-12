// =============================================================================
// ecosystem/manifest/npm_entry.rs — the files a bare import of a package
// resolves to, as its package.json declares them
// =============================================================================

/// Entry candidates in priority order: every path under `exports["."]`
/// (custom conditions included) with `types`/`typings` leaves ahead of the
/// other conditions at each level, then the `types`, `typings`, `main` and
/// `module` fields. Leading `./` is dropped; duplicates keep their first
/// position. The consumer resolves each against the files it holds and takes
/// the first that exists, so a build output that was never indexed yields to
/// the source entry a custom condition names.
pub fn package_entries(content: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    if let Some(root) = value.get("exports") {
        let dot = match root {
            serde_json::Value::Object(map) if map.keys().any(|k| k.starts_with('.')) => {
                map.get(".")
            }
            other => Some(other),
        };
        if let Some(dot) = dot {
            push_leaves(dot, &mut out);
        }
    }
    for field in ["types", "typings", "main", "module"] {
        if let Some(path) = value.get(field).and_then(|v| v.as_str()) {
            push_unique(&mut out, path);
        }
    }
    out
}

fn push_leaves(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(path) => push_unique(out, path),
        serde_json::Value::Object(map) => {
            let declaration_first = |(key, _): &(&String, &serde_json::Value)| {
                !matches!(key.as_str(), "types" | "typings")
            };
            let mut conditions: Vec<_> = map.iter().collect();
            conditions.sort_by_key(declaration_first);
            for (_, leaf) in conditions {
                push_leaves(leaf, out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                push_leaves(item, out);
            }
        }
        _ => {}
    }
}

fn push_unique(out: &mut Vec<String>, path: &str) {
    let path = path.trim_start_matches("./");
    if path.is_empty() || path.ends_with('/') || out.iter().any(|p| p == path) {
        return;
    }
    out.push(path.to_string());
}

#[cfg(test)]
#[path = "npm_entry_tests.rs"]
mod tests;
