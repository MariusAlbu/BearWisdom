// =============================================================================
// bridge/scip.rs  —  SCIP index import
//
// Reads a binary SCIP index produced by scip-typescript, scip-dotnet,
// rust-analyzer, scip-python, etc. and merges its edges into the SQLite graph
// with confidence = 1.0.
//
// # Algorithm
//
//   For every Document in the SCIP index:
//     1. Normalise the document's relative_path and look up its file_id.
//     2. For each Occurrence that is a DEFINITION:
//        - Record a mapping: SCIP symbol string → DB symbol_id.
//        - Lookup is by (file_id, line) so that it works even when
//          qualified names differ between the tree-sitter extractor and the
//          SCIP tool.
//     3. For each Occurrence that is a REFERENCE (non-definition):
//        - Resolve the target symbol_id from the SCIP symbol string via the
//          definition map built in step 2, falling back to a qualified-name
//          lookup.
//        - Resolve the source symbol_id as the narrowest DB symbol containing
//          the reference line.
//        - Upsert the edge (source → target, kind="scip_ref", confidence=1.0).
//        - If a lower-confidence edge already exists for the same
//          (source,target,kind,line), upgrade it instead of inserting.
//
// # Idempotency
//   The edges table has a UNIQUE(source_id, target_id, kind, source_line)
//   constraint.  The INSERT OR REPLACE approach means running import twice
//   leaves the database identical.
//
// # SCIP symbol format (abbreviated)
//   "<scheme> <manager> <package> <version> <descriptor>+"
//   Only the descriptor portion is meaningful for qualified-name matching.
//   We parse out everything after the fourth space-separated token.
// =============================================================================

use crate::db::Database;
use anyhow::{Context, Result};
use prost::Message;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, instrument, warn};

