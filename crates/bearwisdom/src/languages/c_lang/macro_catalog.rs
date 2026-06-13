// =============================================================================
// c_lang/macro_catalog.rs — generic C/C++ macro discovery
//
// Walks the headers near a translation unit (sibling directory + one parent
// directory) and parses `#define NAME[(args)] body` directives. Tree-sitter
// doesn't preprocess, so when a project's macro expands to real declarations
// (Clay's `CLAY__ARRAY_DEFINE`, nginx's `ngx_cdecl`, FreeBSD's `__printflike`,
// any project's helper macros), the extractor sees only an opaque invocation
// and the real symbol stays unresolved.
//
// The catalog is the source-of-truth for what each macro expands to; the
// salvage pass in `extract.rs` re-runs declaration scanners over the
// substituted body. No project-specific names are baked into BW: every macro
// is read from the project's own headers.
//
// Per-directory caching keeps the cost bounded for projects with many files
// in the same directory — the first file in `src/core/` populates the
// catalog, subsequent files reuse it via `Arc` clone.
// =============================================================================

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

#[derive(Clone, Debug)]
pub(crate) struct MacroDef {
    pub args: Vec<String>,
    pub body: String,
}

#[derive(Default, Clone, Debug)]
pub(crate) struct MacroCatalog {
    pub by_name: HashMap<String, MacroDef>,
}

impl MacroCatalog {
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// True when `name` is a defined macro whose body is attribute / linkage /
    /// storage-class shaped (or empty). Such a macro in leading type position is
    /// a qualifier, never the declared type, so a TypeRef carrying its name is
    /// always unresolvable and is dropped.
    ///
    /// A macro whose body names a real type (`#define MyInt int`, `#define
    /// Handle void*`) is NOT attribute-shaped: it legitimately aliases a type,
    /// so its TypeRef must survive.
    pub fn is_attribute_macro(&self, name: &str) -> bool {
        self.by_name
            .get(name)
            .is_some_and(|def| is_attribute_shaped_body(&def.body))
    }

    /// Record one `#define`. The same name often carries several bodies across
    /// conditional branches (`#if WIN32 … #else …`); only one survives the real
    /// preprocessor, but the catalog can't preprocess. When any branch is
    /// attribute-shaped, keep that body — a qualifier macro must read as a
    /// qualifier regardless of which branch the walk encountered first.
    /// Otherwise first definition wins.
    fn merge_define(&mut self, name: String, def: MacroDef) {
        match self.by_name.entry(name) {
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(def);
            }
            std::collections::hash_map::Entry::Occupied(mut slot) => {
                if !is_attribute_shaped_body(&slot.get().body)
                    && is_attribute_shaped_body(&def.body)
                {
                    slot.insert(def);
                }
            }
        }
    }
}

/// Classify a `#define` body as attribute / linkage / storage-shaped. Returns
/// `true` for bodies that consist solely of attribute, linkage, calling-
/// convention, or storage-class tokens (or that are empty) — the shapes a
/// qualifier macro expands to. Returns `false` the moment a token could name a
/// type, so type-aliasing macros are preserved.
///
/// Empty body → `true`: `#define PERL_CALLCONV` is the dominant qualifier form.
fn is_attribute_shaped_body(body: &str) -> bool {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return true;
    }
    // Scan token-by-token. Every token must be attribute-shaped; a single
    // type-naming token disqualifies the whole body.
    let mut saw_token = false;
    for tok in tokenize_body(trimmed) {
        saw_token = true;
        if !is_attribute_token(&tok) {
            return false;
        }
    }
    saw_token
}

