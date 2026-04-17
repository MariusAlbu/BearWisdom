//! Tree-sitter extraction coverage analysis.
//!
//! Measures how well our extractors handle the node kinds that MATTER — the
//! ones declared in `symbol_node_kinds()` and `ref_node_kinds()` on each
//! `LanguagePlugin`. These declarations come from the extraction rules
//! (`research/tree-sitter/languages/<lang>_rules.md`).
//!
//! Coverage = (matched nodes / expected nodes) for symbol-producing and
//! ref-producing node kinds separately.

use crate::languages::{self, LanguagePlugin, LanguageRegistry};
use crate::walker::{self, WalkedFile};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::Serialize;
use std::path::Path;
use tree_sitter::Parser;

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct LanguageCoverage {
    pub language: String,
    pub file_count: usize,

    // Raw extraction counts
    pub symbols_extracted: u64,
    pub refs_extracted: u64,

    // Rules-based coverage
    pub symbol_coverage: CoverageDetail,
    pub ref_coverage: CoverageDetail,

    /// Per-node-kind breakdown for symbol-producing kinds.
    pub symbol_kinds: Vec<NodeKindCoverage>,
    /// Per-node-kind breakdown for ref-producing kinds.
    pub ref_kinds: Vec<NodeKindCoverage>,
    /// Node kinds that appear frequently but are NOT in symbol/ref rules (structural).
    pub structural_top: Vec<NodeKindCount>,
}

