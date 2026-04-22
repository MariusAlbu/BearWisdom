// =============================================================================
// ecosystem/manifest/js_config_aliases.rs — webpack/vite/vue.config resolve.alias parser
//
// Vite, Vue CLI, Nuxt, and webpack all let projects define path aliases in a
// JS/TS config file rather than tsconfig.json. Chatwoot's vite.config.ts:
//
//   resolve: {
//     alias: {
//       vue: 'vue/dist/vue.esm-bundler.js',
//       next: path.resolve('./app/javascript/dashboard/components-next'),
//       dashboard: path.resolve('./app/javascript/dashboard'),
//     },
//   }
//
// Without these, an `import NextButton from 'next/button/Button.vue'` has no
// mapping the resolver can follow, and every template reference to the
// component lands unresolved. This module walks the config AST with
// tree-sitter-javascript, finds every `{ resolve: { alias: { ... } } }`
// object literal (and the webpack wrappers `configureWebpack` /
// `chainWebpack` / `rollupOptions` where projects nest it), and extracts
// the key → path entries.
//
// Output shape is (`alias/`, `target/`) prefix tuples — the exact form
// `NpmManifest.tsconfig_paths` stores so the existing
// `ProjectContext::resolve_tsconfig_alias` mechanism picks them up without
// any resolver changes.
//
// Only statically evaluable values are extracted:
//   - bare string literals:  `next: './src'`
//   - `path.resolve('./foo')`, `path.resolve(__dirname, './foo')`,
//     `path.join(...)` — last string arg wins.
//   - `fileURLToPath(new URL('./foo', import.meta.url))` — the URL's first
//     string arg.
// Values that depend on runtime expressions (variable references, template
// literals with interpolation, function calls returning computed paths) are
// skipped — we'd rather drop the entry than bind it wrong.
//
// Exact-match webpack aliases (`vue$: '...'`) are skipped — they rewrite a
// bare module to a different one, not a prefix mapping.
// =============================================================================

use tree_sitter::{Node, Parser};

/// Parse a vite/vue/webpack/nuxt config file's content and return the
/// discovered `resolve.alias` mappings as `(prefix, target_prefix)` tuples
/// with trailing `/` — the same shape `parse_tsconfig_paths` emits.
///
/// Returns an empty vec for any config without a parseable `alias` object.
pub fn parse_js_config_aliases(content: &str) -> Vec<(String, String)> {
    let language: tree_sitter::Language = tree_sitter_javascript::LANGUAGE.into();
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(content, None) else {
        return Vec::new();
    };
    let src = content.as_bytes();
    let mut out = Vec::new();
    walk_for_alias(&tree.root_node(), src, &mut out);
    out
}

// ---------------------------------------------------------------------------
// AST walk
// ---------------------------------------------------------------------------

