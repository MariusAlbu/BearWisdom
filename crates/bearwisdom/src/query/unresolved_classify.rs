// =============================================================================
// query/unresolved_classify.rs  —  unresolved-ref architectural classifier
//
// Tags every internal `unresolved_refs` row with a category that names its
// likely architectural source (extractor bug, locals miss, missing synthetic,
// real missing symbol, etc.). The output is an ordered worklist:
//   (language, kind, category) -> count + top-N target-name examples
// so per-language resolution work can be prioritized by source instead of
// raw target-name frequency.
//
// The classifier is heuristic and deterministic. It runs entirely over the
// existing schema — no migrations. Rows whose source symbol came from a
// Markdown fence or doctest (`from_snippet = 1`) are excluded, mirroring
// `resolution_breakdown`.
// =============================================================================

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::db::Database;
use crate::query::QueryResult;
use anyhow::Context;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Categories
// ---------------------------------------------------------------------------

/// One architectural source for an unresolved reference. Stable string ids
/// so reports can be diffed across runs.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum UnresolvedCategory {
    /// Extractor emitted a ref that should not exist (keyword, literal,
    /// punctuation-bearing target, empty name).
    ExtractorBug,
    /// Source file is generated or vendor code that should have been
    /// excluded from the resolve-loop input.
    GeneratedOrVendorNoise,
    /// `module IS NOT NULL` or the target name appears in `imports` for the
    /// same file — an import path/alias was identified but resolution failed.
    ModuleResolutionMiss,
    /// Same `target_name` is already classified external elsewhere in the
    /// project (`external_refs`); strong hint this row should be too. Fires
    /// only when no stronger signal applies.
    ExternalApiUnknown,
    /// Source file's language is a host/template language and the row's
    /// shape suggests an embedded-region routing or scope-rebasing miss.
    EmbeddedRegionIssue,
    /// Lowercase / short / single-letter shape that typically maps to a
    /// local variable, parameter, destructured name, or import alias.
    LocalFalsePositive,
    /// Target name carries a syntactic shape the resolver doesn't yet
    /// support (call-chain tail, generic-bracket residue, template
    /// literal, parens).
    UnsupportedSyntax,
    /// Connector-only relation that should have been routed to flow rather
    /// than the symbol resolver. Empty today; reserved for future kinds.
    ConnectorOnlyRelation,
    /// Fallback: shape looks like a normal identifier, no other signal
    /// applies. Most likely a genuinely missing symbol.
    RealMissingSymbol,
}

impl UnresolvedCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            UnresolvedCategory::ExtractorBug => "extractor_bug",
            UnresolvedCategory::GeneratedOrVendorNoise => "generated_or_vendor_noise",
            UnresolvedCategory::ModuleResolutionMiss => "module_resolution_miss",
            UnresolvedCategory::ExternalApiUnknown => "external_api_unknown",
            UnresolvedCategory::EmbeddedRegionIssue => "embedded_region_issue",
            UnresolvedCategory::LocalFalsePositive => "local_false_positive",
            UnresolvedCategory::UnsupportedSyntax => "unsupported_syntax",
            UnresolvedCategory::ConnectorOnlyRelation => "connector_only_relation",
            UnresolvedCategory::RealMissingSymbol => "real_missing_symbol",
        }
    }
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// One bucket of the worklist: a (language, kind, category) triple with
/// its count.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationBucket {
    pub language: String,
    pub kind: String,
    pub category: String,
    pub count: u64,
}

/// One target-name example surfaced for a (language, category) group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampleEntry {
    pub target_name: String,
    pub count: u32,
    pub example_file: Option<String>,
    pub example_line: Option<u32>,
}

/// Full classifier report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassificationReport {
    /// Total internal unresolved rows considered (excludes `from_snippet=1`).
    pub total: u64,
    /// Total per language, summed across all categories.
    pub by_language: BTreeMap<String, u64>,
    /// Total per category, summed across all languages.
    pub by_category: BTreeMap<String, u64>,
    /// Buckets sorted by count desc.
    pub buckets: Vec<ClassificationBucket>,
    /// Top-N target-name examples per `"<language>.<category>"` key,
    /// sorted by count desc within each group.
    pub samples: BTreeMap<String, Vec<SampleEntry>>,
}

// ---------------------------------------------------------------------------
// Public entry
// ---------------------------------------------------------------------------

