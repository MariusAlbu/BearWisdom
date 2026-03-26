// =============================================================================
// types.rs  —  all data types shared across the crate
//
// Convention:
//   • Types that go into SQLite use simple owned Strings (no lifetimes needed).
//   • "Extracted*" types are intermediate values produced by the parser but
//     not yet written to the DB (no IDs assigned yet).
//   • "Symbol", "Edge" etc. are the DB-row representations (with IDs).
// =============================================================================

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Enumerations
// ---------------------------------------------------------------------------

/// The syntactic kind of a symbol.
///
/// C# adds Namespace, Field, Event, Delegate that v1 was missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SymbolKind {
    // Shared
    Class,
    Struct,
    Interface,
    Enum,
    EnumMember,
    Method,
    Constructor,
    Property,
    Field,
    // C# specific
    Namespace,
    Event,
    Delegate,
    // TypeScript specific
    Function,     // top-level function (not a method)
    TypeAlias,    // `type Foo = ...`
    Variable,     // `const`, `let`, `var`
    // Test methods (detected by attribute / naming)
    Test,
}

impl SymbolKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Class => "class",
            Self::Struct => "struct",
            Self::Interface => "interface",
            Self::Enum => "enum",
            Self::EnumMember => "enum_member",
            Self::Method => "method",
            Self::Constructor => "constructor",
            Self::Property => "property",
            Self::Field => "field",
            Self::Namespace => "namespace",
            Self::Event => "event",
            Self::Delegate => "delegate",
            Self::Function => "function",
            Self::TypeAlias => "type_alias",
            Self::Variable => "variable",
            Self::Test => "test",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "class" => Some(Self::Class),
            "struct" => Some(Self::Struct),
            "interface" => Some(Self::Interface),
            "enum" => Some(Self::Enum),
            "enum_member" => Some(Self::EnumMember),
            "method" => Some(Self::Method),
            "constructor" => Some(Self::Constructor),
            "property" => Some(Self::Property),
            "field" => Some(Self::Field),
            "namespace" => Some(Self::Namespace),
            "event" => Some(Self::Event),
            "delegate" => Some(Self::Delegate),
            "function" => Some(Self::Function),
            "type_alias" => Some(Self::TypeAlias),
            "variable" => Some(Self::Variable),
            "test" => Some(Self::Test),
            _ => None,
        }
    }
}

impl std::fmt::Display for SymbolKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Kinds of directed edges in the code graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// A method/function calls another method/function.
    Calls,
    /// A type inherits from another (class → class).
    Inherits,
    /// A type implements an interface (class/struct → interface).
    Implements,
    /// A parameter, return type, or field references another type.
    TypeRef,
    /// An `object_creation_expression` (`new Foo()`).
    Instantiates,
    /// A `using` directive (C#) or `import` statement (TS/JS) that brings
    /// a namespace or module into scope.  `target_name` and `module` both
    /// hold the full namespace/module path.
    Imports,
    /// A fetch/axios call in TS matches a route defined in C#.
    HttpCall,
    /// A DbSet<T> property is linked to its entity class.
    DbEntity,
    /// An edge discovered purely by an LSP server (no tree-sitter counterpart).
    LspResolved,
}

impl EdgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Calls => "calls",
            Self::Inherits => "inherits",
            Self::Implements => "implements",
            Self::TypeRef => "type_ref",
            Self::Instantiates => "instantiates",
            Self::Imports => "imports",
            Self::HttpCall => "http_call",
            Self::DbEntity => "db_entity",
            Self::LspResolved => "lsp_resolved",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "calls" => Some(Self::Calls),
            "inherits" => Some(Self::Inherits),
            "implements" => Some(Self::Implements),
            "type_ref" => Some(Self::TypeRef),
            "instantiates" => Some(Self::Instantiates),
            "imports" => Some(Self::Imports),
            "http_call" => Some(Self::HttpCall),
            "db_entity" => Some(Self::DbEntity),
            "lsp_resolved" => Some(Self::LspResolved),
            _ => None,
        }
    }
}

