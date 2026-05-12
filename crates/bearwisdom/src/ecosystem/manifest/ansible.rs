// ecosystem/manifest/ansible.rs — Ansible requirements.yml reader
//
// Parses `requirements.yml` at the project root (and `requirements.yaml`
// as an alternate extension). The file carries role and collection
// dependency declarations in two supported shapes:
//
//   Shape A — legacy flat list:
//     - src: geerlingguy.nginx
//       name: nginx
//       version: 1.2.3
//
//   Shape B — current keyed:
//     roles:
//       - src: https://github.com/…
//         name: my_role
//     collections:
//       - name: community.general
//
// `ManifestData.dependencies` receives the declared role/collection names
// (the `name:` field when present, otherwise the bare `src:` value
// stripped of URL prefix and version suffix). These names are used by the
// Jinja resolver to classify bare-name refs that start with a declared
// external role's prefix as `external_refs`.
//
// Per the ecosystem rules this reader is locators-only: no synthetics,
// no predicates, no builtin lists. It reads the on-disk file and returns
// the set of declared dependency names.

use std::path::Path;

use super::{ManifestData, ManifestKind, ManifestReader};

pub struct AnsibleRequirementsManifest;

impl ManifestReader for AnsibleRequirementsManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::AnsibleRequirements
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        for name in &["requirements.yml", "requirements.yaml"] {
            let path = project_root.join(name);
            if path.is_file() {
                let content = std::fs::read_to_string(&path).ok()?;
                let mut data = ManifestData::default();
                for dep in parse_requirements(&content) {
                    data.dependencies.insert(dep);
                }
                return Some(data);
            }
        }
        None
    }
}

/// Parse both flat-list and keyed shapes of `requirements.yml`.
///
/// Returns all candidate variable-namespace prefixes for every declared role
/// or collection. Each entry in the returned vec is one prefix without a
/// trailing underscore; the caller appends `_` when checking `starts_with`.
///
/// For each role the following candidates are emitted:
/// - The `activation_prefix` value (trailing `_` stripped), when the field
///   is present — this is the authoritative prefix when the role author
///   provides it.
/// - The `name:` value, when present.
/// - The basename derived from `src:` (`normalise_src`), always.
/// - When `src:` is a URL, the `<org>_<basename>` composite — roles from
///   orgs like `devture` use `devture_<rolename>_` as their variable prefix
///   even though `name: <rolename>` omits the org segment.
pub fn parse_requirements(content: &str) -> Vec<String> {
    let Ok(val) = serde_yaml::from_str::<serde_yaml::Value>(content) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    match &val {
        serde_yaml::Value::Sequence(seq) => {
            // Shape A: top-level list of role entries.
            for entry in seq {
                out.extend(extract_entry_prefixes(entry));
            }
        }
        serde_yaml::Value::Mapping(map) => {
            // Shape B: keyed `roles:` and/or `collections:` sections.
            for key in &["roles", "collections"] {
                if let Some(serde_yaml::Value::Sequence(seq)) =
                    map.get(&serde_yaml::Value::String(key.to_string()))
                {
                    for entry in seq {
                        out.extend(extract_entry_prefixes(entry));
                    }
                }
            }
        }
        _ => {}
    }
    out
}

/// Extract all candidate variable-namespace prefixes from a single requirements
/// entry, without trailing underscores.
///
/// Returns up to three candidates: the `activation_prefix` (stripped),
/// the `name:` value, and the `<org>_<name>` composite when the `src:`
/// value is a URL with an extractable org segment.
fn extract_entry_prefixes(entry: &serde_yaml::Value) -> Vec<String> {
    let Some(map) = entry.as_mapping() else { return Vec::new() };
    let mut candidates: Vec<String> = Vec::new();

    let src_str: Option<&str> = map
        .get(&serde_yaml::Value::String("src".to_string()))
        .and_then(|v| v.as_str());

    // `activation_prefix:` is the authoritative prefix when present.
    if let Some(ap) = map.get(&serde_yaml::Value::String("activation_prefix".to_string())) {
        if let Some(s) = ap.as_str() {
            let s = s.trim().trim_end_matches('_');
            if !s.is_empty() {
                candidates.push(s.to_string());
                return candidates; // activation_prefix is definitive; skip derivation
            }
        }
    }

    // `name:` value — the declared role name, used as the base for prefix derivation.
    let declared_name: Option<String> =
        map.get(&serde_yaml::Value::String("name".to_string()))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

    if let Some(ref n) = declared_name {
        candidates.push(n.clone());
    }

    // `src:`-derived basename — only added when it differs from the declared name.
    if let Some(src) = src_str {
        let basename = normalise_src(src);
        if !basename.is_empty() && !candidates.contains(&basename) {
            candidates.push(basename.clone());
        }

        // For URL-shaped sources emit `<org>_<effective_name>` so roles from
        // named organisations match their `<org>_<role>_` variable convention.
        // The effective name is the declared `name:` when available; otherwise
        // the normalised src basename.
        let effective = declared_name.as_deref().unwrap_or(&basename);
        if let Some(org) = extract_url_org(src) {
            let composite = format!("{org}_{effective}");
            if !candidates.contains(&composite) {
                candidates.push(composite);
            }
        }
    }

    candidates
}

/// Extract the organisation/owner segment from a git hosting URL.
///
/// For `github.com/<org>/<repo>` and similar forges, returns `<org>`.
/// Returns `None` for non-URL values (bare Galaxy names, local paths).
fn extract_url_org(src: &str) -> Option<String> {
    // Strip scheme.
    let s = src.trim();
    let s = s
        .strip_prefix("git+https://")
        .or_else(|| s.strip_prefix("git+http://"))
        .or_else(|| s.strip_prefix("https://"))
        .or_else(|| s.strip_prefix("http://"))?;

    // Split on `/`: [host, org, repo, ...].  We need at least host + org.
    let mut parts = s.splitn(4, '/');
    let _host = parts.next()?;
    let org = parts.next()?;
    let org = org.trim();
    if org.is_empty() {
        return None;
    }
    Some(org.to_string())
}

/// Normalise a `src:` value to a bare role name.
///
/// Strips URL schemes (`git+https://`, `https://`), git hosting path
/// components, common repo-name prefixes (`ansible-role-`), and the
/// `.git` suffix. For Galaxy-style `<namespace>.<role>` values the
/// full dotted form is returned as-is because it's already a clean name.
fn normalise_src(src: &str) -> String {
    let s = src.trim();

    // Strip scheme prefixes.
    let s = s
        .strip_prefix("git+https://")
        .or_else(|| s.strip_prefix("git+http://"))
        .or_else(|| s.strip_prefix("https://"))
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s);

    // For URL-shaped sources take the last path segment as the repo name.
    let basename = if s.contains('/') {
        s.rsplit('/').next().unwrap_or(s)
    } else {
        s
    };

    // Strip `.git` suffix.
    let basename = basename.strip_suffix(".git").unwrap_or(basename);

    // Strip common Ansible role repo-name prefixes.
    let basename = basename
        .strip_prefix("ansible-role-")
        .unwrap_or(basename);

    basename.to_string()
}

// ---------------------------------------------------------------------------
// Tests live in sibling file.
// ---------------------------------------------------------------------------
#[cfg(test)]
#[path = "ansible_tests.rs"]
mod tests;