/// Split a macro body into identifier-or-symbol tokens. Identifiers (incl. a
/// leading `__`) group together; parenthesised attribute arguments
/// (`__attribute__((x))`, `__declspec(y)`) are consumed as part of the
/// preceding token so their inner words don't leak out as bare identifiers.
fn tokenize_body(body: &str) -> Vec<String> {
    let bytes = body.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if is_ident_byte(b) {
            let start = i;
            while i < bytes.len() && is_ident_byte(bytes[i]) {
                i += 1;
            }
            let mut tok = body[start..i].to_string();
            // Swallow a directly-following `(...)` group (attribute args).
            while i < bytes.len() && bytes[i] == b'(' {
                let mut depth = 0usize;
                let grp_start = i;
                while i < bytes.len() {
                    match bytes[i] {
                        b'(' => depth += 1,
                        b')' => {
                            depth -= 1;
                            i += 1;
                            if depth == 0 {
                                break;
                            }
                            continue;
                        }
                        _ => {}
                    }
                    i += 1;
                }
                tok.push_str(&body[grp_start..i]);
            }
            out.push(tok);
        } else if b == b'"' {
            // A string literal is one token (`extern "C"` carries `"C"`), so
            // its contents never leak out as bare identifiers.
            let start = i;
            i += 1;
            while i < bytes.len() {
                match bytes[i] {
                    b'\\' => i += 2,
                    b'"' => {
                        i += 1;
                        break;
                    }
                    _ => i += 1,
                }
            }
            out.push(body[start..i.min(body.len())].to_string());
        } else {
            // Lone punctuation (`*`, `&`, `(`) — record as its own token so the
            // classifier can reject pointer/reference type aliases.
            out.push((b as char).to_string());
            i += 1;
        }
    }
    out
}

/// True when a single body token is an attribute / linkage / storage / calling-
/// convention specifier — never a type name. Recognises, in order:
///   * String literals — `extern "C"` carries a `"C"` token.
///   * Attribute carriers `__attribute__(...)` / `__declspec(...)`, with their
///     `(...)` already swallowed by the tokenizer.
///   * The C storage / linkage / qualifier keywords (`extern`, `static`, …).
///   * Reserved-identifier shapes (`__`-prefixed, or `_` + uppercase) — the
///     standard reserves these for the implementation, so a leading-position
///     token of this shape is a compiler intrinsic / convention specifier, not
///     a user type (`__cdecl`, `__stdcall`, `_System`).
///
/// Anything else — an ordinary or SCREAMING_CASE identifier, a pointer/
/// reference symbol — returns `false`, so a type-aliasing body survives.
fn is_attribute_token(tok: &str) -> bool {
    let t = tok.trim();
    if t.is_empty() {
        return true;
    }
    if t.starts_with('"') {
        return true;
    }
    for carrier in ["__attribute__", "__attribute", "__declspec"] {
        if t == carrier || t.starts_with(&format!("{carrier}(")) {
            return true;
        }
    }
    if matches!(
        t,
        "extern" | "static" | "inline" | "register" | "const" | "volatile"
    ) {
        return true;
    }
    is_reserved_identifier(t)
}

/// True for identifiers the C/C++ standard reserves for the implementation:
/// a `__` prefix anywhere, or a leading `_` followed by an uppercase letter.
/// User type names never take these shapes, so a body composed only of such
/// tokens is a compiler-convention specifier, not a type alias.
fn is_reserved_identifier(t: &str) -> bool {
    let bytes = t.as_bytes();
    if bytes.first() != Some(&b'_') {
        return false;
    }
    bytes.get(1) == Some(&b'_') || bytes.get(1).is_some_and(|b| b.is_ascii_uppercase())
}

/// Discover macros visible to a translation unit, with a directory-level
/// cache. The lookup walks (a) the file's own directory and (b) one parent
/// directory's `include/` and `inc/` siblings — the conventional shapes for
/// projects that split sources from headers (`src/foo.c` + `include/foo.h`).
///
/// `file_path` may be relative (the indexer passes paths relative to the
/// project root, which it sets as the process working directory before
/// indexing). We resolve via `current_dir().join(file_path)` so the
/// directory walk hits real on-disk headers regardless of what shape the
/// path takes.
pub(crate) fn catalog_for_file(file_path: &str) -> Arc<MacroCatalog> {
    // In a real index session the project root is known: build one bounded
    // project-wide catalog so macros defined in deep include trees (nginx
    // `src/core`/`src/os`, redis, curl) reach every translation unit, not just
    // the file's own directory. Tests / direct callers (no session) fall back
    // to the per-directory walk below.
    if let Some(root) = current_project_root() {
        return project_macro_catalog(&root);
    }
    let Some(dir) = resolve_file_dir(file_path) else {
        return Arc::new(MacroCatalog::default());
    };
    let cache = global_cache();
    if let Ok(read) = cache.read() {
        if let Some(c) = read.get(&dir) {
            return c.clone();
        }
    }
    let catalog = Arc::new(build_catalog(&dir));
    if let Ok(mut write) = cache.write() {
        write.entry(dir).or_insert_with(|| catalog.clone());
    }
    catalog
}