/// Classify every internal unresolved reference and return the report.
///
/// `samples_per_group` caps the number of top target-name examples kept
/// per `(language, category)` group. 10 is a reasonable default for
/// CLI/MCP consumption.
pub fn classify_unresolved(
    db: &Database,
    samples_per_group: usize,
) -> QueryResult<ClassificationReport> {
    let _timer = db.timer("classify_unresolved");
    let conn = db.conn();

    // Names already classified external anywhere in the project — any of
    // these popping up as unresolved internal refs is a strong signal
    // that the same library wasn't recognized in this scope yet.
    let external_names: HashSet<String> = {
        let mut stmt = conn
            .prepare("SELECT DISTINCT target_name FROM external_refs")
            .context("classify_unresolved: prepare external_refs scan")?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .context("classify_unresolved: execute external_refs scan")?;
        rows.filter_map(|r| r.ok()).collect()
    };

    // Per-file imported names — a target appearing here flips the
    // category toward module_resolution_miss even when `module` is null
    // on the unresolved row itself.
    let imports_by_file: HashMap<i64, HashSet<String>> = {
        let mut stmt = conn
            .prepare("SELECT file_id, imported_name FROM imports")
            .context("classify_unresolved: prepare imports scan")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .context("classify_unresolved: execute imports scan")?;
        let mut map: HashMap<i64, HashSet<String>> = HashMap::new();
        for row in rows.flatten() {
            map.entry(row.0).or_default().insert(row.1);
        }
        map
    };

    // Internal unresolved rows (excluding from_snippet) joined with the
    // source symbol/file metadata the heuristics need.
    let mut stmt = conn
        .prepare(
            "SELECT u.target_name, u.kind, u.module, u.source_line,
                    f.id, f.path, f.language
             FROM unresolved_refs u
             JOIN symbols s ON s.id = u.source_id
             JOIN files   f ON f.id = s.file_id
             WHERE f.origin = 'internal' AND u.from_snippet = 0",
        )
        .context("classify_unresolved: prepare internal unresolved scan")?;

    // (language, kind, category) -> count
    let mut bucket_counts: BTreeMap<(String, String, UnresolvedCategory), u64> =
        BTreeMap::new();
    // "lang.category" -> target_name -> (count, example_file, example_line)
    let mut sample_counts: BTreeMap<String, HashMap<String, SampleEntry>> =
        BTreeMap::new();
    let mut by_language: BTreeMap<String, u64> = BTreeMap::new();
    let mut by_category: BTreeMap<String, u64> = BTreeMap::new();
    let mut total: u64 = 0;

    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,           // target_name
                r.get::<_, String>(1)?,           // kind
                r.get::<_, Option<String>>(2)?,   // module
                r.get::<_, Option<u32>>(3)?,      // source_line
                r.get::<_, i64>(4)?,              // file_id
                r.get::<_, String>(5)?,           // file_path
                r.get::<_, String>(6)?,           // file_language
            ))
        })
        .context("classify_unresolved: execute internal unresolved scan")?;

    for row in rows {
        let (target_name, kind, module, source_line, file_id, file_path, language) =
            match row {
                Ok(r) => r,
                Err(_) => continue,
            };

        let row = ClassifyRow {
            target_name: &target_name,
            kind: &kind,
            module: module.as_deref(),
            file_path: &file_path,
            language: &language,
        };
        let category = classify_row(
            &row,
            &external_names,
            imports_by_file.get(&file_id),
        );

        total += 1;
        *by_language.entry(language.clone()).or_default() += 1;
        *by_category.entry(category.as_str().to_string()).or_default() += 1;
        *bucket_counts
            .entry((language.clone(), kind.clone(), category))
            .or_default() += 1;

        let sample_key = format!("{}.{}", language, category.as_str());
        let entry = sample_counts
            .entry(sample_key)
            .or_default()
            .entry(target_name.clone())
            .or_insert(SampleEntry {
                target_name: target_name.clone(),
                count: 0,
                example_file: Some(file_path.clone()),
                example_line: source_line,
            });
        entry.count += 1;
    }

    // Flatten + sort buckets.
    let mut buckets: Vec<ClassificationBucket> = bucket_counts
        .into_iter()
        .map(|((language, kind, category), count)| ClassificationBucket {
            language,
            kind,
            category: category.as_str().to_string(),
            count,
        })
        .collect();
    buckets.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then(a.language.cmp(&b.language))
            .then(a.kind.cmp(&b.kind))
            .then(a.category.cmp(&b.category))
    });

    // Top-N samples per group.
    let mut samples: BTreeMap<String, Vec<SampleEntry>> = BTreeMap::new();
    for (key, name_map) in sample_counts {
        let mut entries: Vec<SampleEntry> = name_map.into_values().collect();
        entries.sort_by(|a, b| b.count.cmp(&a.count).then(a.target_name.cmp(&b.target_name)));
        entries.truncate(samples_per_group);
        samples.insert(key, entries);
    }

    Ok(ClassificationReport {
        total,
        by_language,
        by_category,
        buckets,
        samples,
    })
}

