// =============================================================================
// ecosystem/npm/ts_scan.rs — TS source scanning (exports, globals, header, vue)
// =============================================================================

use std::collections::HashMap;

use tree_sitter::{Node, Parser};

use super::package_declares_globals;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ExportSource {
    /// `function X() {}`, `class X {}`, `export { localX }`, etc. — X is
    /// defined in this file.
    Local,
    /// `export { Orig as Exposed } from 'module'` — the exposed name is
    /// sourced from `module`, under `original_name` (which equals the
    /// exposed name when there's no renaming).
    Reexport { module: String, original: String },
    /// `export * as ns from 'module'` — the exposed name is the whole
    /// module's namespace object. Resolves to the module's entry file;
    /// there's no single `original` symbol name to track because the
    /// namespace is the aggregate of every export in `module`.
    Namespace { module: String },
}

/// Per-file export summary built by `scan_ts_file_exports`. Keys of
/// `named` are exposed names; the enum tells us whether a name is a
/// definition or a re-export so the index builder can follow re-exports
/// to the file that actually defines the symbol.
#[derive(Debug, Default, Clone)]
pub(super) struct FileExports {
    /// exposed_name → where the value comes from
    pub(super) named: HashMap<String, ExportSource>,
    /// Module specifiers of `export * from '<module>'` statements.
    pub(super) wildcards: Vec<String>,
    /// `declare global { ... }` names — surfaced separately (pollute the
    /// global namespace regardless of any import).
    pub(super) globals: Vec<String>,
}

/// Header-only tree-sitter scan of a TS/TSX/JS source file. Returns a
/// FileExports describing:
///
/// - `named`: top-level decls and named exports. Each keyed by the name
///   *as exposed by this file* and tagged with whether it's defined here
///   (Local) or re-exported from another module (Reexport).
/// - `wildcards`: `export * from 'x'` module specifiers.
/// - `globals`: names declared inside `declare global { ... }` blocks.
///
/// Function/method/class bodies are not walked. DefinitelyTyped shapes
/// (`declare module 'foo' { ... }`) surface their inner decls as Local
/// under the ambient module name of the containing file.
pub(crate) fn scan_ts_file_exports(source: &str, language: &str) -> FileExports {
    let mut out = FileExports::default();

    let ts_lang: tree_sitter::Language = match language {
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "javascript" | "jsx" => tree_sitter_javascript::LANGUAGE.into(),
        _ => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
    };
    let mut parser = Parser::new();
    if parser.set_language(&ts_lang).is_err() {
        return out;
    }
    let Some(tree) = parser.parse(source, None) else {
        return out;
    };

    let root = tree.root_node();
    let bytes = source.as_bytes();

    // First pass: build a local→(module, original_name) import map so the
    // export pass can decide whether `export { X }` (no `from`) is a
    // genuine local forward or a re-export of an imported binding. Without
    // this, `import { X } from './mod'; export { X }` was misrecorded as
    // Local pointing at the barrel file — breaking any downstream lookup
    // that expected X's definition to be in `./mod`.
    let mut imports: HashMap<String, (String, String)> = HashMap::new();
    {
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            collect_imports(&child, bytes, &mut imports);
        }
    }

    // Second pass: exports + wildcards + namespace re-exports.
    {
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            collect_file_exports(&child, bytes, &mut out, &imports);
        }
    }

    // `declare global { ... }` extraction: tree-sitter-typescript's grammar
    // wraps this inconsistently across minor grammar releases, so fall back
    // to a regex sweep of the source.
    out.globals = scan_declare_global_blocks(source);

    // A global-script `.d.ts` (no top-level import/export) also contributes its
    // top-level `declare const`/`var`/`function`/… to the ambient scope — the
    // shape `@types/jest` uses for `expect`, `describe`, `it`, … which the
    // `declare global` / `declare namespace` sweep above does not reach.
    out.globals.extend(scan_global_script_top_level_decls(source));

    // `declare module 'vue' { interface GlobalComponents { ... } }` —
    // member names are auto-registered as global Vue template components
    // by `app.use(<plugin>)`. Source forms covered:
    //
    //   declare module 'vue' { interface GlobalComponents { NButton: ...; } }
    //   declare module '@vue/runtime-core' { interface GlobalComponents { RouterLink: ...; } }
    //   declare module 'vue/types/vue' { interface GlobalComponents { ... } }
    //
    // Lifted into `out.globals` so the existing `__npm_globals__` demand-pull
    // path and heuristic ambient-path priority pick them up identically to
    // `declare global { const expect; }`-style names.
    if source.contains("GlobalComponents") {
        out.globals.extend(scan_vue_global_components(source));
    }

    out
}

