// =============================================================================
// search/grep.rs  —  on-demand filesystem text/regex search (Ctrl+Shift+F)
//
// Walks the project tree with .gitignore awareness, builds a ripgrep-backed
// Searcher for each file, collects per-line matches with byte-level column
// offsets, and supports cancellation between files.
//
// Design notes:
//   • Non-regex mode escapes metacharacters manually so we never take the
//     `regex` crate as a transitive dependency.
//   • We use a single Searcher per call (Searcher is cheap to construct but
//     re-use avoids repeated heap allocs for the internal buffer).
//   • The Sink implementation is a plain closure via `sinks::UTF8`.  Context
//     lines are not implemented here — the spec says to skip them when they
//     complicate the Sink.
// =============================================================================

use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use anyhow::{Context, Result};
use grep_matcher::Matcher;
use grep_regex::RegexMatcherBuilder;
use grep_searcher::{sinks::UTF8, Searcher, SearcherBuilder};
use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use tracing::trace;

use crate::search::scope::{detect_language_from_path, SearchScope};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A single line-level match from a grep search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrepMatch {
    /// File path relative to the project root, forward-slash separated.
    pub file_path: String,
    /// 1-based line number.
    pub line_number: u32,
    /// 0-based byte offset of the match start within the line.
    pub column: u32,
    /// The matched line with trailing newline stripped.
    pub line_content: String,
    /// Byte offset of the match start within `line_content`.
    pub match_start: u32,
    /// Byte offset of the match end within `line_content`.
    pub match_end: u32,
}

/// Options that control a grep search.
#[derive(Debug, Clone)]
pub struct GrepOptions {
    /// Match case exactly.  Default: `true`.
    pub case_sensitive: bool,
    /// Only match whole words (word boundary on both sides).  Default: `false`.
    pub whole_word: bool,
    /// Treat `pattern` as a regular expression.  Default: `false` (literal).
    pub regex: bool,
    /// Stop after collecting this many matches.  Default: `1000`.
    pub max_results: usize,
    /// File/language/directory filter applied before searching.
    pub scope: SearchScope,
    /// Lines of context before and after each match.  Default: `0`.
    ///
    /// Non-zero values are accepted but context lines themselves are not
    /// emitted in the current implementation — only direct match lines are
    /// returned.
    pub context_lines: u32,
}

impl Default for GrepOptions {
    fn default() -> Self {
        Self {
            case_sensitive: true,
            whole_word: false,
            regex: false,
            max_results: 1000,
            scope: SearchScope::default(),
            context_lines: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Search `project_root` for `pattern`, returning line-level matches.
///
/// Walks the directory tree respecting `.gitignore` rules.  Files are pre-
/// filtered by `options.scope` before any I/O.  The search is cancelled
/// between files when `cancelled` is set to `true`.
pub fn grep_search(
    project_root: &Path,
    pattern: &str,
    options: &GrepOptions,
    cancelled: &Arc<AtomicBool>,
) -> Result<Vec<GrepMatch>> {
    let effective_pattern = build_pattern(pattern, options);

    let matcher = RegexMatcherBuilder::new()
        .case_insensitive(!options.case_sensitive)
        .case_smart(false)
        .word(options.whole_word)
        .build(&effective_pattern)
        .with_context(|| format!("Invalid search pattern: {pattern}"))?;

    let mut searcher = SearcherBuilder::new()
        .line_number(true)
        .build();

    let mut results: Vec<GrepMatch> = Vec::new();

    let walker = WalkBuilder::new(project_root)
        .hidden(true)      // skip hidden files/dirs
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    'walk: for entry in walker {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }
        if results.len() >= options.max_results {
            break;
        }

        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Walk error: {e}");
                continue;
            }
        };

        // Skip directories — only process files.
        if entry.file_type().map(|ft| !ft.is_file()).unwrap_or(true) {
            continue;
        }

        let abs_path = entry.path();
        let rel_path = match abs_path.strip_prefix(project_root) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };

        let language = detect_language_from_path(&rel_path);
        if !options.scope.matches_file(&rel_path, language) {
            continue;
        }

        trace!(file = %rel_path, "searching");

        let file_matches = search_file(
            abs_path,
            &rel_path,
            &matcher,
            &mut searcher,
            options.max_results.saturating_sub(results.len()),
        );

        match file_matches {
            Ok(mut matches) => {
                let remaining = options.max_results.saturating_sub(results.len());
                matches.truncate(remaining);
                results.extend(matches);
                if results.len() >= options.max_results {
                    break 'walk;
                }
            }
            Err(e) => {
                tracing::warn!(file = %rel_path, "grep error: {e}");
            }
        }
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Convert the user's pattern into a regex string, applying escaping or
/// whole-word wrapping as needed.
fn build_pattern(pattern: &str, options: &GrepOptions) -> String {
    let base = if options.regex {
        pattern.to_owned()
    } else {
        escape_for_regex(pattern)
    };

    if options.whole_word {
        // RegexMatcherBuilder::word() applies its own \b wrapping, so we
        // do not double-wrap here.
        base
    } else {
        base
    }
}

/// Escape all regex metacharacters so a literal string can be passed to the
/// regex engine.  Avoids taking the `regex` crate as a dependency.
fn escape_for_regex(s: &str) -> String {
    // Characters that have special meaning in a Rust/PCRE-compatible regex.
    const META: &[char] = &[
        '\\', '.', '+', '*', '?', '(', ')', '|', '[', ']', '{', '}', '^', '$', '-', '#', '&',
        '~',
    ];
    let mut out = String::with_capacity(s.len() + 8);
    for ch in s.chars() {
        if META.contains(&ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Search a single file and return all matches up to `limit`.
fn search_file(
    path: &Path,
    rel_path: &str,
    matcher: &grep_regex::RegexMatcher,
    searcher: &mut Searcher,
    limit: usize,
) -> Result<Vec<GrepMatch>> {
    let mut matches: Vec<GrepMatch> = Vec::new();

    // `sinks::UTF8` calls our closure for every matched line.
    // The closure receives (line_number: u64, line_content: &str).
    let sink = UTF8(|line_number, line_content| {
        if matches.len() >= limit {
            // Returning false halts the search for this file.
            return Ok(false);
        }

        // Strip the trailing newline(s) for clean content.
        let clean = line_content.trim_end_matches(['\n', '\r']);

        // Find the byte offsets of the first match within the line.
        let (match_start, match_end) = find_match_offsets(matcher, clean.as_bytes());

        matches.push(GrepMatch {
            file_path: rel_path.to_owned(),
            line_number: line_number as u32,
            column: match_start,
            line_content: clean.to_owned(),
            match_start,
            match_end,
        });

        Ok(true)
    });

    searcher
        .search_path(matcher, path, sink)
        .with_context(|| format!("Failed to search {}", path.display()))?;

    Ok(matches)
}

/// Return the (start, end) byte offsets of the first match in `haystack`.
/// Returns (0, 0) if the matcher cannot locate a match (shouldn't happen
/// since we're inside a matched line, but we handle it gracefully).
fn find_match_offsets(
    matcher: &grep_regex::RegexMatcher,
    haystack: &[u8],
) -> (u32, u32) {
    match matcher.find(haystack) {
        Ok(Some(m)) => (m.start() as u32, m.end() as u32),
        // Ok(None): match was purely in the stripped newline bytes (rare).
        // Err(_): NoError is an uninhabited error type; this arm is unreachable.
        Ok(None) | Err(_) => (0, 0),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "grep_tests.rs"]
mod tests;