// ---------------------------------------------------------------------------
// SCIP protobuf types  (manually defined — no .proto codegen required)
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, prost::Message)]
pub struct ScipIndex {
    #[prost(message, optional, tag = "1")]
    pub metadata: Option<Metadata>,
    #[prost(message, repeated, tag = "2")]
    pub documents: Vec<Document>,
    #[prost(message, repeated, tag = "3")]
    pub external_symbols: Vec<SymbolInformation>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Metadata {
    #[prost(enumeration = "ProtocolVersion", tag = "1")]
    pub version: i32,
    #[prost(message, optional, tag = "2")]
    pub tool_info: Option<ToolInfo>,
    #[prost(string, tag = "3")]
    pub project_root: String,
    #[prost(enumeration = "TextEncoding", tag = "4")]
    pub text_document_encoding: i32,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct ToolInfo {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub version: String,
    #[prost(string, repeated, tag = "3")]
    pub arguments: Vec<String>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Document {
    #[prost(string, tag = "1")]
    pub relative_path: String,
    #[prost(message, repeated, tag = "2")]
    pub occurrences: Vec<Occurrence>,
    #[prost(message, repeated, tag = "3")]
    pub symbols: Vec<SymbolInformation>,
    #[prost(string, tag = "4")]
    pub language: String,
    #[prost(string, tag = "5")]
    pub text: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Occurrence {
    /// [startLine, startChar, endLine, endChar]  or  [startLine, startChar, endChar]
    #[prost(int32, repeated, tag = "1")]
    pub range: Vec<i32>,
    #[prost(string, tag = "2")]
    pub symbol: String,
    /// Bitmask of SymbolRole constants.
    #[prost(int32, tag = "3")]
    pub symbol_roles: i32,
    #[prost(string, repeated, tag = "4")]
    pub override_documentation: Vec<String>,
    #[prost(enumeration = "SyntaxKind", tag = "5")]
    pub syntax_kind: i32,
    #[prost(message, repeated, tag = "6")]
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct SymbolInformation {
    #[prost(string, tag = "1")]
    pub symbol: String,
    #[prost(string, repeated, tag = "3")]
    pub documentation: Vec<String>,
    #[prost(message, repeated, tag = "4")]
    pub relationships: Vec<Relationship>,
    #[prost(enumeration = "SymbolKindScip", tag = "5")]
    pub kind: i32,
    #[prost(string, tag = "6")]
    pub display_name: String,
    #[prost(message, optional, tag = "7")]
    pub signature_documentation: Option<Document>,
    #[prost(string, tag = "8")]
    pub enclosing_symbol: String,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Relationship {
    #[prost(string, tag = "1")]
    pub symbol: String,
    #[prost(bool, tag = "2")]
    pub is_reference: bool,
    #[prost(bool, tag = "3")]
    pub is_implementation: bool,
    #[prost(bool, tag = "4")]
    pub is_type_definition: bool,
    #[prost(bool, tag = "5")]
    pub is_definition: bool,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct Diagnostic {
    #[prost(enumeration = "Severity", tag = "1")]
    pub severity: i32,
    #[prost(string, tag = "2")]
    pub code: String,
    #[prost(string, tag = "3")]
    pub message: String,
    #[prost(string, tag = "4")]
    pub source: String,
    #[prost(message, repeated, tag = "5")]
    pub tags: Vec<DiagnosticTag>,
}

#[derive(Clone, PartialEq, prost::Message)]
pub struct DiagnosticTag {
    #[prost(enumeration = "DiagnosticTagKind", tag = "1")]
    pub tag: i32,
}

// ---------------------------------------------------------------------------
// Enumerations
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, prost::Enumeration)]
#[repr(i32)]
pub enum ProtocolVersion {
    UnspecifiedProtocolVersion = 0,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, prost::Enumeration)]
#[repr(i32)]
pub enum TextEncoding {
    UnspecifiedTextEncoding = 0,
    Utf8 = 1,
    Utf16 = 2,
}

/// SCIP symbol kinds.  Mapped from the canonical SCIP proto enum values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, prost::Enumeration)]
#[repr(i32)]
pub enum SymbolKindScip {
    UnspecifiedKind = 0,
    Array = 1,
    Assertion = 2,
    AssociatedType = 3,
    Attribute = 4,
    Axiom = 5,
    Boolean = 6,
    Class = 7,
    Constant = 8,
    Constructor = 9,
    Contract = 62,
    DataFamily = 10,
    DefinitionMacro = 11,
    Enum = 12,
    EnumMember = 13,
    Error = 14,
    Event = 15,
    Fact = 16,
    Field = 17,
    File = 18,
    Function = 19,
    Getter = 20,
    Grammar = 21,
    Instance = 22,
    Interface = 23,
    Key = 24,
    Lang = 25,
    Lemma = 26,
    Library = 67,
    Macro = 27,
    Method = 28,
    Message = 29,
    Modifier = 30,
    Module = 31,
    Namespace = 32,
    Null = 33,
    Number = 34,
    Object = 35,
    Operator = 36,
    Package = 37,
    PackageObject = 38,
    Parameter = 39,
    ParameterLabel = 40,
    Pattern = 41,
    Predicate = 42,
    Property = 43,
    Protocol = 44,
    Quasiquoter = 45,
    MethodReceiver = 46,
    SelfParameter = 47,
    Setter = 48,
    Signature = 49,
    String = 50,
    Struct = 51,
    Subscript = 52,
    Tactic = 53,
    Theorem = 54,
    ThisParameter = 55,
    Trait = 56,
    TraitMethod = 57,
    Type = 58,
    TypeAlias = 59,
    TypeClass = 60,
    TypeClassMethod = 61,
    TypeFamily = 63,
    TypeParameter = 64,
    Union = 65,
    AbstractMethod = 66,
    Value = 68,
    Variable = 69,
    Accessor = 72,
    Delegate = 73,
    Destructor = 74,
    MethodAlias = 75,
    MethodSpecification = 76,
    ProtocolMethod = 77,
    PureVirtualMethod = 78,
    SingletonClass = 79,
    SingletonMethod = 80,
    StaticDataMember = 81,
    StaticEvent = 82,
    StaticField = 83,
    StaticMethod = 84,
    StaticProperty = 85,
    StaticVariable = 86,
    VirtualMethod = 87,
}

/// SCIP syntax highlight kinds — stored as-is; we only inspect symbol_roles.
/// Duplicate discriminant values from the SCIP spec are collapsed to the
/// first canonical name (e.g. value 4 is `Keyword`; `IdentifierKeyword` is
/// omitted because prost rejects duplicate discriminants).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, prost::Enumeration)]
#[repr(i32)]
pub enum SyntaxKind {
    UnspecifiedSyntaxKind = 0,
    Comment = 1,
    PunctuationDelimiter = 2,
    PunctuationBracket = 3,
    /// Value 4 in the SCIP spec is shared by `Keyword` and `IdentifierKeyword`.
    /// We keep `Keyword` as the canonical name.
    Keyword = 4,
    IdentifierOperator = 5,
    Identifier = 6,
    IdentifierBuiltin = 7,
    IdentifierNull = 8,
    IdentifierConstant = 9,
    IdentifierMutableGlobal = 10,
    IdentifierParameter = 11,
    IdentifierLocal = 12,
    IdentifierShadowed = 13,
    IdentifierNamespace = 14,
    IdentifierFunction = 15,
    IdentifierFunctionDefinition = 16,
    IdentifierMacro = 17,
    IdentifierMacroDefinition = 18,
    IdentifierType = 19,
    IdentifierBuiltinType = 20,
    IdentifierAttribute = 21,
    RegexEscape = 22,
    RegexRepeated = 23,
    RegexWildcard = 24,
    RegexDelimiter = 25,
    RegexJoin = 26,
    StringLiteral = 27,
    StringLiteralEscape = 28,
    StringLiteralSpecial = 29,
    StringLiteralKey = 30,
    CharacterLiteral = 31,
    NumericLiteral = 32,
    BooleanLiteral = 33,
    Tag = 34,
    TagAttribute = 35,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, prost::Enumeration)]
#[repr(i32)]
pub enum Severity {
    UnspecifiedSeverity = 0,
    Error = 1,
    Warning = 2,
    Information = 3,
    Hint = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, prost::Enumeration)]
#[repr(i32)]
pub enum DiagnosticTagKind {
    UnspecifiedDiagnosticTag = 0,
    Unnecessary = 1,
    Deprecated = 2,
}

// ---------------------------------------------------------------------------
// SymbolRole bitmask constants
// ---------------------------------------------------------------------------

pub const SYMBOL_ROLE_DEFINITION: i32 = 1;
pub const SYMBOL_ROLE_IMPORT: i32 = 2;
pub const SYMBOL_ROLE_WRITE_ACCESS: i32 = 4;
pub const SYMBOL_ROLE_READ_ACCESS: i32 = 8;
pub const SYMBOL_ROLE_GENERATED: i32 = 16;
pub const SYMBOL_ROLE_TEST: i32 = 32;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Statistics returned after a completed SCIP import.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScipImportStats {
    /// Number of SCIP documents visited.
    pub documents_processed: u32,
    /// Number of SCIP symbol occurrences successfully matched to a DB symbol.
    pub symbols_matched: u32,
    /// Number of new edges written to the `edges` table.
    pub edges_created: u32,
    /// Number of existing edges whose confidence was upgraded to 1.0.
    pub edges_upgraded: u32,
    /// Number of SCIP symbol strings that could not be resolved to a DB symbol.
    pub symbols_unmatched: u32,
}

/// Import a SCIP index file into the existing database.
///
/// Matches SCIP symbols to existing DB symbols by file path and line number
/// (definitions) or qualified name (references), then creates or upgrades
/// edges with `confidence = 1.0`.
///
/// The import is idempotent: running it twice against the same index produces
/// the same graph state.
#[instrument(skip(db), fields(scip_path = %scip_path.display()))]
pub fn import_scip(
    db: &Database,
    scip_path: &Path,
    project_root: &Path,
) -> Result<ScipImportStats> {
    let bytes = std::fs::read(scip_path)
        .with_context(|| format!("Failed to read SCIP file: {}", scip_path.display()))?;

    let index = ScipIndex::decode(bytes.as_slice())
        .with_context(|| format!("Failed to decode SCIP protobuf: {}", scip_path.display()))?;

    let tool_name = index
        .metadata
        .as_ref()
        .and_then(|m| m.tool_info.as_ref())
        .map(|t| t.name.as_str())
        .unwrap_or("unknown");

    let scip_root = index
        .metadata
        .as_ref()
        .map(|m| m.project_root.as_str())
        .unwrap_or("");

    tracing::info!(
        tool = tool_name,
        documents = index.documents.len(),
        "starting SCIP import"
    );

    let mut stats = ScipImportStats::default();

    // symbol_string → DB symbol_id, built per-document from definition occurrences.
    // Kept across all documents so cross-file references can be resolved.
    let mut scip_symbol_to_db_id: HashMap<String, i64> = HashMap::new();

    // Pass 1: resolve all definitions across every document first so that
    // cross-file references in pass 2 can look up targets that were
    // defined in a different document.
    for doc in &index.documents {
        let norm_path = normalise_doc_path(&doc.relative_path, project_root, scip_root);

        let file_id = match lookup_file_id(&db.conn, &norm_path)? {
            Some(id) => id,
            None => {
                debug!(path = %norm_path, "SCIP document has no matching DB file — skipping");
                continue;
            }
        };

        for occ in &doc.occurrences {
            if occ.symbol.is_empty() {
                continue;
            }
            if occ.symbol_roles & SYMBOL_ROLE_DEFINITION != 0 {
                let line = scip_range_start_line(&occ.range);
                if let Some(sym_id) = lookup_symbol_by_file_and_line(&db.conn, file_id, line)? {
                    scip_symbol_to_db_id.insert(occ.symbol.clone(), sym_id);
                    stats.symbols_matched += 1;
                } else {
                    debug!(
                        symbol = %occ.symbol,
                        file = %norm_path,
                        line,
                        "no DB symbol at SCIP definition site"
                    );
                    stats.symbols_unmatched += 1;
                }
            }
        }
    }

    // Also walk external_symbols for relationship edges — their symbol IDs
    // may already be in our map from the document pass.
    // (No separate pass needed; we handle them during edge resolution below.)

    // Pass 2: create edges for reference occurrences.
    for doc in &index.documents {
        let norm_path = normalise_doc_path(&doc.relative_path, project_root, scip_root);

        let file_id = match lookup_file_id(&db.conn, &norm_path)? {
            Some(id) => id,
            None => continue,
        };

        stats.documents_processed += 1;

        for occ in &doc.occurrences {
            if occ.symbol.is_empty() {
                continue;
            }

            // Skip definitions — they populate the map but don't create edges.
            if occ.symbol_roles & SYMBOL_ROLE_DEFINITION != 0 {
                continue;
            }

            let ref_line = scip_range_start_line(&occ.range);

            // Resolve the source symbol — narrowest DB symbol enclosing this line.
            let source_id =
                match lookup_narrowest_symbol_at_line(&db.conn, file_id, ref_line)? {
                    Some(id) => id,
                    None => {
                        stats.symbols_unmatched += 1;
                        continue;
                    }
                };

            // Resolve the target symbol from the definition map, falling back
            // to a qualified-name scan.
            let target_id = match scip_symbol_to_db_id.get(&occ.symbol) {
                Some(&id) => id,
                None => {
                    // Fall back: parse the SCIP symbol and look up by qualified name.
                    let qname = scip_symbol_to_qualified_name(&occ.symbol);
                    match lookup_symbol_by_qualified_name(&db.conn, &qname)? {
                        Some(id) => {
                            // Cache for future lookups within this import run.
                            scip_symbol_to_db_id.insert(occ.symbol.clone(), id);
                            id
                        }
                        None => {
                            debug!(
                                symbol = %occ.symbol,
                                qname = %qname,
                                "SCIP reference target not found in DB"
                            );
                            stats.symbols_unmatched += 1;
                            continue;
                        }
                    }
                }
            };

            // Don't create self-edges.
            if source_id == target_id {
                continue;
            }

            let changed = upsert_scip_edge(
                &db.conn,
                source_id,
                target_id,
                ref_line,
            )?;

            match changed {
                EdgeChange::Created => stats.edges_created += 1,
                EdgeChange::Upgraded => stats.edges_upgraded += 1,
                EdgeChange::Unchanged => {}
            }
        }

        // Also process SymbolInformation.relationships declared in the document —
        // these encode inheritance, implementation, etc. between named symbols.
        for sym_info in &doc.symbols {
            if sym_info.symbol.is_empty() {
                continue;
            }
            let source_id = match scip_symbol_to_db_id.get(&sym_info.symbol) {
                Some(&id) => id,
                None => continue,
            };

            for rel in &sym_info.relationships {
                if rel.symbol.is_empty() {
                    continue;
                }
                let target_id = match scip_symbol_to_db_id.get(&rel.symbol) {
                    Some(&id) => id,
                    None => {
                        let qname = scip_symbol_to_qualified_name(&rel.symbol);
                        match lookup_symbol_by_qualified_name(&db.conn, &qname)? {
                            Some(id) => id,
                            None => continue,
                        }
                    }
                };

                if source_id == target_id {
                    continue;
                }

                // Relationships don't carry a source line.
                let edge_kind = scip_relationship_kind(rel);
                let changed = upsert_edge_by_kind(
                    &db.conn,
                    source_id,
                    target_id,
                    edge_kind,
                    None,
                )?;

                match changed {
                    EdgeChange::Created => stats.edges_created += 1,
                    EdgeChange::Upgraded => stats.edges_upgraded += 1,
                    EdgeChange::Unchanged => {}
                }
            }
        }
    }

    // Upgrade any existing sub-1.0 edges that are now confirmed by SCIP.
    // The upsert logic handles this per-edge; this bulk pass catches any
    // that slipped through (e.g. edges inserted by tree-sitter after a
    // prior partial SCIP import).
    let upgraded_bulk = bulk_upgrade_confirmed_edges(&db.conn)?;
    stats.edges_upgraded += upgraded_bulk;

    warn!(
        documents = stats.documents_processed,
        symbols_matched = stats.symbols_matched,
        edges_created = stats.edges_created,
        edges_upgraded = stats.edges_upgraded,
        symbols_unmatched = stats.symbols_unmatched,
        "SCIP import complete"
    );

    Ok(stats)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Result of a single edge upsert.
#[derive(Debug, PartialEq, Eq)]
enum EdgeChange {
    Created,
    Upgraded,
    Unchanged,
}

/// Normalise a SCIP relative_path so it can be matched against `files.path`.
///
/// Strategy (in order):
///   1. If the path already exactly matches `files.path` format, use it as-is.
///   2. Strip a `file:///` URI prefix if present.
///   3. Strip `project_root` or `scip_root` prefixes (forward-slash normalised).
///   4. Strip any leading `/` or `./` so the result is a clean relative path.
fn normalise_doc_path(relative_path: &str, project_root: &Path, scip_root: &str) -> String {
    // Strip URI scheme if present.
    let p = relative_path
        .strip_prefix("file:///")
        .unwrap_or(relative_path);

    // Normalise separators to forward slash for uniform comparison.
    let p = p.replace('\\', "/");

    // Project root as forward-slash string.
    let root_fwd = project_root.to_string_lossy().replace('\\', "/");
    let root_prefix = if root_fwd.ends_with('/') {
        root_fwd.clone()
    } else {
        format!("{root_fwd}/")
    };

    // Try stripping project_root prefix first, then scip_root.
    let stripped = if let Some(rel) = p.strip_prefix(&root_prefix) {
        rel.to_string()
    } else if !scip_root.is_empty() {
        let scip_stripped = scip_root.strip_prefix("file:///").unwrap_or(scip_root);
        let scip_fwd = scip_stripped.replace('\\', "/");
        let scip_prefix = if scip_fwd.ends_with('/') {
            scip_fwd.clone()
        } else {
            format!("{scip_fwd}/")
        };
        p.strip_prefix(&scip_prefix)
            .unwrap_or(&p)
            .to_string()
    } else {
        p.clone()
    };

    // Remove any remaining leading slash or `./`.
    let stripped = stripped
        .trim_start_matches('/')
        .trim_start_matches("./")
        .to_string();

    // If stripping removed everything (path was the root itself), keep original.
    if stripped.is_empty() {
        p.trim_start_matches('/').to_string()
    } else {
        stripped
    }
}

/// Extract the 0-based start line from a SCIP range vector.
///
/// SCIP range encoding:
///   - 3 elements: [startLine, startChar, endChar]         (single-line)
///   - 4 elements: [startLine, startChar, endLine, endChar] (multi-line)
fn scip_range_start_line(range: &[i32]) -> i32 {
    range.first().copied().unwrap_or(0)
}

/// Parse a SCIP symbol string into a qualified name fragment.
///
/// SCIP symbol format (space-separated):
///   `<scheme> <manager> <package> <version> <descriptor...>`
///
/// We take everything from the 5th token onward as the "descriptor" and
/// collapse it to a dot-separated qualified name.
///
/// Examples:
///   `scip-typescript npm my-pkg 1.0 src/foo.ts/MyClass#method().`
///   → `src/foo.ts/MyClass#method().`
///
///   `scip-dotnet nuget Microsoft.Extensions.DI 7.0 Microsoft.Extensions.DependencyInjection.ServiceCollection#AddSingleton().`
///   → `Microsoft.Extensions.DependencyInjection.ServiceCollection#AddSingleton().`
///
/// The descriptor is returned as-is; callers do a LIKE-based lookup so the
/// exact format is irrelevant as long as the qualified_name column contains
/// the meaningful part.
pub fn scip_symbol_to_qualified_name(symbol: &str) -> String {
    let tokens: Vec<&str> = symbol.splitn(5, ' ').collect();
    if tokens.len() < 5 {
        // Malformed or local symbol — return the whole string.
        return symbol.to_string();
    }

    // The descriptor is the fifth token (index 4).  Convert SCIP path
    // separators to dots for a rough qualified-name match.
    tokens[4]
        .trim_end_matches('.')
        .replace('/', ".")
        .replace('#', ".")
        .replace("().", "")
        .replace("()!", "")
        .replace('(', "")
        .replace(')', "")
        .to_string()
}

/// Determine the edge kind for a SCIP Relationship.
fn scip_relationship_kind(rel: &Relationship) -> &'static str {
    if rel.is_implementation {
        "implements"
    } else if rel.is_type_definition {
        "type_ref"
    } else if rel.is_definition {
        "calls"
    } else {
        "scip_ref"
    }
}

// ---------------------------------------------------------------------------
// DB query helpers
// ---------------------------------------------------------------------------

fn lookup_file_id(conn: &rusqlite::Connection, path: &str) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT id FROM files WHERE path = ?1",
        rusqlite::params![path],
        |row| row.get(0),
    )
    .optional()
    .context("lookup_file_id")
}