/// Extract member names from `declare module 'vue' { interface GlobalComponents { ... } }`
/// and equivalent augmentations (`@vue/runtime-core`, `vue/types/vue`).
/// Each property declaration inside the interface contributes its name.
///
/// Two member shapes are common:
///   - explicit-list (Naive UI volar.d.ts, Vue Router, Element Plus,
///     unplugin-vue-components-generated `components.d.ts`):
///     `NButton: (typeof import('naive-ui'))['NButton']`
///   - reference shape (Vue Router): `RouterLink: typeof RouterLink`
///
/// Both surface as a property whose name is the leftmost identifier — only
/// the name is extracted; the type expression is irrelevant for resolution
/// since the symbol gets pulled by name via `__npm_globals__`.
pub(crate) fn scan_vue_global_components(source: &str) -> Vec<String> {
    // Match `declare module '<vue-ish>'` opening braces.
    let module_re = regex::Regex::new(
        r#"declare\s+module\s+['"](?:vue|@vue/runtime-core|vue/types/vue)['"]\s*\{"#,
    )
    .expect("vue module regex");
    let bytes = source.as_bytes();

    let mut out: Vec<String> = Vec::new();
    for m in module_re.find_iter(source) {
        let open_brace = m.end() - 1;
        let Some(close) = find_matching_brace(bytes, open_brace) else {
            continue;
        };
        let module_block = &source[open_brace + 1..close];

        // Find `interface GlobalComponents` (with optional `export`/`extends`)
        // inside the module block, then collect property names from its body.
        let iface_re = regex::Regex::new(
            r"(?:export\s+)?interface\s+GlobalComponents(?:\s+extends\s+[^{]+)?\s*\{",
        )
        .expect("globalcomponents interface regex");
        let iface_block_bytes = module_block.as_bytes();
        for cap in iface_re.find_iter(module_block) {
            let body_open = cap.end() - 1;
            let Some(body_close) = find_matching_brace(iface_block_bytes, body_open) else {
                continue;
            };
            let body = &module_block[body_open + 1..body_close];
            // Property declarations: `Name: <type>` or `Name?: <type>`.
            // Skip nested braces (e.g. mapped types) by only matching at the
            // shallow level — naïve approach via line-anchored regex.
            let prop_re =
                regex::Regex::new(r"(?m)^\s*([A-Za-z_$][\w$]*)\s*\??\s*:").expect("property regex");
            for prop_cap in prop_re.captures_iter(body) {
                out.push(prop_cap[1].to_string());
            }
        }
    }
    out
}

