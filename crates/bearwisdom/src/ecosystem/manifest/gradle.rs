// indexer/manifest/gradle.rs — build.gradle / build.gradle.kts reader
//
// Two layers:
//
//   1. ManifestReader impl — surfaces dependency *groupIds* for high-level
//      consumers (ProjectContext, ecosystem activation, framework-presence
//      checks). Backward-compat layer.
//
//   2. parse_gradle_coords + parse_version_catalog — produce full
//      MavenCoord {group_id, artifact_id, version} for the externals
//      walker. Resolves `libs.<accessor>` references against
//      `gradle/*.versions.toml` catalogs.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{ManifestData, ManifestKind, ManifestReader, ReaderEntry};
use crate::ecosystem::manifest::maven::MavenCoord;

pub struct GradleManifest;

impl ManifestReader for GradleManifest {
    fn kind(&self) -> ManifestKind {
        ManifestKind::Gradle
    }

    fn read(&self, project_root: &Path) -> Option<ManifestData> {
        let entries = self.read_all(project_root);
        if entries.is_empty() {
            return None;
        }
        let mut data = ManifestData::default();
        for e in &entries {
            data.dependencies.extend(e.data.dependencies.iter().cloned());
        }
        Some(data)
    }

    fn read_all(&self, project_root: &Path) -> Vec<ReaderEntry> {
        let mut gradle_paths = Vec::new();
        collect_gradle_files(project_root, &mut gradle_paths, 0);

        let mut out = Vec::new();
        for manifest_path in gradle_paths {
            let Ok(content) = std::fs::read_to_string(&manifest_path) else { continue };

            let mut data = ManifestData::default();
            for group_id in parse_gradle_dependencies(&content) {
                data.dependencies.insert(group_id);
            }

            let package_dir = manifest_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| project_root.to_path_buf());

            // Gradle doesn't mandate a name inside build.gradle — rootProject.name
            // lives in settings.gradle(.kts). Fall back to the package directory's
            // name (consistent with how Gradle itself names subprojects by default).
            let name = package_dir
                .file_name()
                .map(|s| s.to_string_lossy().into_owned());

            out.push(ReaderEntry {
                package_dir,
                manifest_path,
                data,
                name,
            });
        }
        out
    }
}

// ---------------------------------------------------------------------------
// File walkers (build.gradle[.kts] + libs.versions.toml)
// ---------------------------------------------------------------------------

fn collect_gradle_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 8 {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                ".git" | "target" | "build" | "node_modules" | ".gradle" | "bin" | "obj"
            ) {
                continue;
            }
            collect_gradle_files(&path, out, depth + 1);
        } else {
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy();
            if name == "build.gradle" || name == "build.gradle.kts" {
                out.push(path);
            }
        }
    }
}

/// Walk the project for `build.gradle` and `build.gradle.kts` files. Public
/// helper for the externals walker so it doesn't duplicate the prune list.
pub fn collect_gradle_build_files(project_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_gradle_files(project_root, &mut out, 0);
    out
}

/// Walk the project for `gradle/*.versions.toml` files. The accessor name
/// (e.g. `libs` for `libs.versions.toml`) is the file stem before
/// `.versions.toml`.
pub fn collect_version_catalogs(project_root: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    collect_version_catalogs_recursive(project_root, &mut out, 0);
    out
}

fn collect_version_catalogs_recursive(
    dir: &Path,
    out: &mut Vec<(String, PathBuf)>,
    depth: usize,
) {
    if depth > 3 {
        return;
    }
    let gradle_dir = dir.join("gradle");
    if gradle_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&gradle_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                        if let Some(stem) = file_name.strip_suffix(".versions.toml") {
                            if !stem.is_empty() {
                                out.push((stem.to_string(), path));
                            }
                        }
                    }
                }
            }
        }
    }
    if depth < 2 {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if matches!(
                            name,
                            ".git" | "build" | "target" | ".gradle" | "node_modules"
                        ) {
                            continue;
                        }
                    }
                    collect_version_catalogs_recursive(&path, out, depth + 1);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Group-ID-only parser (legacy — used by ProjectContext for framework checks)