/// Find the DB symbol id at exactly (file_id, line).
/// SCIP range[0] is 0-based; `symbols.line` is stored 0-based by the extractor.
fn lookup_symbol_by_file_and_line(
    conn: &rusqlite::Connection,
    file_id: i64,
    line: i32,
) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT id FROM symbols WHERE file_id = ?1 AND line = ?2 LIMIT 1",
        rusqlite::params![file_id, line],
        |row| row.get(0),
    )
    .optional()
    .context("lookup_symbol_by_file_and_line")
}

/// Find the narrowest DB symbol (by line span) that contains `ref_line`.
/// Used to identify the source symbol of a reference occurrence.
fn lookup_narrowest_symbol_at_line(
    conn: &rusqlite::Connection,
    file_id: i64,
    ref_line: i32,
) -> Result<Option<i64>> {
    conn.query_row(
        "SELECT id FROM symbols
         WHERE file_id = ?1
           AND line <= ?2
           AND COALESCE(end_line, line) >= ?2
         ORDER BY (COALESCE(end_line, line) - line) ASC
         LIMIT 1",
        rusqlite::params![file_id, ref_line],
        |row| row.get(0),
    )
    .optional()
    .context("lookup_narrowest_symbol_at_line")
}

/// Lookup a symbol by qualified name.
/// Uses a LIKE suffix match so partial descriptor strings still find the symbol.
fn lookup_symbol_by_qualified_name(
    conn: &rusqlite::Connection,
    qname: &str,
) -> Result<Option<i64>> {
    if qname.is_empty() {
        return Ok(None);
    }

    // Exact match first (fast, uses index).
    let exact: Option<i64> = conn
        .query_row(
            "SELECT id FROM symbols WHERE qualified_name = ?1 LIMIT 1",
            rusqlite::params![qname],
            |row| row.get(0),
        )
        .optional()
        .context("lookup_symbol_by_qualified_name exact")?;

    if exact.is_some() {
        return Ok(exact);
    }

    // Suffix match — handles cases where the SCIP descriptor includes a
    // package prefix that is absent from the tree-sitter qualified name.
    let suffix = format!("%{qname}");
    conn.query_row(
        "SELECT id FROM symbols WHERE qualified_name LIKE ?1 LIMIT 1",
        rusqlite::params![suffix],
        |row| row.get(0),
    )
    .optional()
    .context("lookup_symbol_by_qualified_name suffix")
}

