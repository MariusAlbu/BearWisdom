// ecosystem/android_compile_sdk.rs — the platform API level a Gradle module
// compiles against.
//
// The Android extension pins the platform in one of four spellings:
//
//   compileSdk = 35                                   Kotlin DSL, literal
//   compileSdk 35                                     Groovy DSL, literal
//   compileSdkVersion(29)                             legacy accessor
//   compileSdkVersion "android-30"                    platform directory name
//   compileSdk = libs.versions.android.compileSdk...  version-catalog ref
//
// The pin names the `platforms/android-<N>` and `sources/android-<N>`
// directories of the SDK install the module builds against.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::ecosystem::manifest::gradle::parse_version_catalog;
use crate::ecosystem::manifest::gradle_plugins::{
    accessor_spelling, first_quoted, strip_line_comment,
};

/// Every catalog's `[versions]` table, keyed by the accessor prefix a build
/// script spells it with, then by the version's DSL accessor spelling.
pub type VersionCatalogs = HashMap<String, HashMap<String, String>>;

/// The catalog segment a build script reaches a version entry through.
const VERSIONS_SEGMENT: &str = "versions";

/// Load the `[versions]` table of every catalog file, keyed by accessor
/// prefix. Unreadable files contribute nothing.
pub fn load_version_catalogs(catalog_files: &[(String, PathBuf)]) -> VersionCatalogs {
    let mut out = VersionCatalogs::new();
    for (accessor, path) in catalog_files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        let catalog = parse_version_catalog(&text);
        if catalog.versions.is_empty() {
            continue;
        }
        let entry = out.entry(accessor.clone()).or_default();
        for (key, value) in catalog.versions {
            entry.insert(accessor_spelling(&key), value);
        }
    }
    out
}

/// The platform API level `content` pins, or `None` when the script declares
/// no numeric pin (an unpinned module, or one on a preview platform named by
/// codename rather than level).
pub fn compile_sdk_pin(content: &str, catalogs: &VersionCatalogs) -> Option<u32> {
    for raw in content.lines() {
        let line = strip_line_comment(raw).trim();
        let Some(rest) = strip_compile_sdk_token(line) else {
            continue;
        };
        if let Some(level) = api_level(rest, catalogs) {
            return Some(level);
        }
    }
    None
}

/// `line` minus a leading `compileSdk`/`compileSdkVersion` accessor and the
/// assignment or call punctuation that follows it.
fn strip_compile_sdk_token(line: &str) -> Option<&str> {
    for token in ["compileSdkVersion", "compileSdk"] {
        let Some(rest) = line.strip_prefix(token) else {
            continue;
        };
        // A longer accessor (`compileSdkPreview`, `compileSdkExtension`) pins
        // something this reader does not express.
        if rest.starts_with(|c: char| c.is_alphanumeric() || c == '_') {
            return None;
        }
        return Some(rest.trim_start_matches(['(', '=', ' ', '\t']));
    }
    None
}

/// The API level a pin expression evaluates to.
fn api_level(rest: &str, catalogs: &VersionCatalogs) -> Option<u32> {
    let rest = rest.trim();
    if rest.starts_with('"') || rest.starts_with('\'') {
        return parse_level(&first_quoted(rest)?);
    }
    if rest.starts_with(|c: char| c.is_ascii_digit()) {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        return digits.parse().ok();
    }
    catalog_level(rest, catalogs)
}

/// The level behind `<catalog>.versions.<accessor>` — the catalog literal is
/// itself a level or a platform directory name.
fn catalog_level(rest: &str, catalogs: &VersionCatalogs) -> Option<u32> {
    let head: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '.' || *c == '_' || *c == '-')
        .collect();
    let mut parts = head.splitn(3, '.');
    let catalog = parts.next()?;
    if parts.next()? != VERSIONS_SEGMENT {
        return None;
    }
    let accessor = parts.next()?;
    // `.get()`, `.toInt()` and friends trail the accessor; the catalog key is
    // the longest prefix that names an entry.
    let table = catalogs.get(catalog)?;
    let mut candidate = accessor;
    loop {
        if let Some(literal) = table.get(candidate) {
            return parse_level(literal);
        }
        match candidate.rfind('.') {
            Some(idx) => candidate = &candidate[..idx],
            None => return None,
        }
    }
}

/// A level literal: a bare number, or the `android-<N>` platform directory
/// name the legacy accessor takes.
fn parse_level(literal: &str) -> Option<u32> {
    let trimmed = literal.trim();
    if let Ok(n) = trimmed.parse::<u32>() {
        return Some(n);
    }
    trimmed.strip_prefix("android-")?.parse().ok()
}

#[cfg(test)]
#[path = "android_compile_sdk_tests.rs"]
mod tests;