/// Depth-first walk the CST. At every `object` node, check each pair. If the
/// pair's key is `alias`, treat its value object as an alias table and
/// extract entries. Continue walking regardless so nested configs (e.g.
/// `configureWebpack.resolve.alias`) are caught.
fn walk_for_alias(node: &Node, src: &[u8], out: &mut Vec<(String, String)>) {
    if node.kind() == "object" {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() != "pair" {
                continue;
            }
            let Some(key_node) = child.child_by_field_name("key") else { continue };
            let Some(key) = key_name(&key_node, src) else { continue };
            if key != "alias" {
                continue;
            }
            let Some(value_node) = child.child_by_field_name("value") else { continue };
            if value_node.kind() == "object" {
                extract_alias_entries(&value_node, src, out);
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_for_alias(&child, src, out);
    }
}

/// Walk the alias object's pairs, extract (key, value) where both are
/// statically evaluable strings, and push as trailing-slash tuples.
fn extract_alias_entries(obj: &Node, src: &[u8], out: &mut Vec<(String, String)>) {
    let mut cursor = obj.walk();
    for child in obj.children(&mut cursor) {
        if child.kind() != "pair" {
            continue;
        }
        let Some(key_node) = child.child_by_field_name("key") else { continue };
        let Some(value_node) = child.child_by_field_name("value") else { continue };
        let Some(key) = key_name(&key_node, src) else { continue };
        // Webpack exact-match alias (`vue$`) isn't a prefix mapping — skip.
        if key.ends_with('$') {
            continue;
        }
        let Some(value) = evaluate_string_value(&value_node, src) else { continue };
        // Empty values would create catch-all rewrites. Skip.
        if value.is_empty() {
            continue;
        }
        out.push((format!("{key}/"), format!("{}/", normalize_path(&value))));
    }
}

// ---------------------------------------------------------------------------
// Key / value extraction
// ---------------------------------------------------------------------------

/// Extract the name of an object-literal pair key. Handles:
///   `alias: { … }`             → property_identifier "alias"
///   `'alias': { … }`           → string "alias"
///   `"alias": { … }`           → string "alias"
///   `[computed]: { … }`        → computed_property_name — skip (can't resolve)
fn key_name(node: &Node, src: &[u8]) -> Option<String> {
    match node.kind() {
        "property_identifier" | "identifier" => Some(node_text(node, src).to_string()),
        "string" => string_literal_contents(node, src),
        _ => None,
    }
}

/// Extract a static string from a value node. Handles the common evaluable
/// shapes in config files; returns None for anything dynamic.
fn evaluate_string_value(node: &Node, src: &[u8]) -> Option<String> {
    match node.kind() {
        "string" => string_literal_contents(node, src),
        "template_string" => template_string_literal_contents(node, src),
        "call_expression" => evaluate_call_expression(node, src),
        "new_expression" => evaluate_new_expression(node, src),
        _ => None,
    }
}

/// `path.resolve(...)`, `path.join(...)`, `fileURLToPath(...)`,
/// `require.resolve(...)` — take the last statically-evaluable string arg.
///
/// Also handles `resolve(...)` / `join(...)` when the caller is a bare
/// identifier (projects sometimes destructure `const { resolve } = path`).
fn evaluate_call_expression(node: &Node, src: &[u8]) -> Option<String> {
    let func = node.child_by_field_name("function")?;
    let func_text = node_text(&func, src);
    let qualifies = matches!(
        func_text,
        "path.resolve" | "path.join" | "resolve" | "join" | "fileURLToPath"
    );
    if !qualifies {
        return None;
    }
    let args = node.child_by_field_name("arguments")?;
    // Pull out all statically evaluable string args, return the last one.
    // path.resolve('a', 'b') → 'a/b' at runtime, but for our purpose the
    // anchoring path is the last arg (the one most often carrying the
    // real alias target).
    //
    // fileURLToPath(new URL('./foo', import.meta.url)) — a single arg that's
    // a new_expression; evaluate_new_expression handles that.
    let mut last: Option<String> = None;
    let mut cursor = args.walk();
    for arg in args.children(&mut cursor) {
        if !arg.is_named() {
            continue;
        }
        if let Some(v) = evaluate_string_value(&arg, src) {
            last = Some(v);
        }
    }
    last
}

/// `new URL('./foo', import.meta.url)` — first statically-evaluable string.
fn evaluate_new_expression(node: &Node, src: &[u8]) -> Option<String> {
    let constructor = node.child_by_field_name("constructor")?;
    if node_text(&constructor, src) != "URL" {
        return None;
    }
    let args = node.child_by_field_name("arguments")?;
    let mut cursor = args.walk();
    for arg in args.children(&mut cursor) {
        if !arg.is_named() {
            continue;
        }
        if let Some(v) = evaluate_string_value(&arg, src) {
            return Some(v);
        }
    }
    None
}

/// Peel the surrounding `"..."` / `'...'` delimiters from a string literal.
fn string_literal_contents(node: &Node, src: &[u8]) -> Option<String> {
    // tree-sitter-javascript string nodes contain a `string_fragment` child
    // with the unescaped content. Older grammar builds expose the raw text;
    // fall back to stripping quotes if the fragment child isn't present.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string_fragment" {
            return Some(node_text(&child, src).to_string());
        }
    }
    let raw = node_text(node, src);
    let trimmed = raw
        .strip_prefix('"').or_else(|| raw.strip_prefix('\''))?
        .strip_suffix('"').or_else(|| raw.strip_suffix('\''))?;
    Some(trimmed.to_string())
}