fn resolve_file_dir(file_path: &str) -> Option<PathBuf> {
    if file_path.is_empty() {
        return None;
    }
    let path = PathBuf::from(file_path);
    let resolved: PathBuf = if path.is_absolute() {
        path
    } else if let Some(root) = current_project_root() {
        root.join(&path)
    } else {
        // No active indexing session — fall back to cwd so unit tests
        // and direct callers still work when they invoke the extractor
        // from a working directory rooted at the project.
        let cwd = std::env::current_dir().ok()?;
        cwd.join(&path)
    };
    let parent = resolved.parent()?;
    if parent.is_dir() {
        Some(parent.to_path_buf())
    } else {
        None
    }
}

fn global_cache() -> &'static RwLock<HashMap<PathBuf, Arc<MacroCatalog>>> {
    static CACHE: OnceLock<RwLock<HashMap<PathBuf, Arc<MacroCatalog>>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn project_root_lock() -> &'static RwLock<Option<PathBuf>> {
    static ROOT: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();
    ROOT.get_or_init(|| RwLock::new(None))
}

/// Set the active indexing project root. The indexer calls this once at
/// the start of a full or incremental index pass so the per-file extractor
/// can resolve relative paths (`src/foo.c`) to absolute on-disk paths
/// without changing the language-plugin trait signature.
///
/// Also clears the per-directory catalog cache — switching projects
/// invalidates everything.
pub fn begin_index_session(root: &std::path::Path) {
    if let Ok(mut w) = project_root_lock().write() {
        *w = Some(root.to_path_buf());
    }
    if let Ok(mut w) = global_cache().write() {
        w.clear();
    }
    if let Ok(mut w) = project_cache().write() {
        w.clear();
    }
}

/// Clear the indexing-session state. Called at the end of an index pass.
pub fn end_index_session() {
    if let Ok(mut w) = project_root_lock().write() {
        *w = None;
    }
}

fn current_project_root() -> Option<PathBuf> {
    project_root_lock().read().ok().and_then(|r| r.clone())
}

/// Upper bound on headers parsed for the project-wide catalog. Caps the cost
/// on trees that vendor a full libc/SDK (zig-compiler, perl). Real C/C++ apps
/// have far fewer headers; the bound only bites on vendored monsters.
const MAX_PROJECT_HEADERS: usize = 20_000;

fn project_cache() -> &'static RwLock<HashMap<PathBuf, Arc<MacroCatalog>>> {
    static CACHE: OnceLock<RwLock<HashMap<PathBuf, Arc<MacroCatalog>>>> = OnceLock::new();
    CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// One bounded `#define` catalog over every header under the project root,
/// cached per root for the index session. Recovers macro-defined callees whose
/// definition lives outside the translation unit's own directory subtree.
fn project_macro_catalog(root: &Path) -> Arc<MacroCatalog> {
    if let Ok(read) = project_cache().read() {
        if let Some(c) = read.get(root) {
            return c.clone();
        }
    }
    let mut headers: Vec<PathBuf> = Vec::new();
    collect_headers_recursive(root, &mut headers, MAX_PROJECT_HEADERS);
    let mut catalog = MacroCatalog::default();
    for h in headers {
        let Ok(content) = std::fs::read_to_string(&h) else {
            continue;
        };
        for (name, def) in parse_defines(&content) {
            catalog.merge_define(name, def);
        }
    }
    let arc = Arc::new(catalog);
    if let Ok(mut w) = project_cache().write() {
        w.entry(root.to_path_buf()).or_insert_with(|| arc.clone());
    }
    arc
}