// ---------------------------------------------------------------------------

/// Parse dependency declarations from build.gradle / build.gradle.kts.
///
/// Handles the common forms:
///   `implementation 'group:artifact:version'`
///   `implementation("group:artifact:version")`
///   `testImplementation 'group:artifact:version'`
///   `api 'group:artifact:version'`
///
/// Returns a list of groupId strings. Catalog-style references
/// (`implementation(libs.assertj.core)`) yield no entries here — those need
/// `parse_gradle_coords` with a resolved `GradleCatalog` to produce a real
/// groupId.
pub fn parse_gradle_dependencies(content: &str) -> Vec<String> {
    parse_gradle_direct_coords(content)
        .into_iter()
        .map(|c| c.group_id)
        .collect()
}

// ---------------------------------------------------------------------------
// Full-coord parser — direct string literals
// ---------------------------------------------------------------------------

const DEPENDENCY_KEYWORDS: &[&str] = &[
    "implementation",
    "testImplementation",
    "androidTestImplementation",
    "api",
    "compileOnly",
    "runtimeOnly",
    "testCompileOnly",
    "testRuntimeOnly",
    "annotationProcessor",
    "kapt",
    "ksp",
    "classpath",
    // Test fixtures plugin — Spock matchers, JUnit base classes, custom
    // test helpers. Without these, every Spec subclass that extends
    // `Specification` and every `Mock`/`Spy` call resolves to nothing.
    "testFixturesApi",
    "testFixturesImplementation",
    "testFixturesCompileOnly",
    "testFixturesRuntimeOnly",
    // Functional / integration test source sets — common in Gradle
    // multi-source-set projects.
    "functionalTestImplementation",
    "integrationTestImplementation",
];

/// Parse `<keyword> 'group:artifact:version'` and `<keyword>("g:a:v")` lines
/// into full `MavenCoord`s. Catalog-style references (`libs.foo`) are skipped
/// here — call `parse_gradle_coords` for those.
pub fn parse_gradle_direct_coords(content: &str) -> Vec<MavenCoord> {
    let mut coords = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
            continue;
        }

        let Some(rest) = strip_dependency_keyword(trimmed) else { continue };

        let coord_str = if let Some(r) = rest.strip_prefix('\'') {
            r.split('\'').next().unwrap_or("").trim()
        } else if let Some(r) = rest.strip_prefix('"') {
            r.split('"').next().unwrap_or("").trim()
        } else {
            continue;
        };

        if let Some(coord) = split_gav(coord_str) {
            coords.push(coord);
        }
    }

    coords
}

fn strip_dependency_keyword(line: &str) -> Option<&str> {
    for kw in DEPENDENCY_KEYWORDS {
        if let Some(r) = line.strip_prefix(kw) {
            let r = r.trim_start();
            // The keyword must be followed by `(`, a string literal, or
            // whitespace — otherwise we're inside a longer identifier.
            if r.starts_with('(') || r.starts_with('\'') || r.starts_with('"') {
                let r = r.trim_start_matches(['(', ' ']);
                return Some(r);
            }
            // Bare-keyword form like `implementation 'foo'` — Groovy DSL.
            if line.len() > kw.len()
                && line.as_bytes()[kw.len()].is_ascii_whitespace()
            {
                return Some(r);
            }
        }
    }
    None
}

fn split_gav(s: &str) -> Option<MavenCoord> {
    let mut parts = s.splitn(3, ':');
    let group_id = parts.next()?.trim();
    let artifact_id = parts.next()?.trim();
    let version = parts.next().map(|v| v.trim().to_string());
    if group_id.is_empty() || artifact_id.is_empty() {
        return None;
    }
    if !is_gradle_ident(group_id) || !is_gradle_ident(artifact_id) {
        return None;
    }
    Some(MavenCoord {
        group_id: group_id.to_string(),
        artifact_id: artifact_id.to_string(),
        version,
    })
}

