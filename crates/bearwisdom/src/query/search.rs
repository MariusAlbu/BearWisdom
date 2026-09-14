// =============================================================================
// query/search.rs  —  FTS5 full-text symbol search
//
// Uses the `symbols_fts` FTS5 virtual table (created in db/schema.rs) to do
// BM25-ranked full-text search across:
//   • symbol names       (e.g. "GetById", "CatalogService")
//   • qualified names    (e.g. "Catalog.CatalogService.GetById")
//   • signatures         (e.g. "Task<CatalogItem> GetById(int id)")
//   • doc comments       (e.g. "Returns the catalog item with the given ID")
//
// FTS5 'rank' column:
//   SQLite FTS5 returns a negative rank (lower = better match).  We negate it
//   before returning so callers see positive scores with higher = better.
//
// Query syntax (passed straight to FTS5):
//   • Simple word:  "catalog" — matches any of the four indexed columns.
//   • Prefix:       "catalog*" — prefix match.
//   • Phrase:       '"get catalog"' — exact phrase.
//   • Column scope: "name:GetById" — match only the name column.
//   See https://www.sqlite.org/fts5.html#full_text_query_syntax for full syntax.
//
// Fallback:
//   If the FTS5 query returns no results (e.g. if symbols_fts is empty because
//   the database predates the FTS triggers), we also attempt a LIKE-based fuzzy
//   fallback search on `symbols.name` and `symbols.qualified_name`.
// =============================================================================

use crate::db::Database;
use crate::query::QueryResult;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

/// One search result from the FTS5 index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub name: String,
    pub qualified_name: String,
    /// Symbol kind string, e.g. "class", "method".
    pub kind: String,
    pub file_path: String,
    /// 1-based line number of the symbol definition.
    pub start_line: u32,
    pub signature: Option<String>,
    /// BM25 relevance score — higher is a better match.
    /// FTS5 returns negative rank; we negate it here for a natural ordering.
    pub score: f64,
}

/// Restricts semantic search to the kind of evidence a caller needs. Keeping
/// this separate from the query text prevents regression tests and production
/// declarations from competing for the same small result budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchResultFilter {
    All,
    Definitions,
    Tests,
}

/// Build a forgiving fallback for the way coding agents naturally phrase
/// symbol searches. FTS5 treats whitespace-separated terms as AND, which is
/// useful for deliberate FTS expressions but surprising for queries that list
/// several candidate identifiers. When the exact query has no hits, retry the
/// distinct plain terms as OR alternatives.
fn fallback_or_query(query: &str) -> Option<String> {
    let mut seen = std::collections::HashSet::new();
    let terms: Vec<String> = query
        .split_whitespace()
        .filter_map(|raw| {
            let term = raw
                .trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != ':' && c != '-');
            if term.len() < 2
                || matches!(term.to_ascii_uppercase().as_str(), "AND" | "OR" | "NOT")
                || !seen.insert(term.to_ascii_lowercase())
            {
                return None;
            }
            Some(format!("\"{}\"", term.replace('"', "\"\"")))
        })
        .collect();

    (terms.len() > 1).then(|| terms.join(" OR "))
}

fn identifier_terms(query: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    query
        .split_whitespace()
        .filter_map(|raw| {
            let term = raw.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
            if term.len() < 2 || !seen.insert(term.to_ascii_lowercase()) {
                return None;
            }
            Some(term.to_owned())
        })
        .collect()
}

static QUERY_STOP_WORDS: std::sync::LazyLock<HashSet<&'static str>> =
    std::sync::LazyLock::new(|| {
        [
            "a",
            "an",
            "and",
            "answer",
            "are",
            "as",
            "at",
            "be",
            "behavior",
            "by",
            "caller",
            "callers",
            "code",
            "defining",
            "definition",
            "direct",
            "do",
            "does",
            "exact",
            "file",
            "files",
            "find",
            "for",
            "from",
            "function",
            "functions",
            "generic",
            "has",
            "have",
            "how",
            "if",
            "implementation",
            "in",
            "is",
            "it",
            "main",
            "method",
            "methods",
            "name",
            "names",
            "new",
            "no",
            "not",
            "of",
            "on",
            "or",
            "outside",
            "question",
            "questions",
            "repository",
            "set",
            "show",
            "specific",
            "symbol",
            "symbols",
            "that",
            "the",
            "this",
            "to",
            "type",
            "used",
            "was",
            "were",
            "what",
            "when",
            "where",
            "which",
            "who",
            "why",
            "will",
            "with",
        ]
        .into_iter()
        .collect()
    });

