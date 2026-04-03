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

#[derive(Debug, Serialize)]
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

            let mut parser = Parser::new();
            if parser.set_language(&grammar).is_err() {
                continue;
            }
            let tree = match parser.parse(&content, None) {
                Some(t) => t,
                None => continue,
            };

            // Collect all node occurrences with their lines
            let mut sym_nodes_by_line: FxHashMap<String, Vec<u32>> = FxHashMap::default();
            let mut ref_nodes_by_line: FxHashMap<String, Vec<u32>> = FxHashMap::default();

            walk_and_classify(
                tree.root_node(),
                &sym_kinds,
                &ref_kinds,
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
                } else if !has_rules {
                    -1.0 // no rules declared
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

fn walk_and_classify(
    node: tree_sitter::Node,
    sym_kinds: &FxHashSet<&str>,
    ref_kinds: &FxHashSet<&str>,
    sym_counts: &mut FxHashMap<String, u64>,
    ref_counts: &mut FxHashMap<String, u64>,
    structural_counts: &mut FxHashMap<String, u64>,
    sym_by_line: &mut FxHashMap<String, Vec<u32>>,
    ref_by_line: &mut FxHashMap<String, Vec<u32>>,
) {
    if node.is_named() {
        let kind = node.kind();
        let line = node.start_position().row as u32;

        if sym_kinds.contains(kind) {
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

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_and_classify(
            child,
            sym_kinds,
            ref_kinds,
            sym_counts,
            ref_counts,
            structural_counts,
            sym_by_line,
            ref_by_line,
        );
    }
}