/// Upsert a `scip_ref` edge between source and target at confidence 1.0.
/// Returns what changed.
fn upsert_scip_edge(
    conn: &rusqlite::Connection,
    source_id: i64,
    target_id: i64,
    source_line: i32,
) -> Result<EdgeChange> {
    upsert_edge_by_kind(conn, source_id, target_id, "scip_ref", Some(source_line))
}

/// Upsert an edge of the given kind at confidence 1.0.
fn upsert_edge_by_kind(
    conn: &rusqlite::Connection,
    source_id: i64,
    target_id: i64,
    kind: &str,
    source_line: Option<i32>,
) -> Result<EdgeChange> {
    // Try to upgrade an existing lower-confidence edge first.
    let upgraded = conn
        .execute(
            "UPDATE edges
             SET confidence = 1.0
             WHERE source_id = ?1
               AND target_id = ?2
               AND kind      = ?3
               AND source_line IS ?4
               AND confidence < 1.0",
            rusqlite::params![source_id, target_id, kind, source_line],
        )
        .context("upsert_edge_by_kind UPDATE")?;

    if upgraded > 0 {
        return Ok(EdgeChange::Upgraded);
    }

    // No sub-1.0 row exists — insert (idempotent via OR IGNORE).
    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO edges
                 (source_id, target_id, kind, source_line, confidence)
             VALUES (?1, ?2, ?3, ?4, 1.0)",
            rusqlite::params![source_id, target_id, kind, source_line],
        )
        .context("upsert_edge_by_kind INSERT")?;

    if inserted > 0 {
        Ok(EdgeChange::Created)
    } else {
        // Row already existed at 1.0.
        Ok(EdgeChange::Unchanged)
    }
}

