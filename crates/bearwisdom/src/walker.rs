// =============================================================================
// walker.rs  —  gitignore-aware file discovery
//
// Delegates entirely to `bearwisdom_profile::walk_files`, which owns the
// walk logic (UNC stripping, OverrideBuilder exclusions, gitignore, sorting).
// This module exists to provide the `WalkedFile` type used by the indexer and
// the `detect_language` helper used by the parser layer.
// =============================================================================

use bearwisdom_profile::detect_language as profile_detect_language;
use bearwisdom_profile::ScannedFile;
use anyhow::Result;
use std::path::{Path, PathBuf};

/// A file that was found and is ready to be parsed.
#[derive(Debug, Clone)]
pub struct WalkedFile {
    /// Path as stored in the DB (relative to project root, forward slashes).
    pub relative_path: String,
    /// Absolute path — used for reading file contents.
    pub absolute_path: PathBuf,
    /// Language identifier (e.g. "csharp", "typescript").
    pub language: &'static str,
}

impl From<&ScannedFile> for WalkedFile {
    fn from(sf: &ScannedFile) -> Self {
        WalkedFile {
            relative_path: sf.relative_path.clone(),
            absolute_path: sf.absolute_path.clone(),
            language: sf.language_id,
        }
    }
}

/// Walk `project_root` and return all indexable source files.
///
/// Delegates to `bearwisdom_profile::walk_files` for all discovery logic.
/// Files are sorted by relative path for deterministic output across OSes.
pub fn walk(project_root: &Path) -> Result<Vec<WalkedFile>> {
    let scanned = bearwisdom_profile::walk_files(project_root);
    Ok(scanned.iter().map(WalkedFile::from).collect())
}

/// Map a file path to a language identifier.
///
/// Returns `None` for paths we don't support so the caller can skip the file.
/// Delegates to `bearwisdom-profile` for all detection logic, preserving
/// the `Option<&'static str>` return type expected by callers.
///
/// Special cases:
/// - `.pp` is shared between Puppet manifests and Free Pascal source. The
///   profile matcher returns Puppet by default; override to Pascal when
///   the file head shows clear Pascal markers (`program`, `unit`,
///   `library`, `{$mode`, etc.).
/// - `.pl` is shared between Perl and Prolog. The profile matcher returns
///   Perl by default (Perl is listed first in the registry); override to
///   Prolog when the head shows clear Prolog markers (`:- module(`,
///   `:- use_module(`, `:- discontiguous`, etc.).
/// - `.h` is shared between C and C++ headers. The profile matcher
///   returns C by default; override to C++ when the head shows
///   C++-only constructs (templates, namespaces, `class` declarations,
///   `nullptr`, `extern "C"`, etc.). The C plugin handles both grammars,
///   but the language tag drives which tree-sitter grammar is used.
pub fn detect_language(path: &Path) -> Option<&'static str> {
    let lang = profile_detect_language(path).map(|desc| desc.id);
    if lang == Some("puppet") && is_likely_pascal(path) {
        return Some("pascal");
    }
    if lang == Some("perl") && is_likely_prolog(path) {
        return Some("prolog");
    }
    if lang == Some("c") && is_dot_h(path) && bearwisdom_profile::file_looks_like_cpp(path) {
        return Some("cpp");
    }
    lang
}

fn is_dot_h(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("h"))
        .unwrap_or(false)
}

