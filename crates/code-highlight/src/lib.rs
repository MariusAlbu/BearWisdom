//! Tree-sitter syntax highlighting: `(source, lang)` → a list of colored byte ranges.
//!
//! Stateless with respect to documents — the caller owns the buffer and the parse
//! lifecycle. This crate turns source text plus a language id into non-overlapping
//! [`Token`]s for a renderer to paint. The grammar and its bundled highlights query
//! come from [`code_grammars`]; the result is normalized to a small [`HighlightKind`]
//! vocabulary so a frontend maps each kind to a colour exactly once.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tree_sitter::{Query, QueryCursor, StreamingIterator};

use once_cell::sync::Lazy;
use serde::Serialize;
use tree_sitter_highlight::{Highlight, HighlightConfiguration, HighlightEvent, Highlighter};

/// Semantic highlight class — the stable contract a renderer maps to one colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HighlightKind {
    Keyword,
    Type,
    Func,
    Name,
    Prop,
    String,
    Comment,
    Operator,
    Number,
}

/// A highlighted span. `start`/`end` are byte offsets into the source string.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Token {
    pub start: u32,
    pub end: u32,
    pub kind: HighlightKind,
}

/// Recognized tree-sitter capture names paired with the [`HighlightKind`] they paint.
///
/// `tree-sitter-highlight` matches a capture to the longest entry here that is a dotted
/// prefix of it, so `keyword.control.return` resolves to `keyword`. Index order is the
/// configured highlight order: [`Highlight`]`.0` indexes into this table.
const RULES: &[(&str, HighlightKind)] = &[
    ("keyword", HighlightKind::Keyword),
    ("conditional", HighlightKind::Keyword),
    ("repeat", HighlightKind::Keyword),
    ("include", HighlightKind::Keyword),
    ("exception", HighlightKind::Keyword),
    ("boolean", HighlightKind::Keyword),
    ("constant.builtin", HighlightKind::Keyword),
    ("tag", HighlightKind::Keyword),
    ("type", HighlightKind::Type),
    ("type.builtin", HighlightKind::Type),
    ("constructor", HighlightKind::Type),
    ("namespace", HighlightKind::Type),
    ("constant", HighlightKind::Type),
    ("function", HighlightKind::Func),
    ("function.builtin", HighlightKind::Func),
    ("function.method", HighlightKind::Func),
    ("function.macro", HighlightKind::Func),
    ("method", HighlightKind::Func),
    ("variable", HighlightKind::Name),
    ("variable.builtin", HighlightKind::Name),
    ("variable.parameter", HighlightKind::Name),
    ("parameter", HighlightKind::Name),
    ("property", HighlightKind::Prop),
    ("field", HighlightKind::Prop),
    ("attribute", HighlightKind::Prop),
    ("string", HighlightKind::String),
    ("string.special", HighlightKind::String),
    ("character", HighlightKind::String),
    ("escape", HighlightKind::String),
    ("comment", HighlightKind::Comment),
    ("operator", HighlightKind::Operator),
    ("punctuation", HighlightKind::Operator),
    ("punctuation.bracket", HighlightKind::Operator),
    ("punctuation.delimiter", HighlightKind::Operator),
    ("punctuation.special", HighlightKind::Operator),
    ("number", HighlightKind::Number),
    ("float", HighlightKind::Number),
];

/// Capture names in `RULES` order, handed to `HighlightConfiguration::configure`.
static NAMES: Lazy<Vec<String>> = Lazy::new(|| RULES.iter().map(|(n, _)| (*n).to_string()).collect());

/// Per-language highlight configs, built and `configure`d once. `None` is cached for
/// languages with no grammar/query or a query that fails to compile, so a bad language
/// is not rebuilt on every call.
static CONFIGS: Lazy<Mutex<HashMap<String, Option<Arc<HighlightConfiguration>>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn kind_for(h: Highlight) -> HighlightKind {
    RULES[h.0].1
}

fn config_for(lang: &str) -> Option<Arc<HighlightConfiguration>> {
    let mut cache = CONFIGS.lock().expect("highlight config cache poisoned");
    if let Some(entry) = cache.get(lang) {
        return entry.clone();
    }
    let built = build_config(lang).map(Arc::new);
    cache.insert(lang.to_string(), built.clone());
    built
}