/// Upgrade any remaining sub-1.0 edges whose (source, target) pairs are now
/// confirmed by SCIP — i.e. a confidence=1.0 `scip_ref` edge exists for the
/// same pair with a different kind.
///
/// Returns the count of rows upgraded.
fn bulk_upgrade_confirmed_edges(conn: &rusqlite::Connection) -> Result<u32> {
    let rows = conn
        .execute(
            "UPDATE edges
             SET confidence = 1.0
             WHERE confidence < 1.0
               AND (source_id, target_id) IN (
                   SELECT source_id, target_id
                   FROM edges
                   WHERE kind = 'scip_ref' AND confidence = 1.0
               )",
            [],
        )
        .context("bulk_upgrade_confirmed_edges")?;

    Ok(rows as u32)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use prost::Message;

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    /// Create an in-memory DB pre-seeded with one file and two symbols.
    ///
    /// Returns `(db, file_id, caller_id, callee_id)`.
    /// - caller: function at line 0, spans lines 0–9.
    /// - callee: function at line 20, spans lines 20–29.
    fn seed_db() -> (Database, i64, i64, i64) {
        let db = Database::open_in_memory().unwrap();

        db.conn
            .execute(
                "INSERT INTO files (path, hash, language, last_indexed)
                 VALUES ('src/app.ts', 'abc', 'typescript', 0)",
                [],
            )
            .unwrap();
        let file_id = db.conn.last_insert_rowid();

        db.conn
            .execute(
                "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, end_line)
                 VALUES (?1, 'caller', 'app.caller', 'function', 0, 0, 9)",
                rusqlite::params![file_id],
            )
            .unwrap();
        let caller_id = db.conn.last_insert_rowid();

        db.conn
            .execute(
                "INSERT INTO symbols (file_id, name, qualified_name, kind, line, col, end_line)
                 VALUES (?1, 'callee', 'app.callee', 'function', 20, 0, 29)",
                rusqlite::params![file_id],
            )
            .unwrap();
        let callee_id = db.conn.last_insert_rowid();

        (db, file_id, caller_id, callee_id)
    }

    /// Build a minimal ScipIndex in memory and encode it to bytes.
    ///
    /// The index describes `src/app.ts`:
    ///   - `scip-typescript npm app 1.0 src/app.ts/caller().`  is defined at line 0.
    ///   - `scip-typescript npm app 1.0 src/app.ts/callee().`  is defined at line 20.
    ///   - `scip-typescript npm app 1.0 src/app.ts/callee().`  is referenced at line 5
    ///     (inside `caller`).
    fn build_scip_bytes(doc_path: &str) -> Vec<u8> {
        let index = ScipIndex {
            metadata: Some(Metadata {
                version: ProtocolVersion::UnspecifiedProtocolVersion as i32,
                tool_info: Some(ToolInfo {
                    name: "scip-typescript".into(),
                    version: "0.3.0".into(),
                    arguments: vec![],
                }),
                project_root: "file:///workspace".into(),
                text_document_encoding: TextEncoding::Utf8 as i32,
            }),
            documents: vec![Document {
                relative_path: doc_path.into(),
                language: "typescript".into(),
                text: String::new(),
                occurrences: vec![
                    // Definition of `caller` at line 0.
                    Occurrence {
                        range: vec![0, 0, 9, 0],
                        symbol: "scip-typescript npm app 1.0 src/app.ts/caller().".into(),
                        symbol_roles: SYMBOL_ROLE_DEFINITION,
                        override_documentation: vec![],
                        syntax_kind: SyntaxKind::IdentifierFunctionDefinition as i32,
                        diagnostics: vec![],
                    },
                    // Definition of `callee` at line 20.
                    Occurrence {
                        range: vec![20, 0, 29, 0],
                        symbol: "scip-typescript npm app 1.0 src/app.ts/callee().".into(),
                        symbol_roles: SYMBOL_ROLE_DEFINITION,
                        override_documentation: vec![],
                        syntax_kind: SyntaxKind::IdentifierFunctionDefinition as i32,
                        diagnostics: vec![],
                    },
                    // Reference to `callee` from within `caller` at line 5.
                    Occurrence {
                        range: vec![5, 4, 10],
                        symbol: "scip-typescript npm app 1.0 src/app.ts/callee().".into(),
                        symbol_roles: SYMBOL_ROLE_READ_ACCESS,
                        override_documentation: vec![],
                        syntax_kind: SyntaxKind::IdentifierFunction as i32,
                        diagnostics: vec![],
                    },
                ],
                symbols: vec![],
            }],
            external_symbols: vec![],
        };

        let mut buf = Vec::new();
        index.encode(&mut buf).unwrap();
        buf
    }

    // -----------------------------------------------------------------------
    // Unit tests: helper functions
    // -----------------------------------------------------------------------

    #[test]
    fn scip_range_start_line_three_element() {
        // [startLine, startChar, endChar]
        assert_eq!(scip_range_start_line(&[5, 4, 10]), 5);
    }

    #[test]
    fn scip_range_start_line_four_element() {
        // [startLine, startChar, endLine, endChar]
        assert_eq!(scip_range_start_line(&[12, 0, 15, 0]), 12);
    }

    #[test]
    fn scip_range_start_line_empty() {
        assert_eq!(scip_range_start_line(&[]), 0);
    }

    #[test]
    fn scip_symbol_to_qualified_name_typescript() {
        let sym = "scip-typescript npm my-pkg 1.0 src/foo.ts/MyClass#method().";
        let qname = scip_symbol_to_qualified_name(sym);
        // Should strip the SCIP preamble (4 tokens) and normalise separators.
        assert_eq!(qname, "src.foo.ts.MyClass.method");
    }

    #[test]
    fn scip_symbol_to_qualified_name_dotnet() {
        let sym = "scip-dotnet nuget Microsoft.Extensions.DI 7.0 Microsoft.Extensions.DependencyInjection.ServiceCollection#AddSingleton().";
        let qname = scip_symbol_to_qualified_name(sym);
        assert_eq!(
            qname,
            "Microsoft.Extensions.DependencyInjection.ServiceCollection.AddSingleton"
        );
    }

    #[test]
    fn scip_symbol_to_qualified_name_too_few_tokens() {
        let sym = "short symbol";
        let qname = scip_symbol_to_qualified_name(sym);
        // Falls back to whole string.
        assert_eq!(qname, sym);
    }

    #[test]
    fn normalise_doc_path_strips_project_root() {
        let root = Path::new("/workspace/myproject");
        let result = normalise_doc_path("src/index.ts", root, "");
        assert_eq!(result, "src/index.ts");
    }

    #[test]
    fn normalise_doc_path_strips_absolute_prefix() {
        let root = Path::new("/workspace/myproject");
        let result = normalise_doc_path("/workspace/myproject/src/index.ts", root, "");
        assert_eq!(result, "src/index.ts");
    }

    #[test]
    fn normalise_doc_path_strips_uri_and_scip_root() {
        let root = Path::new("/other/path");
        let result = normalise_doc_path(
            "file:///workspace/src/app.ts",
            root,
            "file:///workspace",
        );
        assert_eq!(result, "src/app.ts");
    }

    #[test]
    fn normalise_doc_path_strips_leading_dot_slash() {
        let root = Path::new("/workspace");
        let result = normalise_doc_path("./src/app.ts", root, "");
        assert_eq!(result, "src/app.ts");
    }

    // -----------------------------------------------------------------------
    // Unit tests: DB helpers
    // -----------------------------------------------------------------------

    #[test]
    fn lookup_file_id_found_and_not_found() {
        let (db, file_id, _, _) = seed_db();
        let found = lookup_file_id(&db.conn, "src/app.ts").unwrap();
        assert_eq!(found, Some(file_id));

        let missing = lookup_file_id(&db.conn, "nonexistent.ts").unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn lookup_symbol_by_file_and_line_exact() {
        let (db, file_id, caller_id, callee_id) = seed_db();

        let found = lookup_symbol_by_file_and_line(&db.conn, file_id, 0).unwrap();
        assert_eq!(found, Some(caller_id));

        let found2 = lookup_symbol_by_file_and_line(&db.conn, file_id, 20).unwrap();
        assert_eq!(found2, Some(callee_id));
    }

    #[test]
    fn lookup_narrowest_symbol_at_line_inside_span() {
        let (db, file_id, caller_id, callee_id) = seed_db();

        // Line 5 is inside caller (0–9).
        let found = lookup_narrowest_symbol_at_line(&db.conn, file_id, 5).unwrap();
        assert_eq!(found, Some(caller_id));

        // Line 25 is inside callee (20–29).
        let found2 = lookup_narrowest_symbol_at_line(&db.conn, file_id, 25).unwrap();
        assert_eq!(found2, Some(callee_id));

        // Line 50 is outside both.
        let outside = lookup_narrowest_symbol_at_line(&db.conn, file_id, 50).unwrap();
        assert!(outside.is_none());
    }

    #[test]
    fn lookup_symbol_by_qualified_name_exact_and_suffix() {
        let (db, _, caller_id, _) = seed_db();

        // Exact match.
        let found = lookup_symbol_by_qualified_name(&db.conn, "app.caller").unwrap();
        assert_eq!(found, Some(caller_id));

        // Suffix match — SCIP descriptor may have package prefix not in DB.
        let found2 = lookup_symbol_by_qualified_name(&db.conn, "caller").unwrap();
        assert_eq!(found2, Some(caller_id));

        // Not found.
        let missing = lookup_symbol_by_qualified_name(&db.conn, "totally.unknown").unwrap();
        assert!(missing.is_none());
    }

    // -----------------------------------------------------------------------
    // Unit tests: edge upsert
    // -----------------------------------------------------------------------

    #[test]
    fn upsert_scip_edge_creates_new_edge() {
        let (db, _, caller_id, callee_id) = seed_db();

        let change = upsert_scip_edge(&db.conn, caller_id, callee_id, 5).unwrap();
        assert_eq!(change, EdgeChange::Created);

        let conf: f64 = db
            .conn
            .query_row(
                "SELECT confidence FROM edges
                 WHERE source_id = ?1 AND target_id = ?2",
                rusqlite::params![caller_id, callee_id],
                |r| r.get(0),
            )
            .unwrap();
        assert!((conf - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn upsert_scip_edge_idempotent() {
        let (db, _, caller_id, callee_id) = seed_db();

        upsert_scip_edge(&db.conn, caller_id, callee_id, 5).unwrap();
        // Second call should be a no-op.
        let change = upsert_scip_edge(&db.conn, caller_id, callee_id, 5).unwrap();
        assert_eq!(change, EdgeChange::Unchanged);

        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "idempotent: still only one edge");
    }

    #[test]
    fn upsert_scip_edge_upgrades_low_confidence() {
        let (db, _, caller_id, callee_id) = seed_db();

        // Pre-insert a tree-sitter edge at 0.6.
        db.conn
            .execute(
                "INSERT INTO edges (source_id, target_id, kind, source_line, confidence)
                 VALUES (?1, ?2, 'scip_ref', 5, 0.6)",
                rusqlite::params![caller_id, callee_id],
            )
            .unwrap();

        let change = upsert_scip_edge(&db.conn, caller_id, callee_id, 5).unwrap();
        assert_eq!(change, EdgeChange::Upgraded);

        let conf: f64 = db
            .conn
            .query_row(
                "SELECT confidence FROM edges
                 WHERE source_id = ?1 AND target_id = ?2",
                rusqlite::params![caller_id, callee_id],
                |r| r.get(0),
            )
            .unwrap();
        assert!((conf - 1.0).abs() < f64::EPSILON, "should be upgraded to 1.0");
    }

    // -----------------------------------------------------------------------
    // Integration test: import_scip end-to-end
    // -----------------------------------------------------------------------

    #[test]
    fn import_scip_creates_edge_from_reference_occurrence() {
        let dir = tempfile::TempDir::new().unwrap();
        let scip_path = dir.path().join("index.scip");

        let (db, _file_id, caller_id, callee_id) = seed_db();

        // Write the encoded SCIP index to a temp file.
        let bytes = build_scip_bytes("src/app.ts");
        std::fs::write(&scip_path, &bytes).unwrap();

        let project_root = Path::new("/workspace");
        let stats = import_scip(&db, &scip_path, project_root).unwrap();

        assert_eq!(stats.documents_processed, 1);
        // Two definition occurrences should match (caller and callee).
        assert_eq!(stats.symbols_matched, 2, "both definitions should match");
        assert_eq!(stats.edges_created, 1, "one reference edge expected");
        assert_eq!(stats.edges_upgraded, 0);
        assert_eq!(stats.symbols_unmatched, 0);

        // Verify the actual edge exists with confidence 1.0.
        let conf: f64 = db
            .conn
            .query_row(
                "SELECT confidence FROM edges
                 WHERE source_id = ?1 AND target_id = ?2 AND kind = 'scip_ref'",
                rusqlite::params![caller_id, callee_id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            (conf - 1.0).abs() < f64::EPSILON,
            "edge confidence should be 1.0, got {conf}"
        );
    }

    #[test]
    fn import_scip_is_idempotent() {
        let dir = tempfile::TempDir::new().unwrap();
        let scip_path = dir.path().join("index.scip");
        let bytes = build_scip_bytes("src/app.ts");
        std::fs::write(&scip_path, &bytes).unwrap();

        let (db, _, _, _) = seed_db();
        let project_root = Path::new("/workspace");

        let stats1 = import_scip(&db, &scip_path, project_root).unwrap();
        let stats2 = import_scip(&db, &scip_path, project_root).unwrap();

        // Edge count in DB should be the same after both runs.
        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "idempotent: still one edge after two imports");

        // Second run should create nothing and upgrade nothing (edge already at 1.0).
        assert_eq!(stats2.edges_created, 0, "second run should create no new edges");
        // Any upgraded count from bulk_upgrade is fine to be 0 on the second run too.
        let _ = stats1; // used — suppress lint
    }

    #[test]
    fn import_scip_skips_unknown_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let scip_path = dir.path().join("index.scip");

        // Build SCIP index referencing a file that is NOT in the DB.
        let bytes = build_scip_bytes("src/unknown_file.ts");
        std::fs::write(&scip_path, &bytes).unwrap();

        let (db, _, _, _) = seed_db();
        let project_root = Path::new("/workspace");

        let stats = import_scip(&db, &scip_path, project_root).unwrap();

        // Document is not found in DB — should process 0 documents.
        assert_eq!(stats.documents_processed, 0);
        assert_eq!(stats.edges_created, 0);

        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn import_scip_upgrades_preexisting_low_confidence_edge() {
        let dir = tempfile::TempDir::new().unwrap();
        let scip_path = dir.path().join("index.scip");
        let bytes = build_scip_bytes("src/app.ts");
        std::fs::write(&scip_path, &bytes).unwrap();

        let (db, _, caller_id, callee_id) = seed_db();

        // Pre-seed a tree-sitter edge at 0.7 confidence.
        db.conn
            .execute(
                "INSERT INTO edges (source_id, target_id, kind, source_line, confidence)
                 VALUES (?1, ?2, 'scip_ref', 5, 0.7)",
                rusqlite::params![caller_id, callee_id],
            )
            .unwrap();

        let project_root = Path::new("/workspace");
        let stats = import_scip(&db, &scip_path, project_root).unwrap();

        assert_eq!(stats.edges_upgraded, 1, "should upgrade the pre-existing edge");
        assert_eq!(stats.edges_created, 0);

        let conf: f64 = db
            .conn
            .query_row(
                "SELECT confidence FROM edges
                 WHERE source_id = ?1 AND target_id = ?2",
                rusqlite::params![caller_id, callee_id],
                |r| r.get(0),
            )
            .unwrap();
        assert!((conf - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn import_scip_handles_relationship_edges() {
        let dir = tempfile::TempDir::new().unwrap();
        let scip_path = dir.path().join("index.scip");

        let (db, _, caller_id, callee_id) = seed_db();

        // Build a SCIP index that has a SymbolInformation relationship
        // (caller implements callee — contrived but tests the path).
        let caller_sym = "scip-typescript npm app 1.0 src/app.ts/caller().".to_string();
        let callee_sym = "scip-typescript npm app 1.0 src/app.ts/callee().".to_string();

        let index = ScipIndex {
            metadata: None,
            documents: vec![Document {
                relative_path: "src/app.ts".into(),
                language: "typescript".into(),
                text: String::new(),
                occurrences: vec![
                    Occurrence {
                        range: vec![0, 0, 9, 0],
                        symbol: caller_sym.clone(),
                        symbol_roles: SYMBOL_ROLE_DEFINITION,
                        override_documentation: vec![],
                        syntax_kind: 0,
                        diagnostics: vec![],
                    },
                    Occurrence {
                        range: vec![20, 0, 29, 0],
                        symbol: callee_sym.clone(),
                        symbol_roles: SYMBOL_ROLE_DEFINITION,
                        override_documentation: vec![],
                        syntax_kind: 0,
                        diagnostics: vec![],
                    },
                ],
                symbols: vec![SymbolInformation {
                    symbol: caller_sym.clone(),
                    documentation: vec![],
                    relationships: vec![Relationship {
                        symbol: callee_sym.clone(),
                        is_reference: false,
                        is_implementation: true,
                        is_type_definition: false,
                        is_definition: false,
                    }],
                    kind: SymbolKindScip::Function as i32,
                    display_name: "caller".into(),
                    signature_documentation: None,
                    enclosing_symbol: String::new(),
                }],
            }],
            external_symbols: vec![],
        };

        let mut buf = Vec::new();
        index.encode(&mut buf).unwrap();
        std::fs::write(&scip_path, &buf).unwrap();

        let stats = import_scip(&db, &scip_path, Path::new("/workspace")).unwrap();

        assert_eq!(stats.documents_processed, 1);
        assert_eq!(stats.edges_created, 1, "relationship edge should be created");

        let kind: String = db
            .conn
            .query_row(
                "SELECT kind FROM edges WHERE source_id = ?1 AND target_id = ?2",
                rusqlite::params![caller_id, callee_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kind, "implements");
    }

    #[test]
    fn import_scip_no_self_edges() {
        let dir = tempfile::TempDir::new().unwrap();
        let scip_path = dir.path().join("index.scip");

        let (db, _, _, _) = seed_db();

        // Build a SCIP index where caller references itself.
        let sym = "scip-typescript npm app 1.0 src/app.ts/caller().".to_string();
        let index = ScipIndex {
            metadata: None,
            documents: vec![Document {
                relative_path: "src/app.ts".into(),
                language: "typescript".into(),
                text: String::new(),
                occurrences: vec![
                    Occurrence {
                        range: vec![0, 0, 9, 0],
                        symbol: sym.clone(),
                        symbol_roles: SYMBOL_ROLE_DEFINITION,
                        override_documentation: vec![],
                        syntax_kind: 0,
                        diagnostics: vec![],
                    },
                    // Reference to itself at line 5 (still inside caller's span).
                    Occurrence {
                        range: vec![5, 0, 10],
                        symbol: sym.clone(),
                        symbol_roles: SYMBOL_ROLE_READ_ACCESS,
                        override_documentation: vec![],
                        syntax_kind: 0,
                        diagnostics: vec![],
                    },
                ],
                symbols: vec![],
            }],
            external_symbols: vec![],
        };

        let mut buf = Vec::new();
        index.encode(&mut buf).unwrap();
        std::fs::write(&scip_path, &buf).unwrap();

        let stats = import_scip(&db, &scip_path, Path::new("/workspace")).unwrap();

        assert_eq!(stats.edges_created, 0, "self-edge must be suppressed");
        let count: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn import_scip_bad_file_returns_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let bad_path = dir.path().join("garbage.scip");
        std::fs::write(&bad_path, b"this is not a valid protobuf").unwrap();

        let (db, _, _, _) = seed_db();
        let result = import_scip(&db, &bad_path, Path::new("/workspace"));
        assert!(result.is_err(), "corrupt SCIP file should return Err");
    }

    #[test]
    fn import_scip_missing_file_returns_error() {
        let (db, _, _, _) = seed_db();
        let result = import_scip(
            &db,
            Path::new("/no/such/file.scip"),
            Path::new("/workspace"),
        );
        assert!(result.is_err(), "missing SCIP file should return Err");
    }
}
