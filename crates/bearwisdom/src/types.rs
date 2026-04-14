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
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
    strum::AsRefStr, strum::IntoStaticStr, strum::EnumString, strum::Display,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
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
        self.into()
    }
}

/// Kinds of directed edges in the code graph.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
    strum::AsRefStr, strum::IntoStaticStr, strum::EnumString, strum::Display,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
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
        self.into()
    }
}

/// C# and TypeScript visibility modifiers.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
    strum::AsRefStr, strum::IntoStaticStr, strum::EnumString, strum::Display,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Visibility {
    Public,
    Private,
    Protected,
    Internal,
    // C# `protected internal` / `private protected` — simplified to Protected
}

impl Visibility {
    pub fn as_str(self) -> &'static str {
        self.into()
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

// ---------------------------------------------------------------------------
// Member access chain (structured representation of tree-sitter AST)
// ---------------------------------------------------------------------------

/// The semantic role of a segment in a member access chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentKind {
    /// `this` / `self` / `base` — receiver referencing the enclosing type.
    SelfRef,
    /// A plain identifier: variable, parameter, function name, package name.
    Identifier,
    /// A property/field access: `obj.prop`.
    Property,
    /// A static/type-level access: `ClassName.staticMethod()`.
    TypeAccess,
    /// A `new` / object creation: `new Foo()`.
    Construction,
    /// A computed property access: `obj['key']` or `obj[expr]`.
    ComputedAccess,
    /// A namespace/package qualifier: `pkg.Symbol` in Go, `Namespace.Type` in C#.
    NamespaceAccess,
}

/// A single segment in a member access chain.
#[derive(Debug, Clone)]
pub struct ChainSegment {
    /// The identifier text of this segment.
    pub name: String,
    /// The tree-sitter node kind that produced this segment.
    pub node_kind: String,
    /// The semantic role of this segment in the chain.
    pub kind: SegmentKind,
    /// The declared type from a type annotation visible in the AST.
    pub declared_type: Option<String>,
    /// Generic type arguments, if the declared type is generic.
    /// e.g., for `repo: Repository<User>`, type_args = ["User"].
    /// For `map: Map<string, Handler>`, type_args = ["string", "Handler"].
    pub type_args: Vec<String>,
    /// Whether this segment uses optional chaining (`?.`).
    pub optional_chaining: bool,
}

/// A structured member access chain built from tree-sitter AST nodes.
#[derive(Debug, Clone)]
pub struct MemberChain {
    pub segments: Vec<ChainSegment>,
}

// ---------------------------------------------------------------------------
// Extracted types (parser output, pre-resolution)
// ---------------------------------------------------------------------------

