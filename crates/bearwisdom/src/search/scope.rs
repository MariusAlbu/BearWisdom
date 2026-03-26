// =============================================================================
// search/scope.rs  —  search scope / filter specification
//
// SearchScope describes which files to include in a search.  Used by grep,
// FTS5 content search, and hybrid search to narrow results by language,
// directory, and glob patterns.
// =============================================================================

use serde::{Deserialize, Serialize};

/// Filters that narrow a search to a subset of files.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchScope {
    /// Include only files matching these glob patterns (if non-empty).
    pub include_globs: Vec<String>,
    /// Exclude files matching these glob patterns.
    pub exclude_globs: Vec<String>,
    /// Include only files of these languages (e.g. "csharp", "typescript").
    pub languages: Vec<String>,
    /// Restrict search to this directory (relative to project root).
    pub directory: Option<String>,
}

impl SearchScope {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_language(mut self, lang: &str) -> Self {
        self.languages.push(lang.to_string());
        self
    }

    pub fn with_directory(mut self, dir: &str) -> Self {
        self.directory = Some(dir.to_string());
        self
    }

    pub fn with_include(mut self, glob: &str) -> Self {
        self.include_globs.push(glob.to_string());
        self
    }

    pub fn with_exclude(mut self, glob: &str) -> Self {
        self.exclude_globs.push(glob.to_string());
        self
    }

    /// True when no filters are set (matches everything).
    pub fn is_empty(&self) -> bool {
        self.include_globs.is_empty()
            && self.exclude_globs.is_empty()
            && self.languages.is_empty()
            && self.directory.is_none()
    }

    /// Check whether a file at `path` (relative, forward-slash) with the given
    /// `language` tag passes all active filters.
    pub fn matches_file(&self, path: &str, language: &str) -> bool {
        let path_norm = path.replace('\\', "/");

        // Language filter
        if !self.languages.is_empty()
            && !self.languages.iter().any(|l| l.eq_ignore_ascii_case(language))
        {
            return false;
        }

        // Directory prefix filter
        if let Some(dir) = &self.directory {
            let dir_norm = dir.replace('\\', "/");
            let dir_prefix = if dir_norm.ends_with('/') {
                dir_norm
            } else {
                format!("{dir_norm}/")
            };
            if !path_norm.starts_with(&dir_prefix) && path_norm != dir_prefix.trim_end_matches('/') {
                return false;
            }
        }

        // Include globs — at least one must match (if any specified)
        if !self.include_globs.is_empty()
            && !self.include_globs.iter().any(|g| glob_match(g, &path_norm))
        {
            return false;
        }

        // Exclude globs — none must match
        if self.exclude_globs.iter().any(|g| glob_match(g, &path_norm)) {
            return false;
        }

        true
    }
}

/// Minimal glob matching supporting `*` (single segment) and `**` (any depth).
fn glob_match(pattern: &str, path: &str) -> bool {
    let pat = pattern.replace('\\', "/");
    let p = path.replace('\\', "/");

    // Handle multiple ** segments: split pattern on ** and check that all
    // fragments appear in order within the path.
    if pat.contains("**") {
        let fragments: Vec<&str> = pat
            .split("**")
            .map(|s| s.trim_matches('/'))
            .filter(|s| !s.is_empty())
            .collect();

        if fragments.is_empty() {
            // Pattern is just "**" — matches everything.
            return true;
        }

        // All non-empty fragments must appear in order in the path.
        let mut search_from = 0usize;
        for frag in &fragments {
            // Handle simple * within the fragment
            if frag.contains('*') {
                let parts: Vec<&str> = frag.splitn(2, '*').collect();
                if parts.len() == 2 {
                    if let Some(pos) = p[search_from..].find(parts[0]) {
                        let after = search_from + pos + parts[0].len();
                        if p[after..].contains(parts[1]) {
                            search_from = after;
                            continue;
                        }
                    }
                }
                return false;
            }

            match p[search_from..].find(frag) {
                Some(pos) => search_from += pos + frag.len(),
                None => return false,
            }
        }
        return true;
    }

    // * — matches within a single segment
    if pat.contains('*') {
        let parts: Vec<&str> = pat.splitn(2, '*').collect();
        if parts.len() == 2 {
            return p.starts_with(parts[0]) && p.ends_with(parts[1]);
        }
    }

    // Exact or suffix match
    p == pat || p.ends_with(&format!("/{pat}"))
}

// ---------------------------------------------------------------------------
// Language detection helper (by file extension)
// ---------------------------------------------------------------------------

/// Detect language tag from a file path's extension.
/// Returns a best-guess language string matching the `files.language` column.
pub fn detect_language_from_path(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("");
    match ext {
        "cs" => "csharp",
        "ts" => "typescript",
        "tsx" => "tsx",
        "js" => "javascript",
        "jsx" => "jsx",
        "py" => "python",
        "rs" => "rust",
        "java" => "java",
        "go" => "go",
        "rb" => "ruby",
        "php" => "php",
        "cpp" | "cc" | "cxx" => "cpp",
        "c" | "h" => "c",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "html" | "htm" => "html",
        "css" => "css",
        "json" => "json",
        "sh" | "bash" => "bash",
        "yml" | "yaml" => "yaml",
        "lua" => "lua",
        "r" | "R" => "r",
        "dart" => "dart",
        "scala" | "sc" => "scala",
        "hs" => "haskell",
        "ex" | "exs" => "elixir",
        "md" | "markdown" => "markdown",
        "xml" => "xml",
        "proto" => "protobuf",
        _ => "unknown",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "scope_tests.rs"]
mod tests;