fn build_config(lang: &str) -> Option<HighlightConfiguration> {
    let language = code_grammars::get_language(lang)?;
    let query = code_grammars::highlights_query(lang)?;
    let mut config = HighlightConfiguration::new(language, lang, &query, "", "").ok()?;
    config.configure(&NAMES);
    Some(config)
}

/// Highlight `source` as `lang`, returning sorted, non-overlapping [`Token`]s covering
/// only the highlighted ranges (gaps render in the default text colour).
///
/// Returns an empty vec for an unknown language or a grammar with no usable highlights
/// query — never an error: unhighlightable input is simply plain text.
pub fn highlight(source: &str, lang: &str) -> Vec<Token> {
    if lang == "markdown" || lang == "md" {
        return highlight_markdown(source);
    }

    let config = match config_for(lang) {
        Some(c) => c,
        None => return Vec::new(),
    };

    let mut highlighter = Highlighter::new();
    let events = match highlighter.highlight(&config, source.as_bytes(), None, |_| None) {
        Ok(events) => events,
        Err(_) => return Vec::new(),
    };

    let mut tokens: Vec<Token> = Vec::new();
    let mut stack: Vec<Highlight> = Vec::new();
    for event in events {
        match event {
            Ok(HighlightEvent::HighlightStart(h)) => stack.push(h),
            Ok(HighlightEvent::HighlightEnd) => {
                stack.pop();
            }
            Ok(HighlightEvent::Source { start, end }) => {
                let Some(h) = stack.last() else { continue };
                let kind = kind_for(*h);
                // Coalesce a run of same-kind contiguous source into one span.
                if let Some(last) = tokens.last_mut() {
                    if last.end as usize == start && last.kind == kind {
                        last.end = end as u32;
                        continue;
                    }
                }
                tokens.push(Token { start: start as u32, end: end as u32, kind });
            }
            Err(_) => break,
        }
    }
    tokens
}

// ---------------------------------------------------------------------------
// Markdown-specific highlight path
// ---------------------------------------------------------------------------
//
// tree-sitter-md uses two grammars: block (parses structure) and inline (parses
// span-level content inside block leaf nodes). They must be run separately and
// their token streams merged before normalisation.

/// Capture-name prefix → [`HighlightKind`] for Markdown queries.
///
/// Longest dotted-prefix match: `text.strong.emphasis` resolves to `text.strong`.
const MARKDOWN_RULES: &[(&str, HighlightKind)] = &[
    ("text.title", HighlightKind::Type),
    ("text.strong", HighlightKind::Keyword),
    ("text.emphasis", HighlightKind::Name),
    ("text.literal", HighlightKind::String),
    ("text.uri", HighlightKind::Func),
    ("text.reference", HighlightKind::Prop),
    ("string.escape", HighlightKind::String),
    ("punctuation.delimiter", HighlightKind::Operator),
    ("punctuation.special", HighlightKind::Operator),
];

/// Longest dotted-prefix lookup against [`MARKDOWN_RULES`].
fn md_kind_for(capture_name: &str) -> Option<HighlightKind> {
    let mut best: Option<(usize, HighlightKind)> = None;
    for (prefix, kind) in MARKDOWN_RULES {
        if capture_name == *prefix || capture_name.starts_with(&format!("{prefix}.")) {
            let len = prefix.len();
            if best.map_or(true, |(b, _)| len > b) {
                best = Some((len, *kind));
            }
        }
    }
    best.map(|(_, k)| k)
}

/// Collect tokens by running `query` over `root`, appending into `out`.
fn collect_md_tokens(
    query: &Query,
    root: tree_sitter::Node<'_>,
    source: &[u8],
    out: &mut Vec<Token>,
) {
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, root, source);
    while let Some(m) = matches.next() {
        for cap in m.captures {
            let name = &query.capture_names()[cap.index as usize];
            if let Some(kind) = md_kind_for(name) {
                let r = cap.node.byte_range();
                out.push(Token { start: r.start as u32, end: r.end as u32, kind });
            }
        }
    }
}