/// An unresolved reference from one symbol to a named target.
///
/// After all files are parsed, the resolver walks these and attempts to match
/// each `target_name` to a known symbol using the multi-tier lookup.
#[derive(Debug, Clone)]
pub struct ExtractedRef {
    /// Index into the Vec<ExtractedSymbol> that CONTAINS this reference.
    pub source_symbol_index: usize,
    /// The name being referenced.
    /// For chain-bearing refs, this is the LAST segment name (the method/property).
    /// For simple refs, this is the full target name.
    pub target_name: String,
    pub kind: EdgeKind,
    /// 0-based source line of the reference site.
    pub line: u32,
    /// For imports: the module path (e.g. "System.Linq", "./catalog-api").
    pub module: Option<String>,
    /// Structured member access chain from tree-sitter AST.
    /// `None` for simple identifier refs (e.g., `foo()`, import bindings, type refs).
    pub chain: Option<MemberChain>,
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

/// Universal extraction result returned by all language plugins.
pub struct ExtractionResult {
    pub symbols: Vec<ExtractedSymbol>,
    pub refs: Vec<ExtractedRef>,
    pub routes: Vec<ExtractedRoute>,
    pub db_sets: Vec<ExtractedDbSet>,
    pub has_errors: bool,
}

// ---------------------------------------------------------------------------
// Embedded-region dispatch (multi-language host files)
// ---------------------------------------------------------------------------

/// A region of text inside a host file that should be parsed by a different
/// language extractor. Produced by host extractors (Svelte/Vue/Astro/Razor/
/// HTML/PHP/MDX) via the separate `LanguagePlugin::embedded_regions` trait
/// method; the indexer dispatches each region to the plugin for its declared
/// language, re-runs locals filtering against the sub-grammar, and splices
/// the resulting symbols/refs back into the host file with line/column
/// offsets applied.
#[derive(Debug, Clone)]
pub struct EmbeddedRegion {
    /// Language id the sub-extractor should be looked up by — matches the
    /// ids registered in `LanguageRegistry` (e.g. `"typescript"`, `"javascript"`,
    /// `"css"`, `"scss"`, `"csharp"`).
    pub language_id: String,
    /// The raw text of the region, already stripped of any host-language
    /// delimiters (e.g. `<script>…</script>` → the text between the tags).
    pub text: String,
    /// 0-based line number in the host file where `text` begins.
    pub line_offset: u32,
    /// 0-based column offset in the host file for the first line of `text`.
    /// Only applied to symbols/refs that start on line 0 of the sub-extraction.
    pub col_offset: u32,
    /// Semantic role of this region — used for diagnostics and for origin
    /// attribution on spliced symbols.
    pub origin: EmbeddedOrigin,
    /// Byte spans inside `text` that should be blanked out before sub-parsing.
    /// Used for interpolation punch-through in string-embedded DSLs
    /// (e.g. `` sql`SELECT * FROM ${t}` `` — the `${t}` span becomes whitespace
    /// so the SQL grammar sees syntactically valid text). Empty for host-file
    /// consumers like Svelte/Vue/Astro/Razor, which emit whole blocks verbatim.
    pub holes: Vec<Span>,
    /// Synthetic scope prefix to strip from every sub-extracted symbol's
    /// `qualified_name` and `scope_path` before splicing back into the
    /// host file. Set by hosts that wrap their region text in a
    /// synthetic class / namespace to satisfy the sub-language grammar
    /// (e.g. Razor wraps C# bodies in `class __RazorBody { … }` so
    /// tree-sitter-csharp accepts bare method declarations — the
    /// wrapper then needs to disappear from user-facing names).
    /// `None` for hosts that pass the source verbatim.
    pub strip_scope_prefix: Option<String>,
}

/// A half-open byte range `[start, end)` inside an `EmbeddedRegion::text`.
#[derive(Debug, Clone, Copy)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// Where an embedded region came from inside the host file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddedOrigin {
    /// `<script>` / `<script lang="ts">` / `<script setup>` block inside an
    /// HTML-dialect host file (Vue, Svelte, Astro, Razor, plain HTML).
    ScriptBlock,
    /// `<style>` / `<style lang="scss">` block inside an HTML-dialect host.
    StyleBlock,
    /// Astro-style `---`-delimited frontmatter at the top of a file.
    Frontmatter,
    /// Razor `@{}`, `@functions{}`, `@code{}`, `@model`, `@inject`, `@(expr)`
    /// directive or statement block containing C#.
    RazorCode,
    /// A tagged template literal or string argument in Tier-3 string DSLs
    /// (SQL in C# raw strings, GraphQL in TS `gql\`…\``, CSS-in-JS, etc.).
    StringDsl,
    /// PHP `<?php … ?>` / `<?= … ?>` short-echo or `@php … @endphp` Blade
    /// block — an explicit switch into PHP mode from a template host.
    PhpBlock,
    /// `{{ expr }}` / `{!! expr !!}` (Blade) or `{{ expr }}` (Twig /
    /// Jinja / Handlebars / Angular) — a single expression interpolation
    /// inside template text.
    TemplateExpr,
    /// `{% tag … %}` (Twig / Jinja / Liquid) directive forms that control
    /// template flow (`block`, `extends`, `include`, `use`, `set`, etc.).
    TemplateDirective,
    /// Fenced code block (```lang ... ```) inside a Markdown/MDX host or a
    /// host-language doc comment (Rust `///`, JSDoc `@example`, Python
    /// docstring `>>>`). Snippets tag their symbols as `from_snippet=true`
    /// so unresolved references don't pollute the project's resolution
    /// stats — snippets are usually missing imports.
    MarkdownFence,
    /// YAML / TOML / JSON frontmatter block at the top of a Markdown file
    /// (Jekyll, Hugo, Docusaurus, Obsidian, Hexo, Astro content collection).
    /// Not snippet-tagged — frontmatter is structured configuration.
    MarkdownFrontmatter,
    /// A single code cell inside a notebook (Jupyter `.ipynb`,
    /// RMarkdown `.Rmd`, Quarto `.qmd`, or .NET Polyglot `.dib`).
    /// Notebook cells are NOT snippet-tagged — they're real, runnable
    /// project code whose unresolved refs should count against
    /// aggregate resolution stats the same as any other source file.
    NotebookCell,
}

