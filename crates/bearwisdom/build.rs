// =============================================================================
// build.rs — Extract builtin/keyword names from tree-sitter highlights.scm
//
// Auto-discovers ALL tree-sitter grammar crates in the cargo registry.
// No hardcoded grammar list — any crate matching `tree-sitter-*` with a
// `queries/highlights.scm` gets processed.
//
// Extracts:
//   1. String literals in [...] @keyword blocks
//   2. String literals in [...] @*.builtin blocks
//   3. Inline "word" @keyword / "word" @*.builtin
//   4. Names from #match?/#eq? predicates on @*.builtin captures
//
// Output layout:
//   src/indexer/query_builtins.rs           dispatcher (mod declarations + match)
//   src/indexer/query_builtins/<lang>.rs    per-language BUILTINS + LOCALS_SCM
//
// Language aliases (tsx→typescript, etc.) are NOT handled here — they live
// in the language plugin registry where they belong.
// =============================================================================

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let dispatcher_path = PathBuf::from(&manifest).join("src/indexer/query_builtins.rs");
    let per_lang_dir = PathBuf::from(&manifest).join("src/indexer/query_builtins");

    let home = std::env::var("CARGO_HOME")
        .or_else(|_| std::env::var("HOME").map(|h| format!("{h}/.cargo")))
        .or_else(|_| std::env::var("USERPROFILE").map(|h| format!("{h}/.cargo")))
        .unwrap_or_else(|_| String::from(".cargo"));
    let registry_src = PathBuf::from(&home).join("registry/src");

    let mut all_builtins: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut all_locals: BTreeMap<String, String> = BTreeMap::new();

    // Scan all index directories in the cargo registry.
    if let Ok(index_dirs) = fs::read_dir(&registry_src) {
        for index_entry in index_dirs.flatten() {
            let index_path = index_entry.path();
            if !index_path.is_dir() {
                continue;
            }
            if let Ok(crate_dirs) = fs::read_dir(&index_path) {
                for crate_entry in crate_dirs.flatten() {
                    let crate_name = crate_entry.file_name().to_string_lossy().to_string();
                    let crate_path = crate_entry.path();

                    let Some(lang_id) = extract_lang_id(&crate_name) else {
                        continue;
                    };

                    // Extract builtins from highlights.scm.
                    let highlights = crate_path.join("queries/highlights.scm");
                    if highlights.exists() {
                        if let Ok(content) = fs::read_to_string(&highlights) {
                            let names = all_builtins.entry(lang_id.clone()).or_default();
                            extract_builtins_from_scm(&content, names);
                        }
                    }

                    // Embed locals.scm content for scope resolution.
                    let locals = crate_path.join("queries/locals.scm");
                    if locals.exists() {
                        if let Ok(content) = fs::read_to_string(&locals) {
                            if !content.trim().is_empty() {
                                all_locals.entry(lang_id).or_insert(content);
                            }
                        }
                    }
                }
            }
        }
    }

    // Filter builtins (length > 1, starts with alphanumeric or underscore).
    let mut filtered_builtins: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (lang, names) in all_builtins {
        let filtered: Vec<String> = names
            .into_iter()
            .filter(|n| n.len() > 1 && n.starts_with(|c: char| c.is_alphanumeric() || c == '_'))
            .collect();
        if !filtered.is_empty() {
            filtered_builtins.insert(lang, filtered);
        }
    }

    // Union of langs that get a per-language file.
    let mut all_langs: BTreeSet<String> = BTreeSet::new();
    all_langs.extend(filtered_builtins.keys().cloned());
    all_langs.extend(all_locals.keys().cloned());

    // Recreate the per-language directory so stale files from previous builds disappear.
    let _ = fs::remove_dir_all(&per_lang_dir);
    fs::create_dir_all(&per_lang_dir).expect("Failed to create query_builtins dir");

    for lang in &all_langs {
        write_lang_file(
            &per_lang_dir,
            lang,
            filtered_builtins.get(lang),
            all_locals.get(lang),
        );
    }

    write_dispatcher(&dispatcher_path, &all_langs, &filtered_builtins, &all_locals);

    // Rerun only when build.rs itself changes. Grammar query files in the
    // cargo registry are stable per-version — they only change when crate
    // versions are bumped in Cargo.toml.
    println!("cargo:rerun-if-changed=build.rs");
}