fn match_key(term: &str) -> String {
    let lower = term.to_ascii_lowercase();
    if lower.contains('_') {
        return lower;
    }
    let singular = if lower.len() > 4 {
        lower.strip_suffix('s').unwrap_or(&lower)
    } else {
        &lower
    };
    // Keep short role words such as `factory` intact for their alias table,
    // while normalizing common inflections such as `indexed` -> `index`.
    if singular.len() >= 7 && query_aliases(singular).is_empty() {
        singular.chars().take(5).collect()
    } else {
        singular.to_owned()
    }
}

fn query_aliases(term: &str) -> &'static [&'static str] {
    match term {
        "quali" => &["qname"],
        "factory" => &["create", "construct", "build", "make", "new", "all"],
        "regis" => &["registered", "catalog", "all"],
        "dispa" => &["dispatch", "route", "via", "delegate"],
        _ => &[],
    }
}

fn lexical_tokens(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut tokens = Vec::new();
    let mut current = String::new();
    for (index, &ch) in chars.iter().enumerate() {
        if !ch.is_alphanumeric() {
            if !current.is_empty() {
                tokens.push(current.to_ascii_lowercase());
                current.clear();
            }
            continue;
        }
        let previous = index.checked_sub(1).and_then(|i| chars.get(i)).copied();
        let next = chars.get(index + 1).copied();
        let camel_boundary = !current.is_empty()
            && ch.is_uppercase()
            && (previous.is_some_and(|c| c.is_lowercase() || c.is_numeric())
                || (previous.is_some_and(|c| c.is_uppercase())
                    && next.is_some_and(char::is_lowercase)));
        if camel_boundary {
            tokens.push(current.to_ascii_lowercase());
            current.clear();
        }
        current.push(ch);
    }
    if !current.is_empty() {
        tokens.push(current.to_ascii_lowercase());
    }
    tokens
}

fn token_matches_term(token: &str, term: &str) -> bool {
    token.contains(term)
        || query_aliases(term)
            .iter()
            .any(|alias| token == *alias || token.starts_with(alias))
}

fn contains_term(text: &str, term: &str) -> bool {
    text.contains(term)
        || query_aliases(term).iter().any(|alias| {
            lexical_tokens(text)
                .iter()
                .any(|token| token == alias || (alias.len() >= 4 && token.starts_with(alias)))
        })
}

fn adjacent_term_pairs(text: &str, terms: &[String]) -> u32 {
    let tokens = lexical_tokens(text);
    tokens
        .windows(2)
        .filter(|pair| {
            terms.iter().enumerate().any(|(left_index, left)| {
                token_matches_term(&pair[0], left)
                    && terms.iter().enumerate().any(|(right_index, right)| {
                        left_index != right_index && token_matches_term(&pair[1], right)
                    })
            })
        })
        .count() as u32
}

/// Terms suitable for natural-language symbol and path ranking.  Long words
/// use a short lexical root so `resolution`, `resolve`, and `resolver` can
/// contribute to the same result without language-specific synonym tables.
pub(super) fn ranking_terms(query: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut terms: Vec<(usize, usize, String)> = query
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .enumerate()
        .filter_map(|(index, raw)| {
            if raw.len() < 3 {
                return None;
            }
            let lower = raw.to_ascii_lowercase();
            if QUERY_STOP_WORDS.contains(lower.as_str()) {
                return None;
            }
            let key = match_key(&lower);
            if key.len() < 3 || !seen.insert(key.clone()) {
                return None;
            }
            let specificity = raw.len()
                + usize::from(raw.contains('_')) * 12
                + usize::from(raw.chars().any(char::is_uppercase)) * 4;
            Some((index, specificity, key))
        })
        .collect();

    if terms.len() > 16 {
        terms.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        terms.truncate(16);
        terms.sort_by_key(|term| term.0);
    }
    terms.into_iter().map(|(_, _, term)| term).collect()
}

fn symbol_concepts(name: &str) -> HashSet<String> {
    lexical_tokens(name)
        .into_iter()
        .filter(|token| token.len() >= 3 && !QUERY_STOP_WORDS.contains(token.as_str()))
        .map(|token| match_key(&token))
        .collect()
}

fn names_share_concepts(left: &str, right: &str, minimum: usize) -> bool {
    let left = symbol_concepts(left);
    symbol_concepts(right)
        .into_iter()
        .filter(|concept| left.contains(concept))
        .take(minimum)
        .count()
        >= minimum
}