// ---------------------------------------------------------------------------
// Classifier — pure function, no DB
// ---------------------------------------------------------------------------

struct ClassifyRow<'a> {
    target_name: &'a str,
    kind: &'a str,
    module: Option<&'a str>,
    file_path: &'a str,
    language: &'a str,
}

fn classify_row(
    row: &ClassifyRow<'_>,
    external_names: &HashSet<String>,
    imports_for_file: Option<&HashSet<String>>,
) -> UnresolvedCategory {
    // 1. Extractor bug — the cheapest checks first.
    if is_extractor_garbage(row.target_name) {
        return UnresolvedCategory::ExtractorBug;
    }
    if is_keyword_or_literal(row.target_name, row.language) {
        return UnresolvedCategory::ExtractorBug;
    }

    // 2. Generated / vendor source.
    if looks_generated_or_vendor(row.file_path) {
        return UnresolvedCategory::GeneratedOrVendorNoise;
    }

    // 3. Module resolution miss — explicit `module` column on the row, or
    //    the target appears in this file's imports table but didn't resolve.
    if row.module.is_some() {
        return UnresolvedCategory::ModuleResolutionMiss;
    }
    if let Some(imports) = imports_for_file {
        if imports.contains(row.target_name) {
            return UnresolvedCategory::ModuleResolutionMiss;
        }
    }

    // 4. Embedded-region issue — host language with a target shape that
    //    typically belongs to the embedded sub-language.
    if is_embedded_host_language(row.language)
        && looks_like_sub_language_target(row.target_name)
    {
        return UnresolvedCategory::EmbeddedRegionIssue;
    }

    // 5. External API — same name already classified external elsewhere.
    //    Run before LocalFalsePositive so e.g. PascalCase library types
    //    don't get demoted to "local var" by the lowercase test below.
    if external_names.contains(row.target_name) {
        return UnresolvedCategory::ExternalApiUnknown;
    }

    // 6. Local false positive — locals.scm / scope-tree miss.
    if looks_like_local(row.target_name, row.kind) {
        return UnresolvedCategory::LocalFalsePositive;
    }

    // 7. Unsupported syntax — call-chain tails, generic residue, template literals.
    if looks_like_unsupported_syntax(row.target_name) {
        return UnresolvedCategory::UnsupportedSyntax;
    }

    // 8. Fallback.
    UnresolvedCategory::RealMissingSymbol
}

// ---------------------------------------------------------------------------
// Heuristics
// ---------------------------------------------------------------------------

fn is_extractor_garbage(name: &str) -> bool {
    if name.is_empty() || name.trim().is_empty() {
        return true;
    }
    // Punctuation that has no business in an identifier the resolver
    // should be looking up.
    name.contains('(')
        || name.contains(')')
        || name.contains('[')
        || name.contains(']')
        || name.contains('{')
        || name.contains('}')
        || name.contains('=')
        || name.contains(';')
        || name.contains('\n')
        || name.contains('\r')
        || name.contains('"')
        || name.contains('\'')
        || name.contains('`')
        || name.starts_with('.')
        || name.ends_with('.')
        || name.contains("..")
}

fn is_keyword_or_literal(name: &str, language: &str) -> bool {
    // Cross-language literals.
    matches!(
        name,
        "true" | "false" | "null" | "undefined" | "None" | "True" | "False"
            | "nil" | "this" | "self" | "super"
    )
    || is_numeric_literal(name)
    || is_language_keyword(name, language)
}

fn is_numeric_literal(name: &str) -> bool {
    let n = name.trim_start_matches('-');
    !n.is_empty()
        && n.chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == '_' || c == 'e' || c == 'E')
        && n.chars().any(|c| c.is_ascii_digit())
}