fn is_gradle_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '.' || c == '-' || c == '_')
}

// ---------------------------------------------------------------------------
// Full-coord parser — catalog references (`libs.foo.bar`)
// ---------------------------------------------------------------------------

/// Combined coord extractor — resolves both direct `'g:a:v'` strings and
/// `<catalog>.<accessor>` catalog references through the supplied catalogs
/// map. Catalog accessor segments use Gradle's kebab→dot rule
/// (`assertj-core` in TOML → `assertj.core` in DSL).
pub fn parse_gradle_coords(
    content: &str,
    catalogs: &HashMap<String, GradleCatalog>,
) -> Vec<MavenCoord> {
    let mut coords = parse_gradle_direct_coords(content);

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('#') {
            continue;
        }
        let Some(rest) = strip_dependency_keyword(trimmed) else { continue };

        // Catalog refs only appear as bare identifiers — skip if quote-prefixed
        // (already handled by parse_gradle_direct_coords).
        if rest.starts_with('\'') || rest.starts_with('"') {
            continue;
        }

        // Parse `<catalog>.<seg>(.seg)+` until we hit `)`, `,`, `{`, or EOL.
        let head: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '.' || *c == '_')
            .collect();
        if head.is_empty() {
            continue;
        }
        let mut parts = head.splitn(2, '.');
        let catalog_name = parts.next().unwrap_or("");
        let accessor = parts.next().unwrap_or("");
        if accessor.is_empty() {
            continue;
        }
        if let Some(catalog) = catalogs.get(catalog_name) {
            if let Some(coord) = catalog.libraries.get(accessor) {
                coords.push(coord.clone());
            }
        }
    }

    coords
}

// ---------------------------------------------------------------------------
// Version catalog parser (`gradle/libs.versions.toml`)
// ---------------------------------------------------------------------------

/// Parsed `libs.versions.toml`. `versions` resolves `version.ref` indirection;
/// `libraries` keys are the **dotted accessor form** (kebabs in TOML are
/// converted to dots, matching Gradle DSL access).
#[derive(Debug, Default, Clone)]
pub struct GradleCatalog {
    pub versions: HashMap<String, String>,
    pub libraries: HashMap<String, MavenCoord>,
}

/// Parse a Gradle version catalog. Handles three library forms:
///   - `name = { module = "g:a", version = "1.0" }`
///   - `name = { module = "g:a", version.ref = "kotlin" }`
///   - `name = { group = "g", name = "a", version = "1.0" }`
///
/// Plugins and bundles are ignored (out of scope for externals discovery).
/// Tolerates leading/trailing whitespace, comments (`#`), and blank lines.
pub fn parse_version_catalog(content: &str) -> GradleCatalog {
    let mut out = GradleCatalog::default();
    let mut section: Option<&str> = None;

    for raw_line in content.lines() {
        let line = strip_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = parse_section_header(line) {
            section = Some(name);
            continue;
        }
        let Some(sec) = section else { continue };

        match sec {
            "versions" => {
                if let Some((k, v)) = parse_string_kv(line) {
                    out.versions.insert(k, v);
                }
            }
            "libraries" => {
                if let Some((key, body)) = parse_inline_table_line(line) {
                    if let Some(coord) = parse_library_inline(&body, &out.versions) {
                        let accessor = key.replace('-', ".");
                        out.libraries.insert(accessor, coord);
                    }
                }
            }
            _ => {} // [plugins], [bundles] — ignored
        }
    }

    out
}

fn strip_comment(line: &str) -> &str {
    // Naive — fine for catalog files because `#` doesn't appear in coords or
    // version strings. TOML proper requires quote-awareness; we don't need it.
    if let Some(idx) = line.find('#') {
        &line[..idx]
    } else {
        line
    }
}