fn escape_like(term: &str) -> String {
    term.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn fts_term_query(term: &str) -> String {
    let mut alternatives = vec![term];
    alternatives.extend_from_slice(query_aliases(term));
    alternatives
        .into_iter()
        .map(|alternative| format!("\"{alternative}\"*"))
        .collect::<Vec<_>>()
        .join(" OR ")
}

fn push_unique(
    out: &mut Vec<SearchResult>,
    candidates: impl IntoIterator<Item = SearchResult>,
    seen: &mut HashSet<(String, u32, String)>,
    limit: usize,
) {
    for candidate in candidates {
        if out.len() >= limit {
            break;
        }
        let key = (
            candidate.file_path.clone(),
            candidate.start_line,
            candidate.name.clone(),
        );
        if seen.insert(key) {
            out.push(candidate);
        }
    }
}

fn looks_like_code_anchor(term: &str) -> bool {
    term.contains('_')
        || term.contains("::")
        || term.contains('.')
        || (term.chars().any(char::is_lowercase) && term.chars().skip(1).any(char::is_uppercase))
}

pub(super) fn useful_result_kind(kind: &str) -> bool {
    !matches!(
        kind,
        "parameter" | "variable" | "field" | "property" | "enum_member" | "namespace" | "module"
    )
}

fn kind_rank(kind: &str) -> u8 {
    match kind {
        "test" => 0,
        "function" | "method" => 1,
        "trait" | "interface" | "struct" | "class" | "enum" | "type_alias" => 2,
        "module" => 3,
        _ => 4,
    }
}

fn natural_relevance(result: &SearchResult, terms: &[String]) -> f64 {
    let name = result.name.to_ascii_lowercase();
    let path = result.file_path.to_ascii_lowercase();
    let mut matches = 0u32;
    let mut name_matches = 0u32;
    for term in terms {
        if contains_term(&name, term) {
            matches += 1;
            name_matches += 1;
        } else if contains_term(&path, term) {
            matches += 1;
        }
    }
    // Name and path are alternative evidence for concept adjacency. Summing
    // both lets directory names such as `module_resolution` double-count the
    // same match and bury a denser symbol such as `resolve_via_module_resolver`.
    let adjacent =
        adjacent_term_pairs(&result.name, terms).max(adjacent_term_pairs(&result.file_path, terms));
    let name_tokens = lexical_tokens(&result.name);
    let matched_name_tokens = name_tokens
        .iter()
        .filter(|token| terms.iter().any(|term| token_matches_term(token, term)))
        .count();
    let type_like = matches!(
        result.kind.as_str(),
        "trait" | "interface" | "struct" | "class" | "enum" | "type_alias"
    );
    let name_precision_bonus = if name_tokens.is_empty() {
        0.0
    } else {
        60.0 * matched_name_tokens as f64 / name_tokens.len() as f64
    };
    let name_coverage_bonus = 80.0 * matched_name_tokens.min(4) as f64;
    let short_generic_name_penalty = if name_tokens.len() == 1 && name_tokens[0].len() <= 6 {
        400.0
    } else {
        0.0
    };
    let sparse_name_penalty = if terms.len() >= 3 {
        match name_matches {
            0 => 500.0,
            1 => 200.0,
            _ => 0.0,
        }
    } else {
        0.0
    };
    let kind_query_bonus = if terms.iter().any(|term| term == &match_key(&result.kind)) {
        300.0
    } else {
        0.0
    };
    let exact_type_family_bonus = if type_like
        && name_tokens.len() == 2
        && name_tokens
            .iter()
            .all(|token| terms.iter().any(|term| token_matches_term(token, term)))
    {
        75.0
    } else {
        0.0
    };
    let factory_signature_bonus = if terms.iter().any(|term| term == "factory") {
        result
            .signature
            .as_deref()
            .and_then(|signature| {
                let start = signature.find('(')?;
                let end = signature[start + 1..].find(')')? + start + 1;
                let parameters = signature[start + 1..end].trim();
                Some(if parameters.is_empty() {
                    0.0
                } else {
                    15.0 * (parameters.matches(',').count() + 1).min(3) as f64
                })
            })
            .unwrap_or(0.0)
    } else {
        0.0
    };
    let wants_tests = terms.iter().any(|term| term == "test" || term == "regre");
    let path_tokens = lexical_tokens(&path);
    let in_test_path = path_tokens
        .iter()
        .any(|token| token == "test" || token == "tests");
    let in_test_scope = result
        .qualified_name
        .split(['.', ':', '/', '\\'])
        .any(|segment| {
            segment.eq_ignore_ascii_case("test") || segment.eq_ignore_ascii_case("tests")
        });
    let kind_bonus = if wants_tests {
        5u8.saturating_sub(kind_rank(&result.kind))
    } else {
        match result.kind.as_str() {
            "function" => 6,
            "method" => 5,
            "trait" | "interface" | "struct" | "class" | "enum" | "type_alias" => 6,
            "module" => 3,
            "test" => 2,
            _ => 1,
        }
    };
    let test_penalty = if !wants_tests && (result.kind == "test" || in_test_path || in_test_scope) {
        600.0
    } else {
        0.0
    };
    f64::from(matches * 100 + name_matches * 20 + adjacent * 140)
        + f64::from(kind_bonus)
        + name_precision_bonus
        + name_coverage_bonus
        + kind_query_bonus
        + exact_type_family_bonus
        + factory_signature_bonus
        - test_penalty
        - short_generic_name_penalty
        - sparse_name_penalty
}

pub(super) fn rank_for_task(result: &SearchResult, terms: &[String]) -> f64 {
    natural_relevance(result, terms)
}

fn result_family(name: &str, kind: &str) -> (String, usize) {
    let tokens = lexical_tokens(name);
    if kind != "test"
        && tokens.iter().any(|token| token.starts_with("resolv"))
        && tokens.iter().any(|token| token == "via")
        && tokens.iter().any(|token| token == "resolver")
    {
        return ("resolve_via_resolver".to_owned(), 1);
    }
    if matches!(
        kind,
        "trait" | "interface" | "struct" | "class" | "enum" | "type_alias"
    ) {
        if tokens.len() >= 2 {
            return (tokens[tokens.len() - 2..].join("_"), 2);
        }
    }
    if tokens.len() >= 2
        && query_aliases("factory")
            .iter()
            .any(|alias| tokens[0] == *alias)
    {
        return (tokens[..2].join("_"), 2);
    }
    let normalized = name.to_ascii_lowercase();
    if let Some(base) = normalized.strip_suffix("_indexed") {
        (base.to_owned(), 1)
    } else {
        (normalized, 1)
    }
}

fn push_diverse_result(
    result: SearchResult,
    file_counts: &mut HashMap<String, usize>,
    name_counts: &mut HashMap<String, usize>,
    family_counts: &mut HashMap<String, usize>,
    out: &mut Vec<SearchResult>,
    limit: usize,
) -> bool {
    let normalized_name = result.name.to_ascii_lowercase();
    if name_counts.get(&normalized_name).copied().unwrap_or(0) >= 1 {
        return false;
    }
    let (family, family_limit) = result_family(&result.name, &result.kind);
    if family_counts.get(&family).copied().unwrap_or(0) >= family_limit {
        return false;
    }
    let count = file_counts.entry(result.file_path.clone()).or_default();
    if *count >= 3 {
        return false;
    }
    *count += 1;
    *name_counts.entry(normalized_name).or_default() += 1;
    *family_counts.entry(family).or_default() += 1;
    out.push(result);
    out.len() >= limit
}

fn ranked_term_results(
    conn: &rusqlite::Connection,
    query: &str,
    limit: usize,
    include_signature: bool,
) -> anyhow::Result<Vec<SearchResult>> {
    struct Candidate {
        result: SearchResult,
        fts_terms: HashSet<String>,
    }

    let terms = ranking_terms(query);
    if terms.len() < 2 {
        return Ok(Vec::new());
    }

    let sig_col = if include_signature {
        "s.signature"
    } else {
        "NULL"
    };
    let fts_sql = format!(
        "SELECT s.name, s.qualified_name, s.kind, f.path, s.line, {sig_col},
                MIN((SELECT COUNT(*) FROM edges e WHERE e.source_id = s.id), 10) * 6.0
         FROM symbols_fts fts
         JOIN symbols s ON s.id = fts.rowid
         JOIN files f ON f.id = s.file_id
         WHERE symbols_fts MATCH ?1
           AND s.origin = 'internal'
           AND s.kind NOT IN ('parameter', 'variable', 'field', 'property', 'enum_member', 'namespace')
           AND f.path NOT LIKE '.claude/worktrees/%'
           AND f.path NOT LIKE '.codex/worktrees/%'
         ORDER BY fts.rank
         LIMIT ?2"
    );
    let path_sql = format!(
        "WITH candidates AS (
             SELECT s.name, s.qualified_name, s.kind, f.path, s.line, {sig_col},
                    MIN((SELECT COUNT(*) FROM edges e WHERE e.source_id = s.id), 10) * 6.0 AS structure_score,
                    row_number() OVER (
                        PARTITION BY f.id
                        ORDER BY CASE s.kind
                            WHEN 'test' THEN 0
                            WHEN 'function' THEN 1
                            WHEN 'method' THEN 1
                            WHEN 'trait' THEN 2
                            WHEN 'interface' THEN 2
                            WHEN 'struct' THEN 2
                            WHEN 'class' THEN 2
                            ELSE 3 END,
                            s.line
                    ) AS file_rank
             FROM files f JOIN symbols s ON s.file_id = f.id
             WHERE f.origin = 'internal'
               AND lower(f.path) LIKE ?1 ESCAPE '\\'
               AND s.kind NOT IN ('parameter', 'variable', 'field', 'property', 'enum_member', 'namespace')
               AND f.path NOT LIKE '.claude/worktrees/%'
               AND f.path NOT LIKE '.codex/worktrees/%'
         )
         SELECT name, qualified_name, kind, path, line, {signature}, structure_score
         FROM candidates
         WHERE file_rank <= 20
         ORDER BY path, file_rank
         LIMIT ?2",
        signature = if include_signature { "signature" } else { "NULL" }
    );
    let name_coverage = terms
        .iter()
        .map(|term| {
            match term.as_str() {
                "quali" => "CASE WHEN instr(lower(s.name), 'quali') > 0 OR instr(lower(s.name), 'qname') > 0 THEN 1 ELSE 0 END".to_owned(),
                _ => format!(
                    "CASE WHEN instr(lower(s.name), '{term}') > 0 THEN 1 ELSE 0 END"
                ),
            }
        })
        .collect::<Vec<_>>()
        .join(" + ");
    let name_overlap_sql = format!(
        "WITH scored AS (
             SELECT s.name, s.qualified_name, s.kind, f.path, s.line,
                    {sig_col} AS signature,
                    MIN((SELECT COUNT(*) FROM edges e WHERE e.source_id = s.id), 10) * 6.0 AS structure_score,
                    ({name_coverage}) AS coverage
             FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE s.origin = 'internal'
               AND s.kind NOT IN ('parameter', 'variable', 'field', 'property', 'enum_member', 'namespace')
               AND f.path NOT LIKE '.claude/worktrees/%'
               AND f.path NOT LIKE '.codex/worktrees/%'
         )
         SELECT name, qualified_name, kind, path, line, signature, structure_score
         FROM scored
         WHERE coverage >= 2
         ORDER BY coverage DESC,
                  CASE kind WHEN 'function' THEN 0 WHEN 'method' THEN 0
                            WHEN 'trait' THEN 1 WHEN 'interface' THEN 1
                            WHEN 'struct' THEN 1 WHEN 'class' THEN 1
                            WHEN 'test' THEN 2 ELSE 3 END,
                  length(name),
                  path, line
         LIMIT 2000"
    );

    let collect = |stmt: &mut rusqlite::Statement<'_>, value: &str, cap: usize| {
        let rows = stmt.query_map(rusqlite::params![value, cap as i64], |row| {
            Ok(SearchResult {
                name: row.get(0)?,
                qualified_name: row.get(1)?,
                kind: row.get(2)?,
                file_path: row.get(3)?,
                start_line: row.get(4)?,
                signature: row.get(5)?,
                score: row.get(6)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    };

    let per_term_fts = 100;
    let per_term_paths = 300;
    let mut fts = conn.prepare(&fts_sql)?;
    let mut paths = conn.prepare(&path_sql)?;
    let mut candidates: HashMap<(String, u32, String), Candidate> = HashMap::new();
    let mut name_overlap = conn.prepare(&name_overlap_sql)?;
    let rows = name_overlap.query_map([], |row| {
        Ok(SearchResult {
            name: row.get(0)?,
            qualified_name: row.get(1)?,
            kind: row.get(2)?,
            file_path: row.get(3)?,
            start_line: row.get(4)?,
            signature: row.get(5)?,
            score: row.get(6)?,
        })
    })?;
    for result in rows {
        let result = result?;
        candidates
            .entry((
                result.file_path.clone(),
                result.start_line,
                result.name.clone(),
            ))
            .or_insert(Candidate {
                result,
                fts_terms: HashSet::new(),
            });
    }
    for term in &terms {
        let fts_term = fts_term_query(term);
        for result in collect(&mut fts, &fts_term, per_term_fts)? {
            let key = (
                result.file_path.clone(),
                result.start_line,
                result.name.clone(),
            );
            candidates
                .entry(key)
                .and_modify(|candidate| {
                    candidate.fts_terms.insert(term.clone());
                })
                .or_insert_with(|| Candidate {
                    result,
                    fts_terms: HashSet::from([term.clone()]),
                });
        }
        let pattern = format!("%{}%", escape_like(term));
        for result in collect(&mut paths, &pattern, per_term_paths)? {
            candidates
                .entry((
                    result.file_path.clone(),
                    result.start_line,
                    result.name.clone(),
                ))
                .or_insert(Candidate {
                    result,
                    fts_terms: HashSet::new(),
                });
        }
    }

    let mut ranked: Vec<SearchResult> = candidates
        .into_values()
        .filter(|candidate| useful_result_kind(&candidate.result.kind))
        .filter(|candidate| !ranking_terms(&candidate.result.name).is_empty())
        .filter(|candidate| {
            let name = candidate.result.name.to_ascii_lowercase();
            terms.iter().any(|term| contains_term(&name, term))
        })
        .map(|mut candidate| {
            let structure_score = candidate.result.score;
            let base_score = natural_relevance(&candidate.result, &terms);
            let name = candidate.result.name.to_ascii_lowercase();
            let path = candidate.result.file_path.to_ascii_lowercase();
            let fts_only_matches = candidate
                .fts_terms
                .iter()
                .filter(|term| !contains_term(&name, term) && !contains_term(&path, term))
                .count() as f64;
            // Registry and factory implementations tend to construct or
            // delegate to several components, while convenience wrappers have
            // only one or two outgoing edges. Give that neutral graph signal
            // enough weight to distinguish the implementation from a
            // narrowly named wrapper.
            let structure_weight = if terms
                .iter()
                .any(|term| term == "factory" || term == "regis")
            {
                3.0
            } else {
                1.0
            };
            candidate.result.score =
                base_score + fts_only_matches * 80.0 + structure_score * structure_weight;
            candidate.result
        })
        .filter(|result| result.score >= 200.0)
        .collect();
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| kind_rank(&a.kind).cmp(&kind_rank(&b.kind)))
            .then_with(|| a.file_path.cmp(&b.file_path))
            .then_with(|| a.start_line.cmp(&b.start_line))
    });

    let test_groups: HashMap<String, Vec<SearchResult>> = ranked
        .iter()
        .filter(|result| result.kind == "test")
        .fold(HashMap::new(), |mut groups, result| {
            groups
                .entry(result.file_path.clone())
                .or_default()
                .push(result.clone());
            groups
        });
    let mut file_counts: HashMap<String, usize> = HashMap::new();
    let mut name_counts: HashMap<String, usize> = HashMap::new();
    let mut family_counts: HashMap<String, usize> = HashMap::new();
    let mut diverse = Vec::with_capacity(limit);
    for result in ranked {
        let anchor = result.clone();
        if push_diverse_result(
            result,
            &mut file_counts,
            &mut name_counts,
            &mut family_counts,
            &mut diverse,
            limit,
        ) {
            break;
        }
        if anchor.kind == "test" {
            if let Some(siblings) = test_groups.get(&anchor.file_path) {
                for sibling in siblings {
                    if sibling.name == anchor.name
                        || !names_share_concepts(&anchor.name, &sibling.name, 2)
                    {
                        continue;
                    }
                    if push_diverse_result(
                        sibling.clone(),
                        &mut file_counts,
                        &mut name_counts,
                        &mut family_counts,
                        &mut diverse,
                        limit,
                    ) {
                        break;
                    }
                }
            }
        }
        if diverse.len() >= limit {
            break;
        }
    }
    Ok(diverse)
}