fn is_language_keyword(name: &str, language: &str) -> bool {
    // Only the keywords most commonly leaked by extractors as identifier
    // refs. This isn't a full keyword table — it's the "shouldn't have
    // been emitted as a target_name" set.
    let common = [
        "if", "else", "for", "while", "do", "return", "break", "continue",
        "switch", "case", "default", "try", "catch", "finally", "throw",
        "new", "delete", "in", "of", "as", "is",
    ];
    if common.contains(&name) {
        return true;
    }
    let extra: &[&str] = match language {
        "typescript" | "javascript" | "tsx" | "jsx" | "vue" | "svelte" => &[
            "let", "const", "var", "function", "class", "interface", "type",
            "enum", "import", "export", "from", "async", "await", "yield",
            "void", "any", "never", "unknown",
        ],
        "python" => &[
            "def", "class", "lambda", "import", "from", "as", "with", "yield",
            "async", "await", "pass", "raise", "global", "nonlocal", "and",
            "or", "not",
        ],
        "rust" => &[
            "fn", "let", "mut", "pub", "use", "mod", "struct", "enum", "impl",
            "trait", "where", "ref", "match", "loop", "move", "dyn",
        ],
        "csharp" => &[
            "using", "namespace", "class", "struct", "interface", "enum",
            "record", "public", "private", "internal", "protected", "static",
            "readonly", "sealed", "abstract", "virtual", "override", "ref",
            "out", "params", "var",
        ],
        "go" => &[
            "func", "var", "const", "type", "struct", "interface", "package",
            "import", "go", "defer", "chan", "map", "range", "select",
        ],
        "java" | "kotlin" => &[
            "class", "interface", "package", "import", "public", "private",
            "protected", "static", "final", "abstract", "extends", "implements",
            "fun", "val", "var",
        ],
        _ => &[],
    };
    extra.contains(&name)
}

fn looks_generated_or_vendor(path: &str) -> bool {
    let p = path.replace('\\', "/");
    let segments: Vec<&str> = p.split('/').collect();

    // Path-segment patterns.
    for seg in &segments {
        if matches!(
            *seg,
            "node_modules"
                | "vendor"
                | "third_party"
                | "third-party"
                | "dist"
                | "build"
                | "out"
                | ".next"
                | ".nuxt"
                | ".svelte-kit"
                | ".output"
                | "generated"
                | "__generated__"
                | "obj"
                | "bin"
                | "target"
                | ".gradle"
                | ".idea"
                | ".vscode"
        ) {
            return true;
        }
    }

    // Filename-suffix patterns.
    let leaf = segments.last().copied().unwrap_or("");
    leaf.ends_with(".g.cs")
        || leaf.ends_with(".designer.cs")
        || leaf.ends_with(".generated.cs")
        || leaf.ends_with(".generated.ts")
        || leaf.ends_with(".gen.go")
        || leaf.ends_with(".pb.go")
        || leaf.ends_with("_pb2.py")
        || leaf.ends_with("_pb2_grpc.py")
        || leaf.ends_with(".min.js")
        || leaf.ends_with(".bundle.js")
}

fn is_embedded_host_language(language: &str) -> bool {
    matches!(
        language,
        "vue" | "svelte" | "astro" | "mdx" | "razor" | "markdown" | "html"
            | "handlebars" | "ejs" | "pug" | "liquid"
    )
}

fn looks_like_sub_language_target(name: &str) -> bool {
    // Targets that read like "a script-block or style-block reference"
    // rather than a template-host construct. Heuristic — used only when
    // the source language is a host.
    if name.contains('-') {
        return false;
    }
    if name.contains('.') {
        return true;
    }
    name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

fn looks_like_local(name: &str, kind: &str) -> bool {
    if name.len() <= 2 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return true;
    }
    // Locals are typically lowercase identifiers used as call targets,
    // field/method receivers, or type annotations on parameters. Only
    // fire on lowercase-leading shapes — uppercase names are tagged as
    // possible external API or real-missing.
    let first = name.chars().next().unwrap_or(' ');
    if !first.is_ascii_lowercase() && first != '_' {
        return false;
    }
    // Reject anything dotted — dotted lowercase shapes are usually
    // ambient-global or chain-walker territory, not locals.
    if name.contains('.') || name.contains(':') {
        return false;
    }
    // Conservative: only `calls`, `type_ref`, `field` kinds get this
    // treatment. Inheritance / instantiation should not be local.
    matches!(kind, "calls" | "type_ref" | "field" | "reads" | "writes")
}

fn looks_like_unsupported_syntax(name: &str) -> bool {
    name.contains('<')
        || name.contains('>')
        || name.contains('?')
        || name.contains('|')
        || name.contains('&')
        || name.contains('+')
        || name.contains('*')
        || name.contains('!')
        || name.contains('@')
        || name.contains('#')
        || name.contains('$')
        || name.contains('%')
        || name.contains('^')
        || name.contains('~')
        || name.contains(',')
        || name.contains(' ')
}

#[cfg(test)]
#[path = "unresolved_classify_tests.rs"]
mod tests;

#[cfg(test)]
pub(super) fn _test_classify_row(
    target_name: &str,
    kind: &str,
    module: Option<&str>,
    file_path: &str,
    language: &str,
    external_names: &HashSet<String>,
    imports_for_file: Option<&HashSet<String>>,
) -> UnresolvedCategory {
    let row = ClassifyRow {
        target_name,
        kind,
        module,
        file_path,
        language,
    };
    classify_row(&row, external_names, imports_for_file)
}