/// Populate `out` with every import binding surfaced by an
/// `import_statement` node. Entry shape: `local_name → (module, original)`.
///
/// - `import X from 'mod'`                    → `"X" → ("mod", "default")`
/// - `import { X, Y as Y2 } from 'mod'`       → `"X" → ("mod", "X")`, `"Y2" → ("mod", "Y")`
/// - `import * as ns from 'mod'`              → `"ns" → ("mod", "*")`
/// - `import 'side-effect'`                   → nothing
///
/// The sentinel `"*"` in the original-name slot is recognised by the
/// export pass and upgraded to an `ExportSource::Namespace` when the
/// binding is re-exported without a `from` clause.
pub(crate) fn collect_imports(
    node: &Node,
    bytes: &[u8],
    out: &mut HashMap<String, (String, String)>,
) {
    if node.kind() != "import_statement" {
        return;
    }
    let Some(module) = extract_export_source_module(node, bytes) else {
        return;
    };
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() != "import_clause" {
            continue;
        }
        let mut cc = child.walk();
        for piece in child.children(&mut cc) {
            match piece.kind() {
                // `import X from 'mod'` — default import.
                "identifier" => {
                    if let Ok(name) = piece.utf8_text(bytes) {
                        out.insert(name.to_string(), (module.clone(), "default".to_string()));
                    }
                }
                // `import { X, Y as Y2 } from 'mod'`
                "named_imports" => {
                    let mut sc = piece.walk();
                    for spec in piece.children(&mut sc) {
                        if spec.kind() != "import_specifier" {
                            continue;
                        }
                        let orig_node = spec.child_by_field_name("name");
                        let alias_node = spec.child_by_field_name("alias");
                        let local_node = alias_node.or(orig_node);
                        let (Some(on), Some(ln)) = (orig_node, local_node) else {
                            continue;
                        };
                        let Ok(original) = on.utf8_text(bytes) else {
                            continue;
                        };
                        let Ok(local) = ln.utf8_text(bytes) else {
                            continue;
                        };
                        out.insert(local.to_string(), (module.clone(), original.to_string()));
                    }
                }
                // `import * as ns from 'mod'`
                "namespace_import" => {
                    let mut sc = piece.walk();
                    for n in piece.children(&mut sc) {
                        if n.kind() == "identifier" {
                            if let Ok(name) = n.utf8_text(bytes) {
                                out.insert(name.to_string(), (module.clone(), "*".to_string()));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// Back-compat helper: the pre-refactor `scan_ts_header` returned
/// `(regular_names, global_names)` as flat vecs. Tests and a handful of
/// callers still depend on that shape. We derive it from the richer
/// FileExports so both surfaces stay in sync.
#[cfg(test)]
pub(crate) fn scan_ts_header(source: &str, language: &str) -> (Vec<String>, Vec<String>) {
    let exports = scan_ts_file_exports(source, language);
    let mut regular: Vec<String> = exports.named.into_keys().collect();
    regular.sort();
    (regular, exports.globals)
}

/// Extract names declared inside `declare global { ... }` and top-level
/// `declare namespace X { ... }` blocks. Returns a flat list including
/// dotted names for declarations nested inside `namespace` wrappers.
///
/// Examples of names emitted:
/// - `declare global { const expect; }` → `expect`
/// - `declare global { namespace Express { interface Request {} } }` → `Express`, `Express.Request`
/// - `declare namespace google.maps { class Map {} class LatLng {} }` → `google.maps.Map`, `google.maps.LatLng`
/// - `declare namespace google { namespace maps { class Map {} } }` → `google`, `google.maps.Map`
///
/// Source-scan approach (rather than tree-sitter) is grammar-independent
/// against tree-sitter-typescript's variance in how `global` and ambient
/// `namespace` wrappers land in the CST.
pub(crate) fn scan_declare_global_blocks(source: &str) -> Vec<String> {
    let has_global = source.contains("declare global");
    let has_ns = source.contains("declare namespace");
    if !has_global && !has_ns {
        return Vec::new();
    }
    let bytes = source.as_bytes();

    let mut out: Vec<String> = Vec::new();

    if has_global {
        let marker_re = regex::Regex::new(r"declare\s+global\s*\{").expect("declare global regex");
        for m in marker_re.find_iter(source) {
            // Opening `{` is the last char of the match.
            let open_brace = m.end() - 1;
            if let Some(close) = find_matching_brace(bytes, open_brace) {
                let block = &source[open_brace + 1..close];
                collect_namespace_decls("", block, &mut out);
            }
        }
    }

    if has_ns {
        // Top-level `declare namespace X.Y { ... }` and `declare namespace X { ... }`.
        // The namespace path can be dotted (e.g. `declare namespace google.maps`).
        let ns_re = regex::Regex::new(
            r"(?m)^\s*declare\s+namespace\s+([A-Za-z_$][\w$]*(?:\s*\.\s*[A-Za-z_$][\w$]*)*)\s*\{",
        )
        .expect("declare namespace regex");
        for cap in ns_re.captures_iter(source) {
            let path: String = cap[1].chars().filter(|c| !c.is_whitespace()).collect();
            let m = cap.get(0).unwrap();
            let open_brace = m.end() - 1;
            if let Some(close) = find_matching_brace(bytes, open_brace) {
                let block = &source[open_brace + 1..close];
                out.push(path.clone());
                collect_namespace_decls(&path, block, &mut out);
            }
        }
    }

    out
}

/// Top-level value declarations of a *global-script* `.d.ts` — a file with no
/// top-level `import`/`export`, whose top-level `declare` statements therefore
/// land in the global ambient scope. This is the DefinitelyTyped convention for
/// global packages: `@types/jest`'s `index.d.ts` declares `declare const expect`,
/// `declare var describe`, … as ambient globals callable without an `import`.
///
/// A module file (any top-level `import`/`export`) scopes its declarations to
/// the module, so it contributes nothing and returns empty. `declare namespace`
/// is intentionally excluded — `scan_declare_global_blocks` already emits its
/// (dotted) members; this covers the flat value globals it misses: `const` /
/// `let` / `var` / `function` / `class` / `enum`.
pub(crate) fn scan_global_script_top_level_decls(source: &str) -> Vec<String> {
    // A top-level `import`/`export` marks the file a module — its top-level
    // declares are module-scoped, not global, so there is nothing to lift.
    if has_top_level_module_marker(source) {
        return Vec::new();
    }
    static DECL_RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r"(?m)^declare\s+(?:const|let|var|function|async\s+function|class|abstract\s+class|enum)\s+([A-Za-z_$][\w$]*)",
        )
        .expect("global-script decl regex")
    });
    DECL_RE
        .captures_iter(source)
        .map(|c| c[1].to_string())
        .collect()
}

/// True when `source` has a column-0 `import` or `export` statement — the signal
/// that a `.d.ts` is a module (declarations module-scoped) rather than a global
/// script. Anchored to column 0 so inline type imports (`typeof import('x')`)
/// and an `export` nested inside a `declare global { … }` body don't count.
fn has_top_level_module_marker(source: &str) -> bool {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?m)^(?:import|export)\b").expect("module marker regex")
    });
    RE.is_match(source)
}

/// Given a source byte offset pointing at an opening `{`, return the offset
/// of the matching `}`, or `None` if unbalanced. Naïve brace counter — does
/// not skip braces inside strings/comments, but `.d.ts` declaration files
/// don't realistically contain those at significant depth.
pub(crate) fn find_matching_brace(bytes: &[u8], open_brace: usize) -> Option<usize> {
    let mut depth = 1i32;
    let mut i = open_brace + 1;
    while i < bytes.len() && depth > 0 {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Walk a block body, emitting names for each top-level declaration. When
/// a nested `namespace Y { ... }` appears, recurse with the prefix extended
/// (`prefix.Y`) so leaf decls land as `prefix.Y.Leaf`.
///
/// `prefix` is the current dotted namespace path (`""` at the outermost
/// `declare global` body). Decls at the current level are pushed as
/// `prefix.name` (or just `name` when prefix is empty). Inner namespace
/// wrapper names are pushed too, so a chain ref like `Express.Multer` (one
/// hop short of a leaf) still finds *something* in the index.
pub(crate) fn collect_namespace_decls(prefix: &str, block: &str, out: &mut Vec<String>) {
    // JS/TS identifiers allow `$` and `_` as the leading character (and
    // anywhere else). `\w` is `[A-Za-z0-9_]` and silently drops anything
    // starting with `$` — Angular's `$localize`, jQuery's `$`, lodash's
    // `_` (when not a namespace), Cypress's `cy` (only happens to be \w),
    // RxJS's `$`-suffix observables. Use the full JS identifier shape so
    // every globally-declared symbol that downstream projects reference
    // gets indexed.
    let decl_re = regex::Regex::new(
        r"(?m)^\s*(?:export\s+)?(?:const|let|var|function|class|abstract\s+class|type|interface|enum)\s+([A-Za-z_$][\w$]*)",
    )
    .expect("namespace decl regex");
    for cap in decl_re.captures_iter(block) {
        let name = &cap[1];
        out.push(if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}.{name}")
        });
    }

    // Nested namespace wrappers: `namespace X { ... }` (with optional
    // `export`). Path can be dotted: `namespace X.Y { ... }`.
    let ns_re = regex::Regex::new(
        r"(?m)^\s*(?:export\s+)?namespace\s+([A-Za-z_$][\w$]*(?:\s*\.\s*[A-Za-z_$][\w$]*)*)\s*\{",
    )
    .expect("nested namespace regex");
    let bytes = block.as_bytes();
    for cap in ns_re.captures_iter(block) {
        let path: String = cap[1].chars().filter(|c| !c.is_whitespace()).collect();
        let m = cap.get(0).unwrap();
        let open_brace = m.end() - 1;
        if let Some(close) = find_matching_brace(bytes, open_brace) {
            let inner = &block[open_brace + 1..close];
            let new_prefix = if prefix.is_empty() {
                path.clone()
            } else {
                format!("{prefix}.{path}")
            };
            // The wrapper name itself is also a useful index entry — chain
            // refs that stop one hop short of a leaf (`Express.Multer`)
            // still resolve to *something* rather than going unmatched.
            out.push(new_prefix.clone());
            collect_namespace_decls(&new_prefix, inner, out);
        }
    }
}

/// Inspect one direct child of the source-file root and record any top-level
/// declaration or export into `out`. Local definitions land as
/// `ExportSource::Local`; named re-exports (`export { X } from 'mod'`) land
/// as `ExportSource::Reexport`; star re-exports (`export * from 'mod'`) land
/// in `out.wildcards`. Recurses into `export_statement`, `ambient_declaration`,
/// and `internal_module`/`module` (namespace) wrappers. Does NOT recurse into
/// `block` / `statement_block` / `class_body` / `function_body` — bodies are
/// where header-only parsing draws the line.
pub(crate) fn collect_file_exports(
    node: &Node,
    bytes: &[u8],
    out: &mut FileExports,
    imports: &HashMap<String, (String, String)>,
) {
    match node.kind() {
        "function_declaration"
        | "generator_function_declaration"
        | "function_signature"
        | "class_declaration"
        | "abstract_class_declaration"
        | "interface_declaration"
        | "type_alias_declaration"
        | "enum_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(bytes) {
                    out.named
                        .entry(name.to_string())
                        .or_insert(ExportSource::Local);
                }
            }
        }
        "lexical_declaration" | "variable_declaration" => {
            let mut cursor = node.walk();
            for decl in node.children(&mut cursor) {
                if decl.kind() == "variable_declarator" {
                    if let Some(name_node) = decl.child_by_field_name("name") {
                        if name_node.kind() == "identifier" {
                            if let Ok(name) = name_node.utf8_text(bytes) {
                                out.named
                                    .entry(name.to_string())
                                    .or_insert(ExportSource::Local);
                            }
                        }
                    }
                }
            }
        }
        "export_statement" => {
            // Source module (`from '<mod>'`) if present on this statement:
            // `export { X } from './m'`           →  source = Some("./m")
            // `export class Foo {}`                →  source = None
            // `export * from './m'`                →  source = Some("./m")
            // `export * as ns from './m'`          →  source = Some("./m"), namespace re-export
            let source_module = extract_export_source_module(node, bytes);

            // Scan direct children once for the three distinct shapes we need:
            //   `*` alone                 → wildcard re-export
            //   `*` + namespace_export    → named namespace re-export
            //   `default` keyword         → default export (sibling may be a decl or identifier)
            let mut has_star = false;
            let mut namespace_name: Option<String> = None;
            let mut has_default_keyword = false;
            {
                let mut cursor = node.walk();
                for ch in node.children(&mut cursor) {
                    match ch.kind() {
                        "*" => has_star = true,
                        "default" => has_default_keyword = true,
                        "namespace_export" => {
                            // `* as ns` — the identifier lives inside.
                            let mut nc = ch.walk();
                            for nch in ch.children(&mut nc) {
                                if nch.kind() == "identifier" {
                                    if let Ok(n) = nch.utf8_text(bytes) {
                                        namespace_name = Some(n.to_string());
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }

            if has_star {
                if let Some(src) = source_module.clone() {
                    if let Some(ns) = namespace_name {
                        // `export * as ns from './mod'` — single named
                        // export bound to the whole module namespace.
                        out.named
                            .entry(ns)
                            .or_insert(ExportSource::Namespace { module: src });
                    } else {
                        // `export * from './mod'` — pure wildcard.
                        out.wildcards.push(src);
                    }
                }
                // Star statement has no further specifiers to process.
                return;
            }

            // `export default ...` — register a synthetic `default` entry
            // so downstream `export { default as X } from './this'`
            // re-exports in other files can resolve through the index.
            // Points at this file: the wrapped declaration, object literal,
            // or expression is what the default value resolves to.
            if has_default_keyword {
                out.named
                    .entry("default".to_string())
                    .or_insert(ExportSource::Local);
            }

            // Walk children: each export_clause specifier is a (re-)export;
            // other children are wrapped local decls to recurse into.
            let mut cursor = node.walk();
            for inner in node.children(&mut cursor) {
                match inner.kind() {
                    "export_clause" => {
                        let mut cc = inner.walk();
                        for spec in inner.children(&mut cc) {
                            if spec.kind() != "export_specifier" {
                                continue;
                            }
                            let orig_node = spec.child_by_field_name("name");
                            let alias_node = spec.child_by_field_name("alias");
                            let exposed_node = alias_node.or(orig_node);
                            let (Some(on), Some(en)) = (orig_node, exposed_node) else {
                                continue;
                            };
                            let Ok(original) = on.utf8_text(bytes) else {
                                continue;
                            };
                            let Ok(exposed) = en.utf8_text(bytes) else {
                                continue;
                            };
                            // Three cases for the source:
                            //   (a) `export { X } from 'mod'` — direct re-export.
                            //   (b) `export { X }` with no `from`, but X was
                            //       imported in this file — transitive re-export.
                            //       Without this branch, the index would record
                            //       X as Local to the barrel and lose the
                            //       connection to the real definition file.
                            //   (c) `export { X }` where X is a local decl.
                            let source = if let Some(m) = source_module.clone() {
                                // (a)
                                ExportSource::Reexport {
                                    module: m,
                                    original: original.to_string(),
                                }
                            } else if let Some((m, imp_orig)) = imports.get(original) {
                                // (b) — forward the imported binding to its real source.
                                // `import * as ns; export { ns }` needs Namespace,
                                // not Reexport, because `*` isn't a real symbol.
                                if imp_orig == "*" {
                                    ExportSource::Namespace { module: m.clone() }
                                } else {
                                    ExportSource::Reexport {
                                        module: m.clone(),
                                        original: imp_orig.clone(),
                                    }
                                }
                            } else {
                                // (c) — genuinely local.
                                ExportSource::Local
                            };
                            out.named.entry(exposed.to_string()).or_insert(source);
                        }
                    }
                    _ => {
                        collect_file_exports(&inner, bytes, out, imports);
                    }
                }
            }
        }
        "ambient_declaration" => {
            // `declare class X {}`, `declare function f(): void`, `declare
            // module 'foo' { ... }` — recurse so the wrapped declaration
            // gets classified by its own arm.
            let mut cursor = node.walk();
            for inner in node.children(&mut cursor) {
                collect_file_exports(&inner, bytes, out, imports);
            }
        }
        "internal_module" | "module" => {
            // `namespace X { ... }` or `module 'foo' { ... }`. Recurse
            // into the body; each direct child is itself a decl.
            let body = node
                .child_by_field_name("body")
                .or_else(|| find_named_child(node, &["statement_block"]));
            if let Some(body) = body {
                let mut cursor = body.walk();
                for inner in body.children(&mut cursor) {
                    collect_file_exports(&inner, bytes, out, imports);
                }
            }
        }
        _ => {}
    }
}

/// Extract the `'module'` specifier from an `export ... from '...'` statement.
/// Tree-sitter-typescript exposes it as a `source` field on the export_statement
/// node. The raw text includes the surrounding quotes, which we strip here.
pub(crate) fn extract_export_source_module(node: &Node, bytes: &[u8]) -> Option<String> {
    if let Some(src) = node.child_by_field_name("source") {
        if let Ok(raw) = src.utf8_text(bytes) {
            return Some(strip_quotes(raw));
        }
    }
    None
}

pub(crate) fn strip_quotes(s: &str) -> String {
    s.trim()
        .trim_start_matches('"')
        .trim_end_matches('"')
        .trim_start_matches('\'')
        .trim_end_matches('\'')
        .to_string()
}

pub(crate) fn find_named_child<'a>(node: &'a Node<'a>, kinds: &[&str]) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    for c in node.children(&mut cursor) {
        if kinds.iter().any(|k| *k == c.kind()) {
            return Some(c);
        }
    }
    None
}