/// Normalise a raw (possibly overlapping, unsorted) token list into a sorted,
/// non-overlapping sequence with contiguous same-kind spans coalesced.
///
/// Sort order: start ascending, then end DESCENDING so that wider spans (parent
/// nodes) sort before narrower ones with the same start. Overlap resolution is
/// greedy left-to-right: any token whose `start < last_end` is dropped.
fn normalise(mut tokens: Vec<Token>) -> Vec<Token> {
    // Wider span first on equal start — parent wins over child punctuation.
    tokens.sort_unstable_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));

    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    let mut last_end: u32 = 0;

    for tok in tokens {
        if tok.start < last_end {
            continue;
        }
        // Coalesce contiguous same-kind spans.
        if let Some(prev) = out.last_mut() {
            if prev.end == tok.start && prev.kind == tok.kind {
                prev.end = tok.end;
                last_end = tok.end;
                continue;
            }
        }
        last_end = tok.end;
        out.push(tok);
    }

    out
}

/// Collect the `included_ranges` needed to parse inline content for a block node
/// whose kind is `"inline"` or `"pipe_table_cell"`.
///
/// Named children of those nodes that are themselves structural (non-content)
/// are excluded from the inline parse range, mirroring what the two-grammar
/// parser does internally.
fn inline_ranges_for(node: tree_sitter::Node<'_>) -> Vec<tree_sitter::Range> {
    let full_range = node.range();
    let mut ranges = Vec::new();
    let mut start = full_range.start_byte;
    let start_point = full_range.start_point;

    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if child.is_named() {
                let child_range = child.range();
                if child_range.start_byte > start {
                    ranges.push(tree_sitter::Range {
                        start_byte: start,
                        start_point: if start == full_range.start_byte {
                            start_point
                        } else {
                            child_range.start_point
                        },
                        end_byte: child_range.start_byte,
                        end_point: child_range.start_point,
                    });
                }
                start = child_range.end_byte;
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    // Remaining slice after the last named child.
    if start < full_range.end_byte {
        ranges.push(tree_sitter::Range {
            start_byte: start,
            start_point: full_range.start_point,
            end_byte: full_range.end_byte,
            end_point: full_range.end_point,
        });
    }

    if ranges.is_empty() {
        ranges.push(full_range);
    }

    ranges
}

/// Collect all `inline` and `pipe_table_cell` nodes from a block tree by walking
/// it depth-first, returning them in document order.
fn collect_inline_nodes<'t>(root: tree_sitter::Node<'t>) -> Vec<tree_sitter::Node<'t>> {
    let mut out = Vec::new();
    let mut cursor = root.walk();
    let mut visited_children = false;

    loop {
        let node = cursor.node();
        let kind = node.kind();

        if !visited_children && (kind == "inline" || kind == "pipe_table_cell") {
            out.push(node);
            // Don't descend further into inline/table-cell nodes.
            if !cursor.goto_next_sibling() {
                if !cursor.goto_parent() {
                    break;
                }
                visited_children = true;
            }
            continue;
        }

        if !visited_children && cursor.goto_first_child() {
            visited_children = false;
        } else if cursor.goto_next_sibling() {
            visited_children = false;
        } else if cursor.goto_parent() {
            visited_children = true;
        } else {
            break;
        }
    }

    out
}