/// Depth-first header collection bounded by `cap`. Does not follow directory
/// symlinks (`entry.file_type()` reports the link, not its target), so symlink
/// loops cannot occur.
fn collect_headers_recursive(dir: &Path, out: &mut Vec<PathBuf>, cap: usize) {
    if out.len() >= cap {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= cap {
            return;
        }
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_dir() {
            collect_headers_recursive(&path, out, cap);
        } else if ft.is_file() {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let lower = name.to_ascii_lowercase();
            if lower.ends_with(".h")
                || lower.ends_with(".hpp")
                || lower.ends_with(".hxx")
                || lower.ends_with(".hh")
                || lower.ends_with(".inl")
            {
                out.push(path);
            }
        }
    }
}

/// Reset all session state. Used by tests to ensure a clean catalog per test:
/// clears the per-directory cache, the project-wide cache, and the active
/// project root. Without clearing the root, an earlier indexer test that left a
/// session open would route `catalog_for_file` to a stale `project_macro_catalog`
/// instead of the per-directory walk the test fixture relies on.
#[cfg(test)]
pub(crate) fn _reset_cache_for_test() {
    if let Ok(mut write) = global_cache().write() {
        write.clear();
    }
    if let Ok(mut write) = project_cache().write() {
        write.clear();
    }
    if let Ok(mut write) = project_root_lock().write() {
        *write = None;
    }
}

/// Build a catalog directly from one source string (the `#define` parser only),
/// bypassing the on-disk header walk. Lets tests assert which names `#define`
/// shapes capture and how their bodies classify.
#[cfg(test)]
pub(crate) fn _catalog_from_source(source: &str) -> MacroCatalog {
    let mut catalog = MacroCatalog::default();
    for (name, def) in parse_defines(source) {
        catalog.merge_define(name, def);
    }
    catalog
}

#[cfg(test)]
pub(crate) fn _is_attribute_shaped_body(body: &str) -> bool {
    is_attribute_shaped_body(body)
}

fn build_catalog(dir: &Path) -> MacroCatalog {
    let mut header_files: Vec<PathBuf> = Vec::new();
    collect_header_files(dir, &mut header_files);

    // Common split-layout: `src/foo.c` + `../include/foo.h`. Walk one
    // parent's `include/` and `inc/` if present.
    if let Some(parent) = dir.parent() {
        for sibling in ["include", "inc"] {
            collect_header_files(&parent.join(sibling), &mut header_files);
        }
    }

    let mut catalog = MacroCatalog::default();
    for h in header_files {
        let Ok(content) = std::fs::read_to_string(&h) else {
            continue;
        };
        for (name, def) in parse_defines(&content) {
            catalog.merge_define(name, def);
        }
    }
    catalog
}

fn collect_header_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_file() {
            continue;
        }
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".h")
            || lower.ends_with(".hpp")
            || lower.ends_with(".hxx")
            || lower.ends_with(".hh")
            || lower.ends_with(".inl")
        {
            out.push(path);
        }
    }
}

// ---------------------------------------------------------------------------
// `#define` parsing
// ---------------------------------------------------------------------------
//
// Recognises the two #define shapes:
//   * Function-like: `#define NAME(arg1, arg2, ...) body`
//   * Object-like:   `#define NAME body`
//
// Multi-line continuations (`\` at end of line) are joined into a single
// logical body. Comments are stripped because they show up in macro bodies
// and would confuse the substitution.

fn parse_defines(source: &str) -> Vec<(String, MacroDef)> {
    let logical = join_continuations(source);
    let mut out = Vec::new();
    for line in logical.lines() {
        let trimmed = line.trim_start();
        // The directive is `#` then `define`, but the standard permits
        // whitespace between them (`#   define NAME`, used by code that indents
        // directives by conditional-nesting depth). Strip the `#`, then any
        // intervening whitespace, before matching the keyword.
        let Some(after_hash) = trimmed.strip_prefix('#') else {
            continue;
        };
        let Some(rest) = after_hash.trim_start().strip_prefix("define") else {
            continue;
        };
        // Require a separator after `define` so `#defined` / `#definition`
        // don't match.
        if !rest.starts_with(|c: char| c.is_whitespace()) {
            continue;
        }
        let rest = rest.trim_start();
        if rest.is_empty() {
            continue;
        }

        // Read identifier.
        let name_end = rest.find(|c: char| !is_ident_char(c)).unwrap_or(rest.len());
        if name_end == 0 {
            continue;
        }
        let name = rest[..name_end].to_string();
        let after_name = &rest[name_end..];

        // Function-like macro requires `(` IMMEDIATELY after name (no space).
        let (args, body_start) = if after_name.starts_with('(') {
            let Some((args_str, end)) = collect_balanced_parens(after_name, 0) else {
                continue;
            };
            let args: Vec<String> = args_str
                .split(',')
                .map(|a| a.trim().to_string())
                .filter(|a| !a.is_empty())
                .collect();
            (args, end)
        } else {
            (Vec::new(), 0)
        };

        let body = strip_comments(after_name[body_start..].trim())
            .trim()
            .to_string();
        out.push((name, MacroDef { args, body }));
    }
    out
}

