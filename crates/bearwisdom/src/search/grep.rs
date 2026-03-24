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
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_file(dir: &TempDir, name: &str, content: &str) {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }

    fn not_cancelled() -> Arc<AtomicBool> {
        Arc::new(AtomicBool::new(false))
    }

    #[test]
    fn literal_search_finds_exact_match() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "foo.rs", "fn hello() {}\nfn world() {}\n");

        let opts = GrepOptions::default();
        let results = grep_search(dir.path(), "hello", &opts, &not_cancelled()).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].line_number, 1);
        assert_eq!(results[0].line_content, "fn hello() {}");
        assert!(results[0].line_content.contains("hello"));
        // match_start should point at 'h' in hello
        let start = results[0].match_start as usize;
        let end = results[0].match_end as usize;
        assert_eq!(&results[0].line_content[start..end], "hello");
    }

    #[test]
    fn regex_search_uses_pattern_as_regex() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "app.ts", "const x = 42;\nconst y = 99;\nlet z = 0;\n");

        let opts = GrepOptions {
            regex: true,
            ..Default::default()
        };
        let results = grep_search(dir.path(), r"const \w+ = \d+", &opts, &not_cancelled()).unwrap();

        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|m| m.line_content.starts_with("const")));
    }

    #[test]
    fn case_insensitive_search() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "readme.md", "Hello World\nhello world\nHELLO WORLD\n");

        let opts = GrepOptions {
            case_sensitive: false,
            ..Default::default()
        };
        let results = grep_search(dir.path(), "hello", &opts, &not_cancelled()).unwrap();

        assert_eq!(results.len(), 3, "Should match all three case variants");
    }

    #[test]
    fn whole_word_excludes_partial_matches() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "code.rs", "fn foo() {}\nfn foobar() {}\nlet foo_x = 1;\n");

        let opts = GrepOptions {
            whole_word: true,
            ..Default::default()
        };
        let results = grep_search(dir.path(), "foo", &opts, &not_cancelled()).unwrap();

        // "fn foo()" matches; "foobar" and "foo_x" do not (word boundary)
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].line_content.trim(), "fn foo() {}");
    }

    #[test]
    fn max_results_caps_output() {
        let dir = TempDir::new().unwrap();
        // 20 matching lines
        let content = (0..20).map(|i| format!("needle {i}\n")).collect::<String>();
        write_file(&dir, "big.txt", &content);

        let opts = GrepOptions {
            max_results: 5,
            ..Default::default()
        };
        let results = grep_search(dir.path(), "needle", &opts, &not_cancelled()).unwrap();

        assert_eq!(results.len(), 5);
    }

    #[test]
    fn cancellation_stops_search_early() {
        let dir = TempDir::new().unwrap();
        // Write many files so the walk has work to do
        for i in 0..20 {
            write_file(&dir, &format!("file{i}.txt"), "match me\n");
        }

        let cancelled = Arc::new(AtomicBool::new(true)); // already cancelled
        let opts = GrepOptions::default();
        let results = grep_search(dir.path(), "match me", &opts, &cancelled).unwrap();

        // With cancellation set from the start, zero files should be searched.
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn scope_language_filter_applied() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "lib.rs", "fn target() {}\n");
        write_file(&dir, "app.ts", "function target() {}\n");

        let opts = GrepOptions {
            scope: SearchScope::default().with_language("rust"),
            ..Default::default()
        };
        let results = grep_search(dir.path(), "target", &opts, &not_cancelled()).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].file_path, "lib.rs");
    }

    #[test]
    fn literal_metacharacters_escaped_correctly() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "query.txt", "price > (100 + 50)\nprice > 200\n");

        // Pattern contains regex metacharacters; should be treated literally.
        let opts = GrepOptions::default(); // regex: false
        let results = grep_search(dir.path(), "(100 + 50)", &opts, &not_cancelled()).unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].line_content.contains("(100 + 50)"));
    }

    #[test]
    fn match_offsets_are_correct() {
        let dir = TempDir::new().unwrap();
        write_file(&dir, "offsets.txt", "prefix NEEDLE suffix\n");

        let opts = GrepOptions::default();
        let results = grep_search(dir.path(), "NEEDLE", &opts, &not_cancelled()).unwrap();

        assert_eq!(results.len(), 1);
        let m = &results[0];
        let extracted = &m.line_content[m.match_start as usize..m.match_end as usize];
        assert_eq!(extracted, "NEEDLE");
    }
}
