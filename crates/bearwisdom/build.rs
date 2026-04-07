// =============================================================================
// build.rs — Extract builtin/keyword names from tree-sitter query files
//
// Scans highlights.scm and locals.scm from tree-sitter grammar crates in the
// cargo registry. Extracts:
//   1. String literals matched with @keyword (e.g., "async" @keyword)
//   2. Names from #match?/#eq? predicates on @*.builtin captures
//   3. String literals in [...] @keyword blocks
//
// Generates `src/indexer/query_builtins.rs` with per-language builtin arrays
// that are used alongside the handcrafted primitives.rs files.
// =============================================================================

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let out_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_path = PathBuf::from(&out_dir).join("src/indexer/query_builtins.rs");

    // Find the cargo registry source directory.
    let home = std::env::var("CARGO_HOME")
        .or_else(|_| std::env::var("HOME").map(|h| format!("{h}/.cargo")))
        .or_else(|_| std::env::var("USERPROFILE").map(|h| format!("{h}/.cargo")))
        .unwrap_or_else(|_| String::from(".cargo"));
    let registry_src = PathBuf::from(&home).join("registry/src");

    // Map of tree-sitter crate prefix -> BearWisdom language ID.
    let lang_map: Vec<(&str, &str)> = vec![
        ("tree-sitter-ada-", "ada"),
        ("tree-sitter-bash-", "bash"),
        ("tree-sitter-bicep-", "bicep"),
        ("tree-sitter-c-sharp-", "csharp"),
        ("tree-sitter-c-0.", "c"),
        ("tree-sitter-cmake-", "cmake"),
        ("tree-sitter-cpp-", "cpp"),
        ("tree-sitter-css-", "css"),
        ("tree-sitter-dart-", "dart"),
        ("tree-sitter-dockerfile-", "dockerfile"),
        ("tree-sitter-elixir-", "elixir"),
        ("tree-sitter-erlang-", "erlang"),
        ("tree-sitter-fortran-", "fortran"),
        ("tree-sitter-fsharp-", "fsharp"),
        ("tree-sitter-gleam-", "gleam"),
        ("tree-sitter-go-0.", "go"),
        ("tree-sitter-haskell-", "haskell"),
        ("tree-sitter-html-", "html"),
        ("tree-sitter-java-", "java"),
        ("tree-sitter-javascript-", "javascript"),
        ("tree-sitter-json-", "json"),
        ("tree-sitter-kotlin-", "kotlin"),
        ("tree-sitter-lua-", "lua"),
        ("tree-sitter-make-", "make"),
        ("tree-sitter-nix-", "nix"),
        ("tree-sitter-ocaml-", "ocaml"),
        ("tree-sitter-odin-", "odin"),
        ("tree-sitter-pascal-", "pascal"),
        ("tree-sitter-php-", "php"),
        ("tree-sitter-powershell-", "powershell"),
        ("tree-sitter-prisma-", "prisma"),
        ("tree-sitter-proto-", "proto"),
        ("tree-sitter-puppet-", "puppet"),
        ("tree-sitter-python-", "python"),
        ("tree-sitter-r-", "r"),
        ("tree-sitter-ruby-", "ruby"),
        ("tree-sitter-rust-", "rust"),
        ("tree-sitter-scala-", "scala"),
        ("tree-sitter-scss-", "scss"),
        ("tree-sitter-sequel-", "sql"),
        ("tree-sitter-starlark-", "starlark"),
        ("tree-sitter-swift-", "swift"),
        ("tree-sitter-toml-", "toml"),
        ("tree-sitter-typescript-", "typescript"),
        ("tree-sitter-yaml-", "yaml"),
        ("tree-sitter-zig-", "zig"),
    ];

    let mut all_builtins: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    // Scan all index directories in the registry.
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

                    for &(prefix, lang_id) in &lang_map {
                        if !crate_name.starts_with(prefix) {
                            continue;
                        }

                        let names = all_builtins
                            .entry(lang_id.to_string())
                            .or_default();

                        // Process highlights.scm
                        let highlights = crate_path.join("queries/highlights.scm");
                        if highlights.exists() {
                            if let Ok(content) = fs::read_to_string(&highlights) {
                                extract_builtins_from_scm(&content, names);
                            }
                        }

                        // Process locals.scm
                        let locals = crate_path.join("queries/locals.scm");
                        if locals.exists() {
                            if let Ok(content) = fs::read_to_string(&locals) {
                                extract_builtins_from_scm(&content, names);
                            }
                        }

                        break;
                    }
                }
            }
        }
    }

    // Generate the Rust source file.
    let mut output = String::new();
    output.push_str("// AUTO-GENERATED by build.rs — do not edit manually.\n");
    output.push_str("// Extracted from tree-sitter grammar query files (highlights.scm + locals.scm).\n");
    output.push_str("//\n");
    output.push_str("// These names are classified as builtins/keywords when they appear as\n");
    output.push_str("// unresolved references during resolution.\n\n");

    output.push_str("/// Return query-extracted builtins for a language.\n");
    output.push_str("/// Returns an empty slice for languages without query files.\n");
    output.push_str("pub fn query_builtins_for_language(lang: &str) -> &'static [&'static str] {\n");
    output.push_str("    match lang {\n");

    // Also generate aliases.
    let aliases: Vec<(&str, &str)> = vec![
        ("tsx", "typescript"),
        ("jsx", "javascript"),
        ("svelte", "typescript"),
        ("astro", "typescript"),
        ("vue", "typescript"),
        ("angular", "typescript"),
        ("shell", "bash"),
        ("sh", "bash"),
        ("zsh", "bash"),
        ("terraform", "hcl"),
        ("objectpascal", "pascal"),
        ("delphi", "pascal"),
        ("sass", "scss"),
        ("reason", "ocaml"),
    ];

    for (lang, names) in &all_builtins {
        if names.is_empty() {
            continue;
        }
        output.push_str(&format!("        \"{}\" => &[\n", lang));
        for name in names {
            // Skip very short names (single char) and operators.
            if name.len() <= 1 || name.starts_with(|c: char| !c.is_alphanumeric() && c != '_') {
                continue;
            }
            output.push_str(&format!("            \"{}\",\n", name.replace('"', "\\\"")));
        }
        output.push_str("        ],\n");
    }

    // Add aliases.
    for (alias, target) in &aliases {
        if all_builtins.contains_key(*target) {
            output.push_str(&format!(
                "        \"{}\" => query_builtins_for_language(\"{}\"),\n",
                alias, target
            ));
        }
    }

    output.push_str("        _ => &[],\n");
    output.push_str("    }\n");
    output.push_str("}\n");

    fs::write(&out_path, &output).expect("Failed to write query_builtins.rs");

    // Tell cargo to rerun only if build.rs changes (query files are stable).
    println!("cargo:rerun-if-changed=build.rs");
}