fn parse_section_header(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.starts_with('[') && line.ends_with(']') {
        Some(line[1..line.len() - 1].trim())
    } else {
        None
    }
}

fn parse_string_kv(line: &str) -> Option<(String, String)> {
    let (k, v) = line.split_once('=')?;
    let k = k.trim();
    let v = v.trim();
    if k.is_empty() {
        return None;
    }
    let value = unquote(v)?;
    Some((k.to_string(), value))
}

fn parse_inline_table_line(line: &str) -> Option<(String, String)> {
    let (k, v) = line.split_once('=')?;
    let k = k.trim();
    let v = v.trim();
    if !v.starts_with('{') {
        return None;
    }
    let close = v.rfind('}')?;
    let body = &v[1..close];
    Some((k.to_string(), body.to_string()))
}

fn parse_library_inline(body: &str, versions: &HashMap<String, String>) -> Option<MavenCoord> {
    let mut module: Option<String> = None;
    let mut group: Option<String> = None;
    let mut name: Option<String> = None;
    let mut version: Option<String> = None;

    for field in split_table_fields(body) {
        let Some((k, v)) = field.split_once('=') else { continue };
        let k = k.trim();
        let v = v.trim();
        match k {
            "module" => module = unquote(v),
            "group" => group = unquote(v),
            "name" => name = unquote(v),
            "version" => {
                if let Some(s) = unquote(v) {
                    version = Some(s);
                }
            }
            "version.ref" => {
                if let Some(s) = unquote(v) {
                    version = versions.get(&s).cloned();
                }
            }
            _ => {} // nested version = { ref = "..." } skipped in MVP
        }
    }

    if let Some(m) = module {
        let (gid, aid) = m.split_once(':')?;
        return Some(MavenCoord {
            group_id: gid.to_string(),
            artifact_id: aid.to_string(),
            version,
        });
    }
    if let (Some(g), Some(a)) = (group, name) {
        return Some(MavenCoord {
            group_id: g,
            artifact_id: a,
            version,
        });
    }
    None
}

fn split_table_fields(body: &str) -> Vec<String> {
    // Top-level comma split, ignoring commas inside `"..."` strings. Catalog
    // inline tables don't nest, so we don't track brace depth.
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut in_str = false;
    for c in body.chars() {
        match c {
            '"' => {
                in_str = !in_str;
                buf.push(c);
            }
            ',' if !in_str => {
                if !buf.trim().is_empty() {
                    out.push(buf.trim().to_string());
                }
                buf.clear();
            }
            _ => buf.push(c),
        }
    }
    if !buf.trim().is_empty() {
        out.push(buf.trim().to_string());
    }
    out
}

fn unquote(s: &str) -> Option<String> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        Some(rest.to_string())
    } else {
        s.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')).map(|r| r.to_string())
    }
}

// ---------------------------------------------------------------------------
// Gradle version-catalog name discovery (for PluginStateBag)
// ---------------------------------------------------------------------------

/// Newtype wrapping the list of catalog accessor prefixes found in this
/// project (e.g. `["libs"]` for a standard `gradle/libs.versions.toml`).
///
/// Stored in `PluginStateBag` by the Kotlin plugin so the Kotlin resolver
/// can classify `libs.*` and `catalog.*` refs as external Gradle DSL.
#[derive(Debug, Default, Clone)]
pub struct GradleCatalogNames(pub Vec<String>);

/// Discover the version-catalog accessor names declared in this project.
///
/// Scans for `gradle/*.versions.toml` files and returns the file stems
/// before `.versions.toml` (e.g. `libs` for `gradle/libs.versions.toml`).
pub fn discover_gradle_catalog_names(project_root: &std::path::Path) -> GradleCatalogNames {
    let names = collect_version_catalogs(project_root)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    GradleCatalogNames(names)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "gradle_tests.rs"]
mod tests;