impl std::fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// C# and TypeScript visibility modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Public,
    Private,
    Protected,
    Internal,
    // C# `protected internal` / `private protected` — simplified to Protected
}

impl Visibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
            Self::Protected => "protected",
            Self::Internal => "internal",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "public" => Some(Self::Public),
            "private" => Some(Self::Private),
            "protected" => Some(Self::Protected),
            "internal" => Some(Self::Internal),
            _ => None,
        }
    }
}

/// Tracks which subsystem produced an edge — used for provenance,
/// not stored in the `edges` table (stored in `lsp_edge_meta` instead).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeSource {
    /// Edge produced by the tree-sitter 5-priority resolver.
    TreeSitter,
    /// Edge produced or confirmed by a Language Server Protocol server.
    Lsp { server: String },
    /// Edge produced by a connector (HTTP routes, EF Core, gRPC).
    Connector,
    /// Edge imported from a SCIP index file (future).
    Scip,
}

// ---------------------------------------------------------------------------
// Scope tree (produced by parser/scope_tree.rs)
// ---------------------------------------------------------------------------

/// A node in the per-file scope tree.
///
/// The scope tree is built by walking the CST and noting which node kinds
/// "create" a new scope (e.g. `class_declaration`, `method_declaration`).
/// Children of those nodes are placed in the nested scope.
///
/// This tree drives qualified-name generation:
///   root → Namespace("Microsoft.eShop.Catalog")
///     └─ Class("CatalogDbContext")
///         └─ Method("OnModelCreating")
///   → qualified_name = "Microsoft.eShop.Catalog.CatalogDbContext.OnModelCreating"
#[derive(Debug, Clone)]
pub struct ScopeNode {
    /// The simple name of this scope (e.g. "CatalogDbContext").
    pub name: String,
    /// Full dotted path including all ancestors (e.g. "Microsoft.eShop.Catalog.CatalogDbContext").
    pub qualified_name: String,
    /// The tree-sitter node kind that opened this scope.
    pub node_kind: String,
    /// Index of the parent in the owning Vec<ScopeNode>, or None for the root.
    pub parent_index: Option<usize>,
    /// Children of this scope (indexes into the same Vec<ScopeNode>).
    pub children: Vec<usize>,
    /// 0-based byte offset where this scope starts in the source.
    pub start_byte: usize,
    /// 0-based byte offset where this scope ends in the source.
    pub end_byte: usize,
}

// ---------------------------------------------------------------------------
// Intermediate extraction types (parser output, not yet in DB)
// ---------------------------------------------------------------------------

/// A symbol discovered during tree-sitter extraction.
/// All positions are 0-based line numbers matching tree-sitter's convention.
#[derive(Debug, Clone)]
pub struct ExtractedSymbol {
    /// Simple name (e.g. "MapCatalogApiV1").
    pub name: String,
    /// Full dotted path (e.g. "Catalog.CatalogApi.MapCatalogApiV1").
    pub qualified_name: String,
    pub kind: SymbolKind,
    pub visibility: Option<Visibility>,
    /// 0-based start line.
    pub start_line: u32,
    /// 0-based end line.
    pub end_line: u32,
    pub start_col: u32,
    pub end_col: u32,
    /// Function/method signature string, e.g. "Task<User> GetById(int id)".
    pub signature: Option<String>,
    /// C# XML doc comment or JSDoc, if present.
    pub doc_comment: Option<String>,
    /// Scope path (ancestors, dot-separated) — used for DB `scope_path` column.
    pub scope_path: Option<String>,
    /// Index of this symbol's parent in the same Vec<ExtractedSymbol>.
    pub parent_index: Option<usize>,
}

/// An unresolved reference from one symbol to a named target.
///
/// After all files are parsed, the resolver walks these and attempts to match
/// each `target_name` to a known symbol using the 4-priority lookup.
#[derive(Debug, Clone)]
pub struct ExtractedRef {
    /// Index into the Vec<ExtractedSymbol> that CONTAINS this reference.
    pub source_symbol_index: usize,
    /// The name being referenced.
    pub target_name: String,
    pub kind: EdgeKind,
    /// 0-based source line of the reference site.
    pub line: u32,
    /// For imports: the module path (e.g. "System.Linq", "./catalog-api").
    pub module: Option<String>,
}