/// Extract builtin and keyword names from a .scm query file.
fn extract_builtins_from_scm(content: &str, names: &mut BTreeSet<String>) {
    // Pattern 1: "#match?" predicates with regex alternation.
    // e.g., (#match? @function.builtin "^(abs|all|any|...)$")
    extract_match_predicates(content, names);

    // Pattern 2: "#eq?" predicates.
    // e.g., (#eq? @function.builtin "require")
    extract_eq_predicates(content, names);

    // Pattern 3: Quoted string literals tagged with @keyword.
    // e.g., "async" @keyword
    extract_keyword_strings(content, names);

    // Pattern 4: Quoted string literals tagged with @*.builtin.
    // e.g., "nil" @constant.builtin
    extract_builtin_strings(content, names);
}

fn extract_match_predicates(content: &str, names: &mut BTreeSet<String>) {
    // Find #match? predicates targeting @*.builtin captures.
    // The regex pattern is typically: "^(name1|name2|...)$"
    let re_match = regex::Regex::new(
        r#"#match\?\s+@\w+(?:\.\w+)?\s+"[^^]*\^?\(([^)]+)\)\$?""#,
    ).unwrap();

    for cap in re_match.captures_iter(content) {
        if let Some(alternation) = cap.get(1) {
            for name in alternation.as_str().split('|') {
                let name = name.trim();
                if !name.is_empty() {
                    names.insert(name.to_string());
                }
            }
        }
    }
}

fn extract_eq_predicates(content: &str, names: &mut BTreeSet<String>) {
    // Find #eq? predicates: (#eq? @variable.builtin "self")
    let re_eq = regex::Regex::new(
        r#"#eq\?\s+@\w+(?:\.\w+)?\s+"([^"]+)""#,
    ).unwrap();

    for cap in re_eq.captures_iter(content) {
        if let Some(name) = cap.get(1) {
            names.insert(name.as_str().to_string());
        }
    }
}

fn extract_keyword_strings(content: &str, names: &mut BTreeSet<String>) {
    // Match: "word" @keyword  or  "word" @keyword.something
    let re_kw = regex::Regex::new(
        r#""([a-zA-Z_][a-zA-Z0-9_!?]*(?:::)?[a-zA-Z0-9_!?]*)"\s+@keyword"#,
    ).unwrap();

    for cap in re_kw.captures_iter(content) {
        if let Some(name) = cap.get(1) {
            names.insert(name.as_str().to_string());
        }
    }
}

fn extract_builtin_strings(content: &str, names: &mut BTreeSet<String>) {
    // Match: "word" @type.builtin  or  "word" @constant.builtin etc.
    let re_bi = regex::Regex::new(
        r#""([a-zA-Z_][a-zA-Z0-9_!?]*)"\s+@\w+\.builtin"#,
    ).unwrap();

    for cap in re_bi.captures_iter(content) {
        if let Some(name) = cap.get(1) {
            names.insert(name.as_str().to_string());
        }
    }
}