/// Agent queries commonly contain several identifiers that are alternatives,
/// plus a source-file stem. FTS BM25 alone can
/// bury an exact declaration below parameters and cannot return a file-name
/// match.  Seed the result with exact symbols and meaningful symbols from
/// matching source files before filling the remaining budget with FTS hits.
fn anchored_results(
    conn: &rusqlite::Connection,
    query: &str,
    limit: usize,
    include_signature: bool,
) -> anyhow::Result<Vec<SearchResult>> {
    if query.contains('*') {
        return Ok(Vec::new());
    }
    let mut terms = identifier_terms(query);
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    if terms.len() > 1 {
        terms.retain(|term| looks_like_code_anchor(term));
        if terms.is_empty() {
            return Ok(Vec::new());
        }
    }
    let sig_col = if include_signature {
        "s.signature"
    } else {
        "NULL"
    };
    let exact_sql = format!(
        "SELECT s.name, s.qualified_name, s.kind, f.path, s.line,
                {sig_col}, 1000.0
         FROM symbols s JOIN files f ON f.id = s.file_id
         WHERE s.origin = 'internal'
           AND (lower(s.name) = lower(?1) OR lower(s.qualified_name) = lower(?1))
           AND f.path NOT LIKE '.claude/worktrees/%'
           AND f.path NOT LIKE '.codex/worktrees/%'
         ORDER BY CASE WHEN lower(s.name) = lower(?1) THEN 0 ELSE 1 END,
                  CASE s.kind WHEN 'parameter' THEN 2 WHEN 'variable' THEN 2 ELSE 0 END,
                  f.path, s.line
         LIMIT {limit}"
    );
    let file_sql = format!(
        "SELECT s.name, s.qualified_name, s.kind, f.path, s.line,
                {sig_col}, 500.0
         FROM files f JOIN symbols s ON s.file_id = f.id
         WHERE f.origin = 'internal'
           AND lower(f.path) LIKE lower(?1) ESCAPE '\\'
           AND s.kind NOT IN ('parameter', 'variable', 'field')
           AND f.path NOT LIKE '.claude/worktrees/%'
           AND f.path NOT LIKE '.codex/worktrees/%'
         ORDER BY CASE WHEN s.kind = 'test' THEN 0 ELSE 1 END, s.line
         LIMIT {limit}"
    );

    let read_rows = |stmt: &mut rusqlite::Statement<'_>, value: &str| {
        let rows = stmt.query_map([value], |row| {
            Ok(SearchResult {
                name: row.get(0)?,
                qualified_name: row.get(1)?,
                kind: row.get(2)?,
                file_path: row.get(3)?,
                start_line: row.get(4)?,
                signature: row.get(5)?,
                score: row.get(6)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
    };

    let mut exact = conn.prepare(&exact_sql)?;
    let mut file = conn.prepare(&file_sql)?;
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for term in &terms {
        push_unique(&mut out, read_rows(&mut exact, term)?, &mut seen, limit);
        if out.len() >= limit {
            return Ok(out);
        }
    }

    // An exact regression-test name is often one member of a small behavior
    // matrix. Group lexically related sibling tests directly after that anchor
    // so an agent can recover the complete set without another broad call.
    let exact_results = std::mem::take(&mut out);
    seen.clear();
    if exact_results.iter().any(|result| result.kind == "test") {
        let sibling_sql = format!(
            "SELECT s.name, s.qualified_name, s.kind, f.path, s.line,
                    {sig_col}, 900.0
             FROM symbols s JOIN files f ON f.id = s.file_id
             WHERE s.origin = 'internal'
               AND s.kind = 'test'
               AND f.path = ?1
             ORDER BY abs(CAST(s.line AS INTEGER) - CAST(?2 AS INTEGER)), s.line
             LIMIT 20"
        );
        let mut siblings = conn.prepare(&sibling_sql)?;
        for anchor in exact_results {
            let is_test = anchor.kind == "test";
            let path = anchor.file_path.clone();
            let line = anchor.start_line;
            let anchor_name = anchor.name.clone();
            push_unique(&mut out, std::iter::once(anchor), &mut seen, limit);
            if !is_test || out.len() >= limit {
                continue;
            }
            let rows = siblings.query_map(rusqlite::params![path, line], |row| {
                Ok(SearchResult {
                    name: row.get(0)?,
                    qualified_name: row.get(1)?,
                    kind: row.get(2)?,
                    file_path: row.get(3)?,
                    start_line: row.get(4)?,
                    signature: row.get(5)?,
                    score: row.get(6)?,
                })
            })?;
            let found = rows
                .collect::<rusqlite::Result<Vec<_>>>()?
                .into_iter()
                .filter(|result| names_share_concepts(&anchor_name, &result.name, 2));
            push_unique(&mut out, found, &mut seen, limit);
            if out.len() >= limit {
                return Ok(out);
            }
        }
    } else {
        push_unique(&mut out, exact_results, &mut seen, limit);
    }

    for term in &terms {
        let pattern = format!("%{}%", escape_like(term));
        push_unique(&mut out, read_rows(&mut file, &pattern)?, &mut seen, limit);
        if out.len() >= limit {
            break;
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Public function
// ---------------------------------------------------------------------------

/// Full-text search across symbol names, qualified names, signatures, and doc
/// comments.
///
/// `query`  — FTS5 query string (plain words, prefix with `*`, phrases in `""`).
/// `limit`  — maximum results to return (pass 0 for no limit, capped at 500).
///
/// Results are returned in descending relevance order (highest score first).
pub fn search_symbols(
    db: &Database,
    query: &str,
    limit: usize,
    opts: &super::QueryOptions,
) -> QueryResult<Vec<SearchResult>> {
    let _timer = db.timer("search_symbols");
    let conn = db.conn();

    // Guard: FTS5 needs at least one term.
    if query.trim().is_empty() {
        return Ok(vec![]);
    }

    // Cap the limit — an unbounded FTS query on a large index is expensive.
    let effective_limit = if limit == 0 { 500 } else { limit.min(500) };

    let sig_col = if opts.include_signature {
        "s.signature"
    } else {
        "NULL"
    };
    let natural_terms = ranking_terms(query);
    let kind_filter = if natural_terms.len() > 1 {
        "AND s.kind NOT IN ('parameter', 'variable', 'field', 'property', 'enum_member', 'namespace')"
    } else {
        ""
    };

    let mut results = anchored_results(conn, query, effective_limit, opts.include_signature)
        .context("Failed to collect exact symbol and file matches")?;
    let mut seen: HashSet<(String, u32, String)> = results
        .iter()
        .map(|result| {
            (
                result.file_path.clone(),
                result.start_line,
                result.name.clone(),
            )
        })
        .collect();

    if results.len() < effective_limit {
        let ranked = ranked_term_results(conn, query, effective_limit, opts.include_signature)
            .context("Failed to rank multi-term symbol and file matches")?;
        push_unique(&mut results, ranked, &mut seen, effective_limit);
    }

    // --- Primary: FTS5 query ---
    // `rank` in FTS5 is a negative BM25 score; ORDER BY rank ascending puts
    // the best matches first.  We negate it in the SELECT list so callers see
    // positive values.
    let fts_sql = format!(
        "SELECT s.name,
                s.qualified_name,
                s.kind,
                f.path       AS file_path,
                s.line       AS start_line,
                {sig_col}    AS signature,
                (-fts.rank)  AS score
         FROM symbols_fts fts
         JOIN symbols s ON s.id = fts.rowid
         JOIN files   f ON f.id = s.file_id
         WHERE symbols_fts MATCH ?1
           AND s.origin = 'internal'
           {kind_filter}
           AND f.path NOT LIKE '.claude/worktrees/%'
           AND f.path NOT LIKE '.codex/worktrees/%'
         ORDER BY fts.rank
         LIMIT {effective_limit}"
    );

    let mut stmt = conn
        .prepare(&fts_sql)
        .context("Failed to prepare FTS5 search query")?;

    let mut run_fts = |fts_query: &str| -> anyhow::Result<Vec<SearchResult>> {
        let rows = stmt.query_map([fts_query], |row| {
            Ok(SearchResult {
                name: row.get(0)?,
                qualified_name: row.get(1)?,
                kind: row.get(2)?,
                file_path: row.get(3)?,
                start_line: row.get(4)?,
                signature: row.get(5)?,
                score: row.get(6)?,
            })
        })?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect FTS5 search results")
    };

    match run_fts(query) {
        Ok(found) => push_unique(&mut results, found, &mut seen, effective_limit),
        Err(e) => tracing::debug!("FTS5 search error: {e}"),
    }

    if results.len() < effective_limit {
        if let Some(or_query) = fallback_or_query(query) {
            match run_fts(&or_query) {
                Ok(found) => push_unique(&mut results, found, &mut seen, effective_limit),
                Err(e) => tracing::debug!("FTS5 OR fallback error: {e}"),
            }
        }
    }

    if !results.is_empty() {
        return Ok(results);
    }

    // --- Fallback: LIKE search on name and qualified_name ---
    // Useful when symbols_fts is empty (pre-trigger data) or when the FTS
    // query string is not a valid FTS5 expression.
    let like_pattern = format!("%{query}%");
    let like_sql = format!(
        "SELECT s.name,
                s.qualified_name,
                s.kind,
                f.path AS file_path,
                s.line AS start_line,
                {sig_col} AS signature,
                0.0    AS score
         FROM symbols s
         JOIN files f ON f.id = s.file_id
         WHERE s.origin = 'internal'
           AND (s.name           LIKE ?1 ESCAPE '\\'
             OR s.qualified_name LIKE ?1 ESCAPE '\\')
         ORDER BY s.qualified_name
         LIMIT {effective_limit}"
    );

    let mut stmt = conn
        .prepare(&like_sql)
        .context("Failed to prepare LIKE fallback query")?;

    let rows = stmt
        .query_map([&like_pattern], |row| {
            Ok(SearchResult {
                name: row.get(0)?,
                qualified_name: row.get(1)?,
                kind: row.get(2)?,
                file_path: row.get(3)?,
                start_line: row.get(4)?,
                signature: row.get(5)?,
                score: row.get(6)?,
            })
        })
        .context("Failed to execute LIKE fallback query")?;

    Ok(rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .context("Failed to collect LIKE fallback results")?)
}

/// Search a bounded evidence lane. Test searches add generic test concepts to
/// ranking before filtering, so callers do not have to guess which words make
/// test declarations rank above production symbols. Definition searches use a
/// larger internal candidate pool, then remove tests without consuming the
/// caller's result budget.
pub fn search_symbols_filtered(
    db: &Database,
    query: &str,
    limit: usize,
    opts: &super::QueryOptions,
    filter: SearchResultFilter,
) -> QueryResult<Vec<SearchResult>> {
    if filter == SearchResultFilter::All {
        return search_symbols(db, query, limit, opts);
    }

    let requested_limit = if limit == 0 { 500 } else { limit.min(500) };
    let candidate_limit = requested_limit.saturating_mul(4).min(500);
    let ranked_query = match filter {
        SearchResultFilter::Tests => format!("{query} regression tests"),
        SearchResultFilter::Definitions => query.to_owned(),
        SearchResultFilter::All => unreachable!(),
    };
    let mut results = search_symbols(db, &ranked_query, candidate_limit, opts)?;
    results.retain(|result| match filter {
        SearchResultFilter::Tests => result.kind == "test",
        SearchResultFilter::Definitions => result.kind != "test",
        SearchResultFilter::All => true,
    });
    results.truncate(requested_limit);
    Ok(results)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