/// An HTTP route attribute extracted from C#.
///
/// Built up during extraction: `[HttpGet("/api/catalog/{id}")]` produces one.
/// The connector later matches these against TS fetch/axios calls.
#[derive(Debug, Clone)]
pub struct ExtractedRoute {
    /// Index into Vec<ExtractedSymbol> for the handler method.
    pub handler_symbol_index: usize,
    pub http_method: String,  // "GET", "POST", "PUT", "DELETE", "PATCH"
    pub template: String,     // e.g. "/api/catalog/items/{id:int}"
}

/// An EF Core DbSet<T> property extracted from a DbContext class.
#[derive(Debug, Clone)]
pub struct ExtractedDbSet {
    /// Index into Vec<ExtractedSymbol> for the DbSet property itself.
    pub property_symbol_index: usize,
    /// The C# entity type name (the T in DbSet<T>).
    pub entity_type: String,
    /// Table name (from [Table("...")] attribute or convention).
    pub table_name: String,
    /// How the table name was determined.
    pub source: DbMappingSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DbMappingSource {
    Convention,
    Attribute,
    Fluent,
}

impl DbMappingSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Convention => "convention",
            Self::Attribute => "attribute",
            Self::Fluent => "fluent",
        }
    }
}

/// Everything extracted from a single source file.
#[derive(Debug)]
pub struct ParsedFile {
    pub path: String,
    pub language: String,
    pub content_hash: String,
    pub size: u64,
    pub line_count: u32,
    pub symbols: Vec<ExtractedSymbol>,
    pub refs: Vec<ExtractedRef>,
    pub routes: Vec<ExtractedRoute>,
    pub db_sets: Vec<ExtractedDbSet>,
    /// Raw file content, retained for FTS5 content indexing and code chunk extraction.
    pub content: Option<String>,
    /// True if tree-sitter reported syntax errors (extraction is still attempted).
    pub has_errors: bool,
}

// ---------------------------------------------------------------------------
// DB row types (with IDs — returned from query layer)
// ---------------------------------------------------------------------------

/// A symbol row as stored in the `symbols` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub id: i64,
    pub file_path: String,
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub line: u32,
    pub col: u32,
    pub end_line: Option<u32>,
    pub end_col: Option<u32>,
    pub scope_path: Option<String>,
    pub signature: Option<String>,
    pub visibility: Option<String>,
}

/// A resolved edge row (both endpoints resolved to symbol IDs).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub source_id: i64,
    pub target_id: i64,
    pub kind: String,
    pub source_line: Option<u32>,
    pub confidence: f64,
}

// ---------------------------------------------------------------------------
// Query result types (returned to the benchmark CLI or future API layer)
// ---------------------------------------------------------------------------

/// A single "find references" result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceResult {
    /// The symbol that holds the reference (the caller / user).
    pub referencing_symbol: String,
    pub referencing_kind: String,
    pub file_path: String,
    pub line: u32,
    pub edge_kind: String,
    pub confidence: f64,
}

/// A single "go to definition" result.
///
/// There may be multiple results when a name is ambiguous.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefinitionResult {
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub file_path: String,
    pub line: u32,
    pub col: u32,
    pub signature: Option<String>,
    pub confidence: f64,
}

/// A route → handler mapping returned by the HTTP connector.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteInfo {
    pub id: i64,
    pub file_path: String,
    pub http_method: String,
    pub route_template: String,
    pub resolved_route: Option<String>,
    pub line: u32,
    pub handler_name: Option<String>,
}

/// An EF Core entity → table mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbMapping {
    pub id: i64,
    pub entity_type: String,
    pub table_name: String,
    pub source: String,
    pub file_path: String,
}

// ---------------------------------------------------------------------------
// Statistics
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IndexStats {
    pub file_count: u32,
    pub symbol_count: u32,
    pub edge_count: u32,
    pub unresolved_ref_count: u32,
    pub route_count: u32,
    pub db_mapping_count: u32,
    pub files_with_errors: u32,
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