/// Highlight `source` as Markdown, returning sorted, non-overlapping [`Token`]s.
fn highlight_markdown(source: &str) -> Vec<Token> {
    let block_lang: tree_sitter::Language = tree_sitter_md::LANGUAGE.into();
    let inline_lang: tree_sitter::Language = tree_sitter_md::INLINE_LANGUAGE.into();

    let block_query = match Query::new(&block_lang, tree_sitter_md::HIGHLIGHT_QUERY_BLOCK) {
        Ok(q) => q,
        Err(_) => return Vec::new(),
    };
    let inline_query = match Query::new(&inline_lang, tree_sitter_md::HIGHLIGHT_QUERY_INLINE) {
        Ok(q) => q,
        Err(_) => return Vec::new(),
    };

    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&block_lang).is_err() {
        return Vec::new();
    }

    let block_tree = match parser.parse(source.as_bytes(), None) {
        Some(t) => t,
        None => return Vec::new(),
    };

    let bytes = source.as_bytes();
    let mut tokens: Vec<Token> = Vec::new();

    // Block-level highlights (headings, fenced code blocks, link destinations, etc.)
    collect_md_tokens(&block_query, block_tree.root_node(), bytes, &mut tokens);

    // Inline-level highlights: one parse per inline/table-cell node.
    if parser.set_language(&inline_lang).is_err() {
        return normalise(tokens);
    }

    for inline_node in collect_inline_nodes(block_tree.root_node()) {
        let ranges = inline_ranges_for(inline_node);
        if parser.set_included_ranges(&ranges).is_err() {
            continue;
        }
        let inline_tree = match parser.parse(bytes, None) {
            Some(t) => t,
            None => continue,
        };
        collect_md_tokens(&inline_query, inline_tree.root_node(), bytes, &mut tokens);
    }

    normalise(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_rust_keyword_and_string() {
        let toks = highlight("fn main() { let s = \"hi\"; }", "rust");
        assert!(!toks.is_empty(), "expected highlight tokens for rust");
        assert!(toks.iter().any(|t| t.kind == HighlightKind::Keyword), "expected a keyword");
        assert!(toks.iter().any(|t| t.kind == HighlightKind::String), "expected a string");
        // Tokens are ordered and non-overlapping.
        assert!(toks.windows(2).all(|w| w[0].end <= w[1].start), "tokens overlap or unsorted");
    }

    #[test]
    fn highlights_typescript() {
        let toks = highlight("const x: number = 1;", "typescript");
        assert!(toks.iter().any(|t| t.kind == HighlightKind::Keyword));
    }

    #[test]
    fn unknown_language_yields_no_tokens() {
        assert!(highlight("whatever it is", "klingon").is_empty());
    }

    // --- Markdown ---

    fn md_sorted_and_non_overlapping(toks: &[Token]) {
        assert!(
            toks.windows(2).all(|w| w[0].end <= w[1].start),
            "tokens are not sorted or overlap: {toks:?}",
        );
    }

    #[test]
    fn md_heading_yields_type_token() {
        let src = "# Title\n";
        let toks = highlight(src, "markdown");
        md_sorted_and_non_overlapping(&toks);
        // The heading text "Title" must be covered by a Type token.
        let title_start = src.find("Title").unwrap() as u32;
        let title_end = title_start + "Title".len() as u32;
        assert!(
            toks.iter().any(|t| t.kind == HighlightKind::Type && t.start <= title_start && t.end >= title_end),
            "expected a Type token covering 'Title'; got: {toks:?}",
        );
    }

    #[test]
    fn md_bold_yields_keyword_token() {
        let src = "a **bold** b\n";
        let toks = highlight(src, "markdown");
        md_sorted_and_non_overlapping(&toks);
        let bold_start = src.find("bold").unwrap() as u32;
        let bold_end = bold_start + "bold".len() as u32;
        // The Keyword token must overlap the interior "bold" text at minimum.
        assert!(
            toks.iter().any(|t| t.kind == HighlightKind::Keyword && t.start <= bold_start && t.end >= bold_end),
            "expected a Keyword token covering bold content; got: {toks:?}",
        );
    }

    #[test]
    fn md_inline_code_yields_string_token() {
        let src = "use `x` now\n";
        let toks = highlight(src, "markdown");
        md_sorted_and_non_overlapping(&toks);
        let x_start = src.find('`').unwrap() as u32;
        let x_end = src.rfind('`').unwrap() as u32 + 1;
        assert!(
            toks.iter().any(|t| t.kind == HighlightKind::String && t.start <= x_start && t.end >= x_end),
            "expected a String token covering inline code; got: {toks:?}",
        );
    }

    #[test]
    fn md_link_yields_func_or_prop_token() {
        let src = "[text](http://u)\n";
        let toks = highlight(src, "markdown");
        md_sorted_and_non_overlapping(&toks);
        // Must have at least a Func (URL) or Prop (link text) token.
        assert!(
            toks.iter().any(|t| t.kind == HighlightKind::Func || t.kind == HighlightKind::Prop),
            "expected a Func or Prop token for a link; got: {toks:?}",
        );
    }

    #[test]
    fn md_plain_text_does_not_panic() {
        let toks = highlight("plain text no markup\n", "markdown");
        md_sorted_and_non_overlapping(&toks);
        // No panic, valid (possibly empty) result.
        let _ = toks;
    }

    #[test]
    fn md_alias_md_dispatches() {
        // "md" alias must reach the same path as "markdown".
        let a = highlight("# H\n", "markdown");
        let b = highlight("# H\n", "md");
        assert_eq!(a.len(), b.len());
        for (ta, tb) in a.iter().zip(b.iter()) {
            assert_eq!(ta.start, tb.start);
            assert_eq!(ta.end, tb.end);
            assert_eq!(ta.kind, tb.kind);
        }
    }
}