/// Read the first 512 bytes of a `.pp` file and look for Pascal-source
/// markers. Catches `program`, `unit`, `library`, `{$mode ...}`, and the
/// `(* ... *)` block-comment header style; Puppet manifests use `class`,
/// `define`, `node`, `include`, `$var = ...` instead.
fn is_likely_pascal(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else { return false };
    use std::io::Read;
    let mut head = [0u8; 512];
    let n = match (&file).take(512).read(&mut head) {
        Ok(n) => n,
        Err(_) => return false,
    };
    let text = String::from_utf8_lossy(&head[..n]);
    // Strip leading whitespace/comments aggressively.
    let mut t = text.as_ref();
    loop {
        let trimmed = t.trim_start();
        if let Some(rest) = trimmed.strip_prefix("//") {
            t = rest.split_once('\n').map(|(_, r)| r).unwrap_or("");
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("(*") {
            t = rest.split_once("*)").map(|(_, r)| r).unwrap_or("");
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('{') {
            // Pascal `{...}` block comment — skip past matching `}`.
            t = rest.split_once('}').map(|(_, r)| r).unwrap_or("");
            continue;
        }
        break;
    }
    let head_lower = t.trim_start().to_ascii_lowercase();
    head_lower.starts_with("program ")
        || head_lower.starts_with("unit ")
        || head_lower.starts_with("library ")
        || head_lower.starts_with("uses ")
        || head_lower.starts_with("interface\n")
        || head_lower.starts_with("interface\r\n")
        || head_lower.contains("{$mode ")
        || head_lower.contains("{$mode}")
        || head_lower.contains("{$mode:")
}

/// Read the first 1024 bytes of a `.pl` file and look for Prolog-source
/// markers. Catches the canonical `:- module(...)`, `:- use_module(...)`,
/// `:- discontiguous`, `:- dynamic`, `:- multifile`, `:- table` directives,
/// plus the `head(...) :-` rule shape and the doc-comment header style
/// (`/** <module> ... */`). Perl scripts use `use strict;`, `package`,
/// shebangs, and sigil-prefixed variables — none overlap with these
/// markers.
fn is_likely_prolog(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else { return false };
    use std::io::Read;
    let mut head = [0u8; 1024];
    let n = match (&file).take(1024).read(&mut head) {
        Ok(n) => n,
        Err(_) => return false,
    };
    let text = String::from_utf8_lossy(&head[..n]);
    // Strong Perl indicator — bail out fast.
    if text.starts_with("#!") && text[..text.find('\n').unwrap_or(text.len())].contains("perl") {
        return false;
    }
    if text.contains("\nuse strict;")
        || text.contains("\nuse warnings;")
        || text.starts_with("use strict;")
        || text.starts_with("use warnings;")
        || text.contains("\npackage ")
    {
        return false;
    }
    // Strong Prolog markers — directives and module-doc headers.
    let prolog_markers = [
        ":- module(",
        ":- use_module(",
        ":- ensure_loaded(",
        ":- discontiguous",
        ":- dynamic",
        ":- multifile",
        ":- table ",
        ":- table\t",
        ":- set_prolog_flag",
        ":- op(",
        "/** <module>",
    ];
    if prolog_markers.iter().any(|m| text.contains(m)) {
        return true;
    }
    // No directives, no module header. The file might still be Prolog
    // (older `.P` / `.pl` files written as flat clause lists). Score
    // lines: a leading non-comment non-blank line shaped like
    // `head(args).` or `head(args) :- ...` is a strong Prolog signal,
    // while `sub `, `my `, `our `, `local `, `print ` are Perl.
    let mut prolog_score = 0u32;
    let mut perl_score = 0u32;
    for line in text.lines().take(60) {
        let t = line.trim_start();
        if t.starts_with('%') || t.is_empty() || t.starts_with("/*") || t.starts_with('*') {
            continue;
        }
        // Perl indicators.
        if t.starts_with("sub ")
            || t.starts_with("my ")
            || t.starts_with("our ")
            || t.starts_with("local ")
            || t.starts_with("print ")
            || t.starts_with("if (")
            || t.starts_with("foreach ")
            || t.starts_with("while (")
            || t.starts_with("require ")
        {
            perl_score += 1;
            continue;
        }
        // Prolog clause: contains `:-` operator (rule body) OR ends in `.`
        // and looks like a functor application.
        if t.contains(":-") || t.contains("?-") {
            prolog_score += 2;
            continue;
        }
        // `head(args).` — fact. Reject Perl-shape `func(args);`.
        let trimmed_end = t.trim_end();
        if trimmed_end.ends_with('.')
            && !trimmed_end.ends_with(";")
            && trimmed_end.contains('(')
            && trimmed_end.contains(')')
        {
            prolog_score += 1;
        }
    }
    prolog_score >= 2 && prolog_score > perl_score
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "walker_tests.rs"]
mod tests;
