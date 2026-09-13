// ecosystem/manifest/gradle_plugins.rs — the plugin ids a Gradle build
// script applies.
//
// A build script names a plugin in two places: inside the `plugins { }`
// block (`id("x")`, `id 'x'`, `alias(libs.plugins.x)`, `` `x` ``) and in the
// legacy `apply plugin:` / `apply(plugin = )` statement. A catalog alias
// carries its id in a `*.versions.toml` `[plugins]` table, so resolving one
// needs the catalogs the build root declares alongside the script text.
//
// Applying a plugin is what a build script does to opt a module into a
// toolchain. A dependency coordinate mentioning the same namespace — an
// `exclude group:` line, a `classpath` entry for the plugin's own artifact —
// does not, which is why the ids come from these two constructs and never
// from a text search over the script.

use std::collections::HashMap;
use std::path::PathBuf;

/// `accessor → plugin id` for one version catalog's `[plugins]` table.
/// Accessors carry the Gradle DSL spelling: the TOML key's `-` and `_`
/// separators become `.`.
pub type PluginCatalog = HashMap<String, String>;

/// Every catalog's `[plugins]` table, keyed by the accessor prefix a build
/// script spells it with (`libs` for `gradle/libs.versions.toml`).
pub type PluginCatalogs = HashMap<String, PluginCatalog>;

/// Strip a `//` line comment from one Gradle DSL line.
pub fn strip_line_comment(line: &str) -> &str {
    match line.find("//") {
        Some(idx) => &line[..idx],
        None => line,
    }
}

/// The plugin ids `content` applies: every `plugins { }` entry (literal id,
/// backtick id, or resolved catalog alias) plus every `apply plugin:`
/// statement. Order of appearance, deduplicated.
pub fn applied_plugin_ids(content: &str, catalogs: &PluginCatalogs) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for body in plugins_block_bodies(content) {
        for raw in body.lines() {
            if let Some(id) = plugins_block_entry_id(raw, catalogs) {
                push_unique(&mut out, id);
            }
        }
    }
    for raw in content.lines() {
        if let Some(id) = apply_statement_id(raw) {
            push_unique(&mut out, id);
        }
    }
    out
}

/// Parse the `[plugins]` table of a `*.versions.toml` catalog. Two
/// declaration forms carry an id:
///   `name = { id = "com.example.plugin", version.ref = "x" }`
///   `name = "com.example.plugin:1.2.3"`
pub fn parse_plugin_catalog(content: &str) -> PluginCatalog {
    let mut out = PluginCatalog::new();
    let mut in_plugins = false;
    for raw in content.lines() {
        let line = strip_toml_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_plugins = line[1..line.len() - 1].trim() == "plugins";
            continue;
        }
        if !in_plugins {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let accessor = accessor_spelling(key.trim());
        if accessor.is_empty() {
            continue;
        }
        if let Some(id) = plugin_id_from_value(value.trim()) {
            out.insert(accessor, id);
        }
    }
    out
}

/// Parse the `[plugins]` table of every catalog file, keyed by accessor
/// prefix. Unreadable files contribute nothing.
pub fn load_plugin_catalogs(catalog_files: &[(String, PathBuf)]) -> PluginCatalogs {
    let mut out = PluginCatalogs::new();
    for (accessor, path) in catalog_files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let parsed = parse_plugin_catalog(&text);
        if parsed.is_empty() {
            continue;
        }
        out.entry(accessor.clone()).or_default().extend(parsed);
    }
    out
}

/// A TOML key's DSL accessor spelling: `-` and `_` separate segments that
/// the DSL joins with `.`.
pub(crate) fn accessor_spelling(key: &str) -> String {
    key.replace(['-', '_'], ".")
}

// ---------------------------------------------------------------------------
// `plugins { }` block
// ---------------------------------------------------------------------------

