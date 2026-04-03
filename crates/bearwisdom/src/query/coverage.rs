//! Tree-sitter extraction coverage analysis.
//!
//! For each language in a project, parse every file with tree-sitter, walk all
//! CST nodes, and report which node kinds appear in real code vs. how many
//! symbols/refs the extractor produces. This identifies extraction gaps.

use crate::languages::{self, LanguageRegistry};
use crate::parser::languages as grammar_loader;
use crate::walker::{self, WalkedFile};
use rustc_hash::FxHashMap;
use serde::Serialize;
use std::path::Path;
use tree_sitter::Parser;

/// Coverage stats for a single language across all its files in a project.
#[derive(Debug, Serialize)]
pub struct LanguageCoverage {
    pub language: String,
    pub file_count: usize,
    pub total_named_nodes: u64,
    pub unique_node_kinds: usize,
    /// Number of node kinds that exist in the grammar but never appeared in any file.
    pub grammar_node_count: Option<usize>,
    pub symbols_extracted: u64,
    pub refs_extracted: u64,
    /// Node kind → occurrence count, sorted descending by frequency.
    pub node_kind_freq: Vec<NodeKindStat>,
}

#[derive(Debug, Serialize)]
pub struct NodeKindStat {
    pub kind: String,
    pub count: u64,
    /// True if at least one extracted symbol has start_line matching a node of this kind.
    pub produces_symbol: bool,
}

/// Run coverage analysis on a project.
pub fn analyze_coverage(project_root: &Path) -> Vec<LanguageCoverage> {
    let registry = languages::default_registry();

    // Walk files
    let files = match walker::walk(project_root) {
        Ok(f) => f,
        Err(_) => return vec![],
    };

    // Group files by language
    let mut by_language: FxHashMap<String, Vec<WalkedFile>> = FxHashMap::default();
    for file in files {
        by_language
            .entry(file.language.to_string())
            .or_default()
            .push(file);
    }

    let mut results: Vec<LanguageCoverage> = Vec::new();

    for (lang, files) in &by_language {
        // Skip non-code languages
        if matches!(
            lang.as_str(),
            "json" | "yaml" | "xml" | "markdown" | "toml" | "css" | "html" | "sql"
        ) {
            continue;
        }

        let grammar = registry.grammar(lang);
        if grammar.is_none() {
            continue;
        }
        let grammar = grammar.unwrap();

        let mut node_kind_counts: FxHashMap<String, u64> = FxHashMap::default();
        let mut total_named_nodes: u64 = 0;
        let mut total_symbols: u64 = 0;
        let mut total_refs: u64 = 0;
        let mut symbol_line_set: FxHashMap<String, Vec<u32>> = FxHashMap::default();

        for walked in files {
            let content = match std::fs::read_to_string(&walked.absolute_path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Parse with tree-sitter
            let mut parser = Parser::new();
            if parser.set_language(&grammar).is_err() {
                continue;
            }
            let tree = match parser.parse(&content, None) {
                Some(t) => t,
                None => continue,
            };

            // Walk ALL named nodes and count their kinds
            walk_named_nodes(tree.root_node(), &mut node_kind_counts, &mut total_named_nodes);

            // Run the extractor
            let plugin = registry.get(lang);
            let result = plugin.extract(&content, &walked.relative_path, lang);

            total_symbols += result.symbols.len() as u64;
            total_refs += result.refs.len() as u64;

            // Record which lines have symbols (for correlation)
            for sym in &result.symbols {
                symbol_line_set
                    .entry(walked.relative_path.clone())
                    .or_default()
                    .push(sym.start_line);
            }
        }

        // Build frequency list
        let mut freq: Vec<NodeKindStat> = node_kind_counts
            .into_iter()
            .map(|(kind, count)| NodeKindStat {
                kind,
                count,
                produces_symbol: false, // We'll refine this later
            })
            .collect();
        freq.sort_by(|a, b| b.count.cmp(&a.count));
        let unique = freq.len();

        results.push(LanguageCoverage {
            language: lang.clone(),
            file_count: files.len(),
            total_named_nodes,
            unique_node_kinds: unique,
            grammar_node_count: None,
            symbols_extracted: total_symbols,
            refs_extracted: total_refs,
            node_kind_freq: freq,
        });
    }

    results.sort_by(|a, b| b.file_count.cmp(&a.file_count));
    results
}

fn walk_named_nodes(
    node: tree_sitter::Node,
    counts: &mut FxHashMap<String, u64>,
    total: &mut u64,
) {
    if node.is_named() {
        *counts.entry(node.kind().to_string()).or_insert(0) += 1;
        *total += 1;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_named_nodes(child, counts, total);
    }
}