/// Write a per-language file containing `BUILTINS` and `LOCALS_SCM` consts.
fn write_lang_file(
    dir: &Path,
    lang: &str,
    builtins: Option<&Vec<String>>,
    locals: Option<&String>,
) {
    let mut out = String::new();
    out.push_str("// AUTO-GENERATED by build.rs — do not edit manually.\n");
    out.push_str(&format!(
        "// Builtins and locals.scm content for `{lang}` extracted from tree-sitter grammar.\n\n"
    ));

    out.push_str("pub const BUILTINS: &[&str] = &[\n");
    if let Some(names) = builtins {
        for name in names {
            out.push_str(&format!("    \"{}\",\n", name.replace('"', "\\\"")));
        }
    }
    out.push_str("];\n\n");

    out.push_str("pub const LOCALS_SCM: Option<&str> = ");
    if let Some(scm) = locals {
        let escaped = scm
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n\\\n");
        out.push_str(&format!("Some(\"{escaped}\")"));
    } else {
        out.push_str("None");
    }
    out.push_str(";\n");

    let path = dir.join(format!("{lang}.rs"));
    fs::write(&path, &out).expect("Failed to write per-language file");
}

/// Write the dispatcher: mod declarations + two match-based lookup functions.
fn write_dispatcher(
    path: &Path,
    all_langs: &BTreeSet<String>,
    builtins: &BTreeMap<String, Vec<String>>,
    locals: &BTreeMap<String, String>,
) {
    let mut out = String::new();
    out.push_str(
        "// AUTO-GENERATED by build.rs — do not edit manually.\n\
         // Dispatcher over per-language data in `query_builtins/<lang>.rs`.\n\
         //\n\
         // Language IDs are derived from crate names: tree-sitter-foo-1.2.3 → \"foo\".\n\
         // Aliases (tsx→typescript, etc.) are handled by the language plugin registry,\n\
         // NOT here — build.rs only knows about crate names.\n\n",
    );

    for lang in all_langs {
        out.push_str(&format!("mod {lang};\n"));
    }
    out.push('\n');

    out.push_str(
        "/// Return query-extracted builtins for a language.\n\
         /// Returns an empty slice for languages without query files.\n\
         pub fn query_builtins_for_language(lang: &str) -> &'static [&'static str] {\n\
         \x20   match lang {\n",
    );
    for lang in builtins.keys() {
        out.push_str(&format!("        \"{lang}\" => {lang}::BUILTINS,\n"));
    }
    out.push_str("        _ => &[],\n    }\n}\n\n");

    out.push_str(
        "/// Return the locals.scm query string for a language, if available.\n\
         /// Used by LocalResolver for scope-based local variable resolution.\n\
         pub fn locals_scm_for_language(lang: &str) -> Option<&'static str> {\n\
         \x20   match lang {\n",
    );
    for lang in locals.keys() {
        out.push_str(&format!("        \"{lang}\" => {lang}::LOCALS_SCM,\n"));
    }
    out.push_str("        _ => None,\n    }\n}\n");

    fs::write(path, &out).expect("Failed to write dispatcher");
}

/// Derive a BearWisdom language ID from a tree-sitter crate directory name.
///
/// Examples:
///   "tree-sitter-rust-0.24.0"     → Some("rust")
///   "tree-sitter-c-sharp-0.23.1"  → Some("c-sharp")
///   "tree-sitter-c-0.24.1"        → Some("c")
///   "tree-sitter-go-0.25.0"       → Some("go")
///   "tree-sitter-kotlin-ng-0.1.0" → Some("kotlin-ng")
///   "not-a-grammar"               → None
fn extract_lang_id(crate_dir_name: &str) -> Option<String> {
    let rest = crate_dir_name.strip_prefix("tree-sitter-")?;

    // Strip the version suffix: find the last `-DIGIT` boundary.
    // Walk from the end to find where the version starts.
    // Version is always `-N.N.N` at the end.
    let mut version_start = None;
    let bytes = rest.as_bytes();
    for i in (1..bytes.len()).rev() {
        if bytes[i - 1] == b'-' && bytes[i].is_ascii_digit() {
            // Check this looks like a version: digits, dots, possibly more digits.
            let tail = &rest[i..];
            if tail
                .chars()
                .all(|c| c.is_ascii_digit() || c == '.' || c == '-')
            {
                version_start = Some(i - 1);
                break;
            }
        }
    }

    let lang_part = match version_start {
        Some(pos) => &rest[..pos],
        None => rest,
    };

    if lang_part.is_empty() {
        return None;
    }

    // Normalize crate naming conventions to BearWisdom language IDs.
    // This is NOT an alias system — it's purely fixing crate naming mismatches.
    let normalized = match lang_part {
        "c-sharp" => "csharp",
        "vb-dotnet" => "vbnet",
        "kotlin-ng" => "kotlin",
        "sequel" => "sql",
        "scss-local" => "scss",
        "dockerfile-0-25" => "dockerfile",
        "md" => "markdown",
        other => other,
    };

    Some(normalized.to_string())
}