/// The body text of every `plugins { … }` block in `content`, brace-balanced.
/// The `plugins` token must stand alone so a qualified reference
/// (`libs.plugins.foo`) never opens a block.
fn plugins_block_bodies(content: &str) -> Vec<&str> {
    const TOKEN: &str = "plugins";
    let bytes = content.as_bytes();
    let mut out = Vec::new();
    let mut search = 0usize;
    while let Some(rel) = content[search..].find(TOKEN) {
        let start = search + rel;
        search = start + TOKEN.len();
        if start > 0 && is_ident_byte(bytes[start - 1]) {
            continue;
        }
        let mut i = search;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'{' {
            continue;
        }
        let body_start = i + 1;
        let mut depth = 1usize;
        let mut j = body_start;
        while j < bytes.len() {
            match bytes[j] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        if depth != 0 {
            break;
        }
        out.push(&content[body_start..j]);
        search = j;
    }
    out
}

/// The plugin id one line inside a `plugins { }` block declares.
fn plugins_block_entry_id(raw: &str, catalogs: &PluginCatalogs) -> Option<String> {
    let line = strip_line_comment(raw).trim();
    if line.is_empty() {
        return None;
    }
    if let Some(rest) = strip_call_token(line, "id") {
        return first_quoted(rest);
    }
    if let Some(rest) = strip_call_token(line, "alias") {
        return catalog_plugin_id(rest, catalogs);
    }
    backtick_id(line)
}

/// The plugin id an `apply plugin: 'x'` / `apply(plugin = "x")` statement
/// names.
fn apply_statement_id(raw: &str) -> Option<String> {
    let line = strip_line_comment(raw).trim();
    let rest = strip_call_token(line, "apply")?;
    let rest = rest.strip_prefix("plugin")?;
    let rest = rest.trim_start();
    if !rest.starts_with(':') && !rest.starts_with('=') {
        return None;
    }
    first_quoted(rest)
}

/// The id behind `alias(<catalog>.plugins.<accessor>)`.
fn catalog_plugin_id(rest: &str, catalogs: &PluginCatalogs) -> Option<String> {
    let head: String = rest
        .trim_start_matches(['(', ' '])
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '.' || *c == '_' || *c == '-')
        .collect();
    let mut parts = head.splitn(3, '.');
    let catalog = parts.next()?;
    if parts.next()? != "plugins" {
        return None;
    }
    let accessor = parts.next()?;
    catalogs.get(catalog)?.get(accessor).cloned()
}

/// `` `java-library` `` — the Kotlin DSL's backtick accessor form.
fn backtick_id(line: &str) -> Option<String> {
    let rest = line.strip_prefix('`')?;
    let end = rest.find('`')?;
    let id = &rest[..end];
    (!id.is_empty()).then(|| id.to_string())
}

// ---------------------------------------------------------------------------
// Lexing helpers
// ---------------------------------------------------------------------------

/// `line` minus a leading `token` that is followed by a call paren, a quote,
/// or whitespace — so `idea` never matches the token `id`.
fn strip_call_token<'a>(line: &'a str, token: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(token)?;
    let next = rest.chars().next()?;
    if next == '(' || next == '\'' || next == '"' || next.is_whitespace() {
        Some(rest.trim_start_matches(['(', ' ', '\t']))
    } else {
        None
    }
}

/// The first single- or double-quoted literal in `s`.
pub(crate) fn first_quoted(s: &str) -> Option<String> {
    super::gradle::extract_quoted_literals(s).into_iter().next()
}

fn strip_toml_comment(line: &str) -> &str {
    match line.find('#') {
        Some(idx) => &line[..idx],
        None => line,
    }
}

/// The id out of a `[plugins]` table value: an inline table's `id` field or
/// the `"<id>:<version>"` string shorthand.
fn plugin_id_from_value(value: &str) -> Option<String> {
    if let Some(body) = value.strip_prefix('{') {
        let body = body.strip_suffix('}').unwrap_or(body);
        for field in body.split(',') {
            let Some((k, v)) = field.split_once('=') else {
                continue;
            };
            if k.trim() == "id" {
                return first_quoted(v);
            }
        }
        return None;
    }
    let literal = first_quoted(value)?;
    let id = literal.split(':').next().unwrap_or_default();
    (!id.is_empty()).then(|| id.to_string())
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'.'
}

fn push_unique(out: &mut Vec<String>, id: String) {
    if !out.contains(&id) {
        out.push(id);
    }
}

#[cfg(test)]
#[path = "gradle_plugins_tests.rs"]
mod tests;