/// Template string without any `${...}` interpolation — otherwise bail.
fn template_string_literal_contents(node: &Node, src: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    let mut out = String::new();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "template_substitution" => return None, // dynamic, can't evaluate
            "string_fragment" => out.push_str(node_text(&child, src)),
            "`" => {}
            _ => {}
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

// ---------------------------------------------------------------------------
// Path normalisation
// ---------------------------------------------------------------------------

/// Strip leading `./`, convert Windows separators to forward slashes, and
/// collapse redundant slashes. Leaves the path in a form comparable to
/// `tsconfig_paths` targets.
fn normalize_path(raw: &str) -> String {
    let mut s = raw.replace('\\', "/");
    while let Some(rest) = s.strip_prefix("./") {
        s = rest.to_string();
    }
    // Strip trailing slash (the caller adds a single trailing / separator).
    while s.ends_with('/') {
        s.pop();
    }
    s
}

fn node_text<'a>(node: &Node<'a>, src: &'a [u8]) -> &'a str {
    std::str::from_utf8(&src[node.start_byte()..node.end_byte()]).unwrap_or("")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_vite_config_with_path_resolve() {
        let src = r#"
            import { defineConfig } from 'vite';
            import path from 'path';
            export default defineConfig({
                resolve: {
                    alias: {
                        next: path.resolve('./app/javascript/dashboard/components-next'),
                        dashboard: path.resolve('./app/javascript/dashboard'),
                    },
                },
            });
        "#;
        let aliases = parse_js_config_aliases(src);
        assert!(
            aliases.iter().any(|(k, v)|
                k == "next/" && v == "app/javascript/dashboard/components-next/"),
            "expected next → app/javascript/dashboard/components-next, got {aliases:?}"
        );
        assert!(
            aliases.iter().any(|(k, v)|
                k == "dashboard/" && v == "app/javascript/dashboard/"),
            "expected dashboard alias, got {aliases:?}"
        );
    }

    #[test]
    fn parses_bare_string_alias() {
        // Vue$ with exact-match suffix should be skipped; bare string aliases
        // pass through. Mirrors chatwoot's `vue: 'vue/dist/vue.esm-bundler.js'`.
        let src = r#"
            export default {
                resolve: {
                    alias: {
                        'vue$': 'vue/dist/vue.esm-bundler.js',
                        '@': 'src',
                    },
                },
            };
        "#;
        let aliases = parse_js_config_aliases(src);
        assert!(
            aliases.iter().any(|(k, _)| k == "@/"),
            "bare `@` alias should be captured: {aliases:?}"
        );
        assert!(
            !aliases.iter().any(|(k, _)| k.starts_with("vue$")),
            "exact-match vue$ alias must be skipped: {aliases:?}"
        );
    }

    #[test]
    fn parses_webpack_nested_under_configure_webpack() {
        // Vue CLI pattern: `configureWebpack.resolve.alias`.
        let src = r#"
            module.exports = {
                configureWebpack: {
                    resolve: {
                        alias: {
                            '@': path.resolve(__dirname, 'src'),
                            '@components': path.resolve(__dirname, 'src/components'),
                        },
                    },
                },
            };
        "#;
        let aliases = parse_js_config_aliases(src);
        assert!(
            aliases.iter().any(|(k, v)| k == "@/" && v == "src/"),
            "nested @ alias must resolve: {aliases:?}"
        );
        assert!(
            aliases.iter().any(|(k, v)| k == "@components/" && v == "src/components/"),
            "nested @components alias: {aliases:?}"
        );
    }

    #[test]
    fn parses_fileurl_to_path_new_url() {
        let src = r#"
            export default defineConfig({
                resolve: {
                    alias: {
                        '@': fileURLToPath(new URL('./src', import.meta.url)),
                    },
                },
            });
        "#;
        let aliases = parse_js_config_aliases(src);
        assert!(
            aliases.iter().any(|(k, v)| k == "@/" && v == "src/"),
            "fileURLToPath(new URL(...)) should unwrap: {aliases:?}"
        );
    }

    #[test]
    fn ignores_dynamic_values() {
        // Template interpolation and bare identifier references can't be
        // statically evaluated — entries should be dropped, not guessed.
        let src = r#"
            const base = './src';
            export default {
                resolve: {
                    alias: {
                        '@': base,
                        '@str': `${base}/str`,
                        '@static': './literal',
                    },
                },
            };
        "#;
        let aliases = parse_js_config_aliases(src);
        assert!(
            !aliases.iter().any(|(k, _)| k == "@/"),
            "identifier reference `base` must be dropped: {aliases:?}"
        );
        assert!(
            !aliases.iter().any(|(k, _)| k == "@str/"),
            "interpolated template string must be dropped: {aliases:?}"
        );
        assert!(
            aliases.iter().any(|(k, v)| k == "@static/" && v == "literal/"),
            "plain string alias must still pass through: {aliases:?}"
        );
    }

    #[test]
    fn empty_config_yields_empty_vec() {
        let src = "export default {};";
        assert!(parse_js_config_aliases(src).is_empty());
    }

    #[test]
    fn no_alias_key_yields_empty_vec() {
        let src = r#"
            export default {
                resolve: {
                    extensions: ['.js', '.ts'],
                },
            };
        "#;
        assert!(parse_js_config_aliases(src).is_empty());
    }

    #[test]
    fn chatwoot_vite_config_real_shape() {
        // Exact pattern from ruby-chatwoot/vite.config.ts that motivated
        // this work — single-arg `path.resolve()` calls.
        let src = r#"
            export default defineConfig({
                plugins: plugins,
                resolve: {
                    alias: {
                        vue: 'vue/dist/vue.esm-bundler.js',
                        components: path.resolve('./app/javascript/dashboard/components'),
                        next: path.resolve('./app/javascript/dashboard/components-next'),
                        v3: path.resolve('./app/javascript/v3'),
                        dashboard: path.resolve('./app/javascript/dashboard'),
                    },
                },
            });
        "#;
        let aliases = parse_js_config_aliases(src);
        let find = |prefix: &str| aliases.iter().find(|(k, _)| k == prefix).map(|(_, v)| v.clone());
        assert_eq!(find("next/"), Some("app/javascript/dashboard/components-next/".to_string()));
        assert_eq!(find("v3/"), Some("app/javascript/v3/".to_string()));
        assert_eq!(find("dashboard/"), Some("app/javascript/dashboard/".to_string()));
        assert_eq!(find("components/"), Some("app/javascript/dashboard/components/".to_string()));
        assert_eq!(find("vue/"), Some("vue/dist/vue.esm-bundler.js/".to_string()));
    }
}