// ---------------------------------------------------------------------------
// SCM extraction
// ---------------------------------------------------------------------------

fn extract_builtins_from_scm(content: &str, names: &mut BTreeSet<String>) {
    extract_match_predicates(content, names);
    extract_eq_predicates(content, names);
    extract_keyword_strings(content, names);
    extract_builtin_strings(content, names);
}

fn extract_match_predicates(content: &str, names: &mut BTreeSet<String>) {
    let re = regex::Regex::new(
        r#"#match\?\s+@\w+(?:\.\w+)?\s+"[^^]*\^?\(([^)]+)\)\$?""#,
    )
    .unwrap();
    for cap in re.captures_iter(content) {
        if let Some(alt) = cap.get(1) {
            for name in alt.as_str().split('|') {
                let name = name.trim();
                if !name.is_empty() {
                    names.insert(name.to_string());
                }
            }
        }
    }
}

fn extract_eq_predicates(content: &str, names: &mut BTreeSet<String>) {
    let re = regex::Regex::new(r#"#eq\?\s+@\w+(?:\.\w+)?\s+"([^"]+)""#).unwrap();
    for cap in re.captures_iter(content) {
        if let Some(name) = cap.get(1) {
            names.insert(name.as_str().to_string());
        }
    }
}

fn extract_keyword_strings(content: &str, names: &mut BTreeSet<String>) {
    // Inline: "word" @keyword
    let re = regex::Regex::new(
        r#""([a-zA-Z_][a-zA-Z0-9_!?]*(?:::)?[a-zA-Z0-9_!?]*)"\s+@keyword"#,
    )
    .unwrap();
    for cap in re.captures_iter(content) {
        if let Some(name) = cap.get(1) {
            names.insert(name.as_str().to_string());
        }
    }
    // Bracket blocks: [...] @keyword
    extract_bracket_block_names(content, "@keyword", names);
}

fn extract_builtin_strings(content: &str, names: &mut BTreeSet<String>) {
    // Inline: "word" @*.builtin
    let re = regex::Regex::new(r#""([a-zA-Z_][a-zA-Z0-9_!?]*)"\s+@\w+\.builtin"#).unwrap();
    for cap in re.captures_iter(content) {
        if let Some(name) = cap.get(1) {
            names.insert(name.as_str().to_string());
        }
    }
    // Bracket blocks: [...] @type.builtin, [...] @constant.builtin, etc.
    for tag in [
        "@type.builtin",
        "@constant.builtin",
        "@function.builtin",
        "@variable.builtin",
    ] {
        extract_bracket_block_names(content, tag, names);
    }
}

/// Extract quoted names from `[ "w1" "w2" ... ] @tag` blocks.
fn extract_bracket_block_names(content: &str, tag: &str, names: &mut BTreeSet<String>) {
    let re_quoted = regex::Regex::new(r#""([a-zA-Z_][a-zA-Z0-9_!?]*)""#).unwrap();
    let needle = format!("] {}", tag);
    let mut pos = 0;
    while let Some(found) = content[pos..].find(&needle) {
        let abs = pos + found;
        if let Some(open) = content[..abs].rfind('[') {
            for cap in re_quoted.captures_iter(&content[open..abs]) {
                if let Some(name) = cap.get(1) {
                    names.insert(name.as_str().to_string());
                }
            }
        }
        pos = abs + needle.len();
    }
}