#[derive(Debug, Serialize)]
pub struct CoverageDetail {
    /// Total CST nodes of expected kinds found in real code.
    pub expected_nodes: u64,
    /// How many of those produced an extraction (matched by line number).
    pub matched_nodes: u64,
    /// Coverage percentage (matched / expected * 100).
    pub percent: f64,
    /// Number of declared node kinds that appear in the project.
    pub declared_kinds_seen: usize,
    /// Number of declared node kinds total.
    pub declared_kinds_total: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeKindCoverage {
    pub kind: String,
    /// Total occurrences of this node kind in the project.
    pub occurrences: u64,
    /// How many occurrences produced a matching extraction.
    pub matched: u64,
    /// Coverage percentage for this specific node kind.
    pub percent: f64,
}

#[derive(Debug, Serialize)]
pub struct NodeKindCount {
    pub kind: String,
    pub count: u64,
}

// ---------------------------------------------------------------------------
// Analysis
// ---------------------------------------------------------------------------

pub fn analyze_coverage(project_root: &Path) -> Vec<LanguageCoverage> {
    let registry = languages::default_registry();

    let files = match walker::walk(project_root) {
        Ok(f) => f,
        Err(_) => return vec![],
    };

    let mut by_language: FxHashMap<String, Vec<WalkedFile>> = FxHashMap::default();
    for file in files {
        by_language
            .entry(file.language.to_string())
            .or_default()
            .push(file);
    }

    let mut results: Vec<LanguageCoverage> = Vec::new();

    for (lang, files) in &by_language {
        if matches!(lang.as_str(), "json" | "yaml" | "xml" | "markdown" | "toml") {
            continue;
        }

        let plugin = registry.get(lang);
        let grammar = match plugin.grammar(lang) {
            Some(g) => g,
            None => continue,
        };

        let sym_kinds: FxHashSet<&str> = plugin.symbol_node_kinds().iter().copied().collect();
        let ref_kinds: FxHashSet<&str> = plugin.ref_node_kinds().iter().copied().collect();
        let builtins: FxHashSet<&str> = plugin.keywords().iter().copied().collect();
        // (child_kind, parent_kind) pairs: skip counting the child as a ref site
        // when its direct parent has the given kind (e.g., Nix inner apply nodes
        // inside curried application chains).
        let nested_skip: FxHashSet<(&str, &str)> = plugin
            .nested_ref_skip_pairs()
            .iter()
            .copied()
            .collect();
        let has_rules = !sym_kinds.is_empty() || !ref_kinds.is_empty();

        // Per-node-kind counters
        let mut sym_kind_occurrences: FxHashMap<String, u64> = FxHashMap::default();
        let mut ref_kind_occurrences: FxHashMap<String, u64> = FxHashMap::default();
        let mut structural_counts: FxHashMap<String, u64> = FxHashMap::default();

        // Per-node-kind match counters (line-based correlation)
        let mut sym_kind_matched: FxHashMap<String, u64> = FxHashMap::default();
        let mut ref_kind_matched: FxHashMap<String, u64> = FxHashMap::default();

        let mut total_symbols: u64 = 0;
        let mut total_refs: u64 = 0;

        for walked in files {
            let content = match std::fs::read_to_string(&walked.absolute_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Use the per-file language ID to select the correct grammar variant.
            // For example, TypeScript uses LANGUAGE_TYPESCRIPT for .ts files but
            // LANGUAGE_TSX for .tsx files. Using the wrong grammar on JSX/TSX files
            // causes the parser to misinterpret JSX elements as TypeScript constructs
            // (e.g. `<Component>` parsed as a generic), producing spurious CST node
            // kinds that inflate the coverage denominator.
            //
            // The `plugin.extract()` call below already uses the correct grammar (it
            // checks the file extension), so we must match it here for accurate
            // correlation.
            let file_lang_id: &str = {
                // Infer the file-specific language variant from path extension.
                // This mirrors the logic in TypeScriptPlugin::extract().
                let rp = &walked.relative_path;
                if rp.ends_with(".tsx") || rp.ends_with(".jsx") {
                    // Force the TSX variant for files that need JSX support.
                    // If the plugin doesn't support "tsx", fall back to `lang`.
                    if plugin.grammar("tsx").is_some() { "tsx" } else { lang.as_str() }
                } else if rp.ends_with(".jsx") {
                    if plugin.grammar("jsx").is_some() { "jsx" } else { lang.as_str() }
                } else {
                    lang.as_str()
                }
            };
            let file_grammar = match plugin.grammar(file_lang_id) {
                Some(g) => g,
                None => grammar.clone(),
            };

            let mut parser = Parser::new();
            if parser.set_language(&file_grammar).is_err() {
                continue;
            }
            // 5-second timeout guards against grammars that loop on real-world code
            // (e.g. tree-sitter-zig on deeply nested constructs).
            parser.set_timeout_micros(5_000_000);
            let tree = match parser.parse(&content, None) {
                Some(t) => t,
                None => continue,
            };

            // Collect all node occurrences with their lines
            let mut sym_nodes_by_line: FxHashMap<String, Vec<u32>> = FxHashMap::default();
            let mut ref_nodes_by_line: FxHashMap<String, Vec<u32>> = FxHashMap::default();

            walk_and_classify(
                tree.root_node(),
                content.as_bytes(),
                &sym_kinds,
                &ref_kinds,
                &builtins,
                &nested_skip,
                &mut sym_kind_occurrences,
                &mut ref_kind_occurrences,
                &mut structural_counts,
                &mut sym_nodes_by_line,
                &mut ref_nodes_by_line,
            );

            // Run the extractor
            let result = plugin.extract(&content, &walked.relative_path, lang);
            total_symbols += result.symbols.len() as u64;
            total_refs += result.refs.len() as u64;

            // Correlate symbols: count how many nodes per (kind, line) exist,
            // then match up to that many extracted symbols on the same line.
            let mut sym_line_budget: FxHashMap<(String, u32), u64> = FxHashMap::default();
            for (kind, lines) in &sym_nodes_by_line {
                for &line in lines {
                    *sym_line_budget.entry((kind.clone(), line)).or_insert(0) += 1;
                }
            }
            for sym in &result.symbols {
                for kind in plugin.symbol_node_kinds() {
                    let key = (kind.to_string(), sym.start_line);
                    if let Some(budget) = sym_line_budget.get_mut(&key) {
                        if *budget > 0 {
                            *budget -= 1;
                            *sym_kind_matched.entry(kind.to_string()).or_insert(0) += 1;
                            break;
                        }
                    }
                }
            }

            // Correlate refs: same budget-based approach.
            let mut ref_line_budget: FxHashMap<(String, u32), u64> = FxHashMap::default();
            for (kind, lines) in &ref_nodes_by_line {
                for &line in lines {
                    *ref_line_budget.entry((kind.clone(), line)).or_insert(0) += 1;
                }
            }
            for eref in &result.refs {
                for kind in plugin.ref_node_kinds() {
                    let key = (kind.to_string(), eref.line);
                    if let Some(budget) = ref_line_budget.get_mut(&key) {
                        if *budget > 0 {
                            *budget -= 1;
                            *ref_kind_matched.entry(kind.to_string()).or_insert(0) += 1;
                            break;
                        }
                    }
                }
            }
        }

        // Build per-kind coverage stats
        let sym_kinds_detail: Vec<NodeKindCoverage> = plugin
            .symbol_node_kinds()
            .iter()
            .filter_map(|&kind| {
                let occ = *sym_kind_occurrences.get(kind).unwrap_or(&0);
                if occ == 0 {
                    return None;
                }
                let matched = *sym_kind_matched.get(kind).unwrap_or(&0);
                Some(NodeKindCoverage {
                    kind: kind.to_string(),
                    occurrences: occ,
                    matched,
                    percent: if occ > 0 {
                        matched as f64 / occ as f64 * 100.0
                    } else {
                        0.0
                    },
                })
            })
            .collect();

        let ref_kinds_detail: Vec<NodeKindCoverage> = plugin
            .ref_node_kinds()
            .iter()
            .filter_map(|&kind| {
                let occ = *ref_kind_occurrences.get(kind).unwrap_or(&0);
                if occ == 0 {
                    return None;
                }
                let matched = *ref_kind_matched.get(kind).unwrap_or(&0);
                Some(NodeKindCoverage {
                    kind: kind.to_string(),
                    occurrences: occ,
                    matched,
                    percent: if occ > 0 {
                        matched as f64 / occ as f64 * 100.0
                    } else {
                        0.0
                    },
                })
            })
            .collect();

        // Aggregate coverage
        let sym_expected: u64 = sym_kinds_detail.iter().map(|k| k.occurrences).sum();
        let sym_matched: u64 = sym_kinds_detail.iter().map(|k| k.matched).sum();
        let ref_expected: u64 = ref_kinds_detail.iter().map(|k| k.occurrences).sum();
        let ref_matched: u64 = ref_kinds_detail.iter().map(|k| k.matched).sum();

        let sym_seen = sym_kinds_detail.len();
        let ref_seen = ref_kinds_detail.len();

        // Top structural nodes (not in rules)
        let mut structural: Vec<NodeKindCount> = structural_counts
            .into_iter()
            .map(|(kind, count)| NodeKindCount { kind, count })
            .collect();
        structural.sort_by(|a, b| b.count.cmp(&a.count));
        structural.truncate(15);

        results.push(LanguageCoverage {
            language: lang.clone(),
            file_count: files.len(),
            symbols_extracted: total_symbols,
            refs_extracted: total_refs,
            symbol_coverage: CoverageDetail {
                expected_nodes: sym_expected,
                matched_nodes: sym_matched,
                percent: if sym_expected > 0 {
                    sym_matched as f64 / sym_expected as f64 * 100.0
                } else if sym_kinds.is_empty() {
                    // Language intentionally declares no symbol node kinds —
                    // its grammar does not distinguish definition nodes from
                    // invocation nodes by kind alone.  Report N/A (-1.0) so
                    // the aggregate checker treats this dimension as a pass.
                    -1.0
                } else if !has_rules {
                    -1.0 // no rules declared at all
                } else {
                    0.0
                },
                declared_kinds_seen: sym_seen,
                declared_kinds_total: plugin.symbol_node_kinds().len(),
            },
            ref_coverage: CoverageDetail {
                expected_nodes: ref_expected,
                matched_nodes: ref_matched,
                percent: if ref_expected > 0 {
                    ref_matched as f64 / ref_expected as f64 * 100.0
                } else if !has_rules {
                    -1.0
                } else {
                    0.0
                },
                declared_kinds_seen: ref_seen,
                declared_kinds_total: plugin.ref_node_kinds().len(),
            },
            symbol_kinds: sym_kinds_detail,
            ref_kinds: ref_kinds_detail,
            structural_top: structural,
        });
    }

    results.sort_by(|a, b| b.file_count.cmp(&a.file_count));
    results
}

/// Return the "effective" line for a ref-producing CST node.
///
/// For most nodes this is `node.start_position().row`, but for nodes where
/// the extractor consistently emits refs at a child's line (not the parent's),
/// we return that child's line so coverage correlation stays accurate.
///
/// Specifically:
/// - `method_invocation` → line of the `name` field (the method identifier),
///   because fluent chains like `obj\n  .method()` have the node starting at
///   the `obj` line while the ref is emitted at the `.method` identifier line.
fn effective_ref_line(node: &tree_sitter::Node) -> u32 {
    match node.kind() {
        "method_invocation" => {
            // The `name` field is the method identifier. Fall back to node start
            // if the field is absent (shouldn't happen in a valid parse).
            node.child_by_field_name("name")
                .map(|n| n.start_position().row as u32)
                .unwrap_or_else(|| node.start_position().row as u32)
        }
        _ => node.start_position().row as u32,
    }
}

fn walk_and_classify(
    node: tree_sitter::Node,
    src: &[u8],
    sym_kinds: &FxHashSet<&str>,
    ref_kinds: &FxHashSet<&str>,
    builtins: &FxHashSet<&str>,
    nested_skip: &FxHashSet<(&str, &str)>,
    sym_counts: &mut FxHashMap<String, u64>,
    ref_counts: &mut FxHashMap<String, u64>,
    structural_counts: &mut FxHashMap<String, u64>,
    sym_by_line: &mut FxHashMap<String, Vec<u32>>,
    ref_by_line: &mut FxHashMap<String, Vec<u32>>,
) {
    walk_and_classify_inner(
        node, src, sym_kinds, ref_kinds, builtins, nested_skip,
        sym_counts, ref_counts, structural_counts, sym_by_line, ref_by_line,
        None,
    );
}

fn walk_and_classify_inner(
    node: tree_sitter::Node,
    src: &[u8],
    sym_kinds: &FxHashSet<&str>,
    ref_kinds: &FxHashSet<&str>,
    builtins: &FxHashSet<&str>,
    nested_skip: &FxHashSet<(&str, &str)>,
    sym_counts: &mut FxHashMap<String, u64>,
    ref_counts: &mut FxHashMap<String, u64>,
    structural_counts: &mut FxHashMap<String, u64>,
    sym_by_line: &mut FxHashMap<String, Vec<u32>>,
    ref_by_line: &mut FxHashMap<String, Vec<u32>>,
    parent_kind: Option<&str>,
) {
    if node.is_named() {
        let kind = node.kind();
        // For method_invocation and similar chained-call nodes, the extractor emits
        // refs at the `name` child's line (the method name), not the node's start
        // (which may be the receiver object on a previous line in a fluent chain).
        // Use the name-child line when available so coverage correlation is accurate.
        let line = effective_ref_line(&node);

        // Skip constant/scope_resolution nodes that are structural children of
        // scope_resolution. E.g. in `Foo::Bar::Baz`, tree-sitter produces nested
        // scope_resolution nodes. The extractor emits one ref for the outermost node;
        // inner scope_resolution and constant children are structural parts of the
        // syntax, not independent ref-producing sites.
        let is_scope_child = parent_kind == Some("scope_resolution")
            && (kind == "constant" || kind == "scope_resolution");

        // Skip nodes declared by the plugin as nested structural children.
        // E.g. Nix inner apply_expression nodes inside curried application chains
        // (`f a b` → two applies, only outermost is a ref site).
        let is_nested_skip = parent_kind
            .map(|pk| nested_skip.contains(&(kind, pk)))
            .unwrap_or(false);

        if is_scope_child || is_nested_skip {
            // Don't count this node as a ref — it's structural part of a chain.
            // Still count it as structural for info purposes.
            if !sym_kinds.contains(kind) && !ref_kinds.contains(kind) {
                *structural_counts.entry(kind.to_string()).or_insert(0) += 1;
            }
        } else {
            // Skip builtin type_identifiers from the count — extractors correctly
            // don't emit TypeRef for these, so including them inflates the denominator.
            let is_type_id = kind == "type_identifier" || kind == "user_type"
                || kind == "named_type" || kind == "constant";
            if is_type_id && !builtins.is_empty() {
                let text = node.utf8_text(src).unwrap_or("");
                // For qualified types like "std.io.Result", check the last segment
                let name = text.rsplit(&['.', ':', '\\'][..]).next().unwrap_or(text);
                if builtins.contains(name) {
                    // Don't count this node — it's a builtin that extractors correctly skip
                } else if ref_kinds.contains(kind) {
                    *ref_counts.entry(kind.to_string()).or_insert(0) += 1;
                    ref_by_line.entry(kind.to_string()).or_default().push(line);
                }
            } else if sym_kinds.contains(kind) {
                *sym_counts.entry(kind.to_string()).or_insert(0) += 1;
                sym_by_line
                    .entry(kind.to_string())
                    .or_default()
                    .push(line);
            } else if ref_kinds.contains(kind) {
                *ref_counts.entry(kind.to_string()).or_insert(0) += 1;
                ref_by_line
                    .entry(kind.to_string())
                    .or_default()
                    .push(line);
            } else {
                *structural_counts.entry(kind.to_string()).or_insert(0) += 1;
            }
        }
    }

    let node_kind = if node.is_named() { Some(node.kind()) } else { None };
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_and_classify_inner(
            child,
            src,
            sym_kinds,
            ref_kinds,
            builtins,
            nested_skip,
            sym_counts,
            ref_counts,
            structural_counts,
            sym_by_line,
            ref_by_line,
            node_kind,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn debug_measure_all_language_coverage() {
        // (project_path, language_id)
        let projects: &[(&str, &str)] = &[
            ("F:/Work/Projects/TestProjects/java-spring-petclinic", "java"),
            ("F:/Work/Projects/TestProjects/react-calcom", "typescript"),
            ("F:/Work/Projects/TestProjects/ruby-discourse", "ruby"),
            ("F:/Work/Projects/TestProjects/scala-lila", "scala"),
            ("F:/Work/Projects/TestProjects/swift-icecubes", "swift"),
            ("F:/Work/Projects/TestProjects/elixir-plausible", "elixir"),
            ("F:/Work/Projects/TestProjects/dart-aidea", "dart"),
            ("F:/Work/Projects/TestProjects/go-gitea", "go"),
            ("F:/Work/Projects/TestProjects/php-monica", "php"),
            ("F:/Work/Projects/TestProjects/kotlin-komga", "kotlin"),
            ("F:/Work/Projects/TestProjects/rust-lemmy", "rust"),
            ("F:/Work/Projects/TestProjects/eShop", "csharp"),
        ];

        for (project, lang_filter) in projects {
            let path = Path::new(project);
            if !path.exists() {
                eprintln!("SKIP (not found): {} [{}]", project, lang_filter);
                continue;
            }
            let results = analyze_coverage(path);
            let cov = results.iter().find(|c| c.language == *lang_filter);
            match cov {
                None => eprintln!("NO DATA: {} [{}]", project, lang_filter),
                Some(c) => {
                    let sym_ok = c.symbol_coverage.percent < 0.0 || c.symbol_coverage.percent >= 95.0;
                    let ref_ok = c.ref_coverage.percent < 0.0 || c.ref_coverage.percent >= 95.0;
                    let sym_flag = if sym_ok { "✓" } else { "✗" };
                    let ref_flag = if ref_ok { "✓" } else { "✗" };
                    eprintln!(
                        "{:<12} sym: {:>6.1}% {} | ref: {:>6.1}% {} | files: {}",
                        lang_filter,
                        c.symbol_coverage.percent.max(0.0),
                        sym_flag,
                        c.ref_coverage.percent.max(0.0),
                        ref_flag,
                        c.file_count
                    );
                    if !sym_ok {
                        let mut sk = c.symbol_kinds.clone();
                        sk.sort_by(|a, b| a.percent.partial_cmp(&b.percent).unwrap());
                        for k in sk.iter().take(3) {
                            eprintln!("  SYM GAP  {}: {:.1}% miss={}", k.kind, k.percent, k.occurrences - k.matched);
                        }
                    }
                    if !ref_ok {
                        let mut rk = c.ref_kinds.clone();
                        rk.sort_by(|a, b| a.percent.partial_cmp(&b.percent).unwrap());
                        for k in rk.iter().take(3) {
                            eprintln!("  REF GAP  {}: {:.1}% miss={}", k.kind, k.percent, k.occurrences - k.matched);
                        }
                    }
                }
            }
        }
    }
}