impl ExtractionResult {
    pub fn new(
        symbols: Vec<ExtractedSymbol>,
        refs: Vec<ExtractedRef>,
        has_errors: bool,
    ) -> Self {
        Self { symbols, refs, routes: Vec::new(), db_sets: Vec::new(), has_errors }
    }

    pub fn with_connectors(
        symbols: Vec<ExtractedSymbol>,
        refs: Vec<ExtractedRef>,
        routes: Vec<ExtractedRoute>,
        db_sets: Vec<ExtractedDbSet>,
        has_errors: bool,
    ) -> Self {
        Self { symbols, refs, routes, db_sets, has_errors }
    }

    pub fn empty() -> Self {
        Self { symbols: Vec::new(), refs: Vec::new(), routes: Vec::new(), db_sets: Vec::new(), has_errors: false }
    }
}

/// A detected package within a monorepo / workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfo {
    /// Database row ID (assigned after INSERT).
    pub id: Option<i64>,
    /// Package name (ecosystem-native if available, else directory name).
    pub name: String,
    /// Relative path from workspace root to package directory.
    pub path: String,
    /// Ecosystem hint: "npm", "cargo", "dotnet", "go", etc.
    pub kind: Option<String>,
    /// Relative path to the manifest file (e.g., "services/api/package.json").
    pub manifest: Option<String>,
}

/// Everything extracted from a single source file.
#[derive(Debug)]
pub struct ParsedFile {
    pub path: String,
    pub language: String,
    pub content_hash: String,
    pub size: u64,
    pub line_count: u32,
    /// File modification time (seconds since epoch), for fast change detection.
    pub mtime: Option<i64>,
    /// Package this file belongs to (assigned during indexing, `None` for root files).
    pub package_id: Option<i64>,
    pub symbols: Vec<ExtractedSymbol>,
    pub refs: Vec<ExtractedRef>,
    pub routes: Vec<ExtractedRoute>,
    pub db_sets: Vec<ExtractedDbSet>,
    /// Origin language per symbol (indexed same as `symbols`). `None` at a
    /// given index means "same as `language`"; `Some(lang_id)` means the
    /// symbol was produced by a sub-extractor on an embedded region (e.g. a
    /// TypeScript symbol from a `<script lang="ts">` block inside a `.vue`
    /// file). Always the same length as `symbols`, or empty if no sub-
    /// extraction happened (DB insert treats empty as all-None).
    pub symbol_origin_languages: Vec<Option<String>>,
    /// E3: per-symbol snippet flag, parallel to `symbols`. `true` means the
    /// symbol was extracted from a code snippet — Markdown fenced block,
    /// Rust doc-test, Python doctest. Snippet symbols propagate
    /// `unresolved_refs.from_snippet = 1` so resolution-rate aggregates can
    /// exclude them (snippets typically lack imports, so noise is expected).
    /// Same length as `symbols`, or empty if no snippet extraction happened
    /// (DB insert treats empty as all-false).
    pub symbol_from_snippet: Vec<bool>,
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
    pub external_ref_count: u32,
    pub route_count: u32,
    pub db_mapping_count: u32,
    pub flow_edge_count: u32,
    pub package_count: u32,
    pub files_with_errors: u32,
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