fn join_continuations(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            // Line continuation: `\<EOL>` collapses into a single space.
            // Trailing whitespace before `\` is preserved.
            match chars.peek() {
                Some('\n') => {
                    chars.next();
                    out.push(' ');
                    continue;
                }
                Some('\r') => {
                    chars.next();
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    out.push(' ');
                    continue;
                }
                _ => {}
            }
        }
        out.push(ch);
    }
    out
}

fn strip_comments(s: &str) -> String {
    // Strip `// ...` to end-of-line and `/* ... */` block comments. We don't
    // honour string literals because macro bodies rarely contain `//`-bearing
    // strings, and treating them naively is good enough for the discovery
    // pass (worst case: a macro is missed, falling back to honest unresolved).
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            // skip until end-of-string (we already collapsed newlines)
            break;
        }
        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            if i + 1 < bytes.len() {
                i += 2;
            }
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Walk `bytes` from `open_idx` (which must point at `(`) and return the
/// substring inside the matching `)` plus the index just past it.
fn collect_balanced_parens(s: &str, open_idx: usize) -> Option<(String, usize)> {
    let bytes = s.as_bytes();
    if bytes.get(open_idx).copied() != Some(b'(') {
        return None;
    }
    let mut depth = 1usize;
    let args_start = open_idx + 1;
    let mut i = args_start;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((s[args_start..i].to_string(), i + 1));
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// Expansion
// ---------------------------------------------------------------------------

/// Expand a function-like macro invocation. Substitutes each argument's
/// occurrences in the body. Token-paste (`##`) and stringify (`#`) are
/// honoured: `arg##suffix` becomes `<argvalue>suffix`, `#arg` becomes
/// `"<argvalue>"`. Returns the expanded text.
pub(crate) fn expand(def: &MacroDef, args: &[&str]) -> String {
    if def.args.is_empty() {
        return def.body.clone();
    }
    if args.len() != def.args.len() {
        // Variadic and arity mismatches aren't supported in this minimal
        // expander; skip rather than emit garbage symbols.
        return String::new();
    }
    let bytes = def.body.as_bytes();
    let mut out = String::with_capacity(def.body.len() * 2);
    let mut i = 0;
    while i < bytes.len() {
        // `##` token paste — strip surrounding whitespace.
        if i + 1 < bytes.len() && bytes[i] == b'#' && bytes[i + 1] == b'#' {
            // Drop whitespace already pushed onto out.
            while out.ends_with(' ') || out.ends_with('\t') {
                out.pop();
            }
            i += 2;
            while i < bytes.len() && (bytes[i] == b' ' || bytes[i] == b'\t') {
                i += 1;
            }
            continue;
        }
        // `#arg` stringify — only when followed by an argument identifier.
        if bytes[i] == b'#' {
            let mut j = i + 1;
            while j < bytes.len() && is_ident_byte(bytes[j]) {
                j += 1;
            }
            if j > i + 1 {
                let ident = &def.body[i + 1..j];
                if let Some(idx) = def.args.iter().position(|a| a == ident) {
                    out.push('"');
                    out.push_str(args[idx]);
                    out.push('"');
                    i = j;
                    continue;
                }
            }
        }
        // Identifier substitution.
        if is_ident_byte(bytes[i]) && (i == 0 || !is_ident_byte(bytes[i - 1])) {
            let mut j = i + 1;
            while j < bytes.len() && is_ident_byte(bytes[j]) {
                j += 1;
            }
            let ident = &def.body[i..j];
            if let Some(idx) = def.args.iter().position(|a| a == ident) {
                out.push_str(args[idx]);
                i = j;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}
