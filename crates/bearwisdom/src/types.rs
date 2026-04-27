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
use std::collections::HashMap;

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

/// Structural shape of a type alias's right-hand side.
///
/// Captured at extract time so the chain walker can decide whether the
/// alias is *expandable* (a single concrete type application) or carries
/// non-application semantics that need different machinery (unions need
/// member-set semantics, intersections combine member sets, mapped types
/// generate fields, etc.).
///
/// Only TypeScript classifies aliases this finely today; other languages
/// (C/C++ typedefs, Dart typedefs, F# abbrevs) emit simple applications
/// and rely on the engine to derive an `Application` from their first
/// `TypeRef`. The `Other` arm is the conservative bucket — chain walkers
/// must not expand it, since the underlying shape is unknown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AliasTarget {
    /// Single type application: `type Foo = Bar` or `type Foo<T> = Map<string, T>`.
    /// `root` is the head type's name; `args` are the in-source type arguments
    /// (already substituted with the alias's own generic params if any).
    Application { root: String, args: Vec<String> },
    /// `type Foo = A | B | C` — record the branch types for future
    /// member-set / narrowing logic. Chain walking does not expand unions
    /// in PR 9; this arm is captured so a future PR can wire keyof and
    /// member-set semantics without re-touching extract.
    Union(Vec<String>),
    /// `type Foo = A & B` — branch types stored for the same future use.
    Intersection(Vec<String>),
    /// `type Foo = { ... }` — members are emitted as Property/Method
    /// symbols by the existing `recurse_for_object_types` walk, so chain
    /// walking against the alias name already finds them via members_of.
    /// No expansion needed here; the marker exists so callers can tell
    /// "this alias has its own structural shape" from "we don't know".
    Object,
    /// `type Foo = typeof someValue` — the alias evaluates to the *type*
    /// of a value reference. The chain walker dereferences this by
    /// looking up the value's `field_type` (or `return_type` if it's a
    /// function) and continuing with that. The string is the value's
    /// referenced name as written in the source (e.g. `"api"`,
    /// `"users.get"`). PR 10.
    Typeof(String),
    /// Anything else — `keyof`, mapped, conditional, indexed,
    /// template-literal types, etc. These need their own machinery in
    /// later PRs. Chain walkers must NOT treat this as `Application`.
    Other,
}

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
    /// **Canonical exported name** in the resolved module.
    ///
    /// Post-import-resolution invariant: `target_name` is the name as
    /// defined in `module`'s export list, not the local alias from the
    /// importing file. For `import { foo as bar } from 'pkg'; bar()`
    /// the ref carries `target_name="foo"`, not `"bar"`. For
    /// `import * as F from 'pkg'; F.A.B` the ref carries
    /// `target_name="B"`, `namespace_segments=["A"]`, `module="pkg"`.
    ///
    /// For chain-bearing call refs, this is the LAST segment name (the
    /// method/property at the chain leaf).
    pub target_name: String,
    pub kind: EdgeKind,
    /// 0-based source line of the reference site.
    pub line: u32,
    /// **Resolved final module** the target is exported from.
    ///
    /// Set by per-ecosystem import-resolution passes (see
    /// `ecosystem::npm::imports::resolve_import_refs` and analogues).
    /// `None` means "no import context — bare identifier" (e.g. a local
    /// variable, an in-file type, or a name we couldn't trace back to
    /// any import).
    pub module: Option<String>,
    /// Intermediate namespace segments between `module` and
    /// `target_name`. Empty for plain refs. Populated when a ref like
    /// `Foo.Bar.Baz` resolves to module `pkg`, target `Baz` with
    /// `namespace_segments=["Bar"]` — the resolver walks these as
    /// nested-namespace steps when looking up the canonical symbol.
    ///
    /// Most languages and most refs leave this empty. Currently used by
    /// ECMAScript-family extractors (TS/JS/JSX/TSX/Vue/Svelte/Astro
    /// scripts) and Dart.
    pub namespace_segments: Vec<String>,
    /// Structured member access chain from tree-sitter AST.
    /// `None` for simple identifier refs (e.g., `foo()`, import bindings, type refs).
    pub chain: Option<MemberChain>,
    /// R5 byte offset of the ref site (the node where the ref was emitted).
    /// Used by the resolver cursor (for narrowing lookups) and by the shared
    /// `indexer/flow` runner to correlate assignment RHS byte ranges with
    /// specific refs. `0` means "not populated" — languages whose extractors
    /// haven't been wired up yet emit this as default.
    pub byte_offset: u32,
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
#[derive(Default)]
pub struct ExtractionResult {
    pub symbols: Vec<ExtractedSymbol>,
    pub refs: Vec<ExtractedRef>,
    pub routes: Vec<ExtractedRoute>,
    pub db_sets: Vec<ExtractedDbSet>,
    pub has_errors: bool,
    /// Cross-service wiring artifacts spotted by the plugin. Stage 3 of the
    /// refactored pipeline folds these across files into `flow_edges` rows
    /// via in-memory Start×Stop matching. Empty default; plugins fill it
    /// one framework at a time as they migrate connector detection.
    pub connection_points: Vec<ConnectionPoint>,
    /// `(module_path, symbol_name)` pairs this file contributes to the
    /// shared demand accumulator — the subset of `refs` whose target is an
    /// external import. Used by Stage 2's demand-driven external parser to
    /// decide which external files to pull. Empty default; plugins fill it
    /// one ecosystem at a time as they migrate.
    pub demand_contributions: Vec<(String, String)>,
    /// Structural shape of every type alias emitted in this file.
    /// Pairs the alias's qualified name with its `AliasTarget`. Only TS
    /// populates this today; other languages leave it empty and the engine
    /// derives an `Application` shape from `field_type` for their typedefs.
    pub alias_targets: Vec<(String, AliasTarget)>,
}

// ---------------------------------------------------------------------------
// Connection points — pipeline-refactor scaffolding
// ---------------------------------------------------------------------------
//
// A ConnectionPoint is an intermediate value a language plugin emits when its
// AST walk spots a cross-service / cross-module wiring artifact: an HTTP
// route handler (Start), an `http.Client.Get` call (Stop), a DI registration
// (Start), an `IMediator.Send` call (Stop), a Tauri `#[command]` (Start), an
// `invoke()` call (Stop), etc.
//
// Stage 3 of the refactored pipeline folds connection points across all
// parsed files into `flow_edges` rows by matching Start×Stop pairs with the
// same (kind, key). No DB round-trip, no connector re-parse.
//
// These types are scaffolding: the field does not yet live on ParsedFile /
// ExtractionResult and no plugin emits them. Added here so downstream
// wiring work has a target to reference.

/// What wiring category a connection point belongs to. Match keys only
/// compare within the same kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionKind {
    /// HTTP REST wiring: route handlers ↔ client calls.
    Rest,
    /// gRPC service method ↔ client stub call.
    Grpc,
    /// GraphQL resolver ↔ client query / mutation.
    GraphQL,
    /// Dependency-injection registration ↔ consumer site.
    Di,
    /// Inter-process command / handler (Tauri `#[command]`, Electron
    /// `ipcMain.handle`) ↔ invoke call site.
    Ipc,
    /// Event publish ↔ subscribe (pub/sub, domain events).
    Event,
    /// Message queue producer ↔ consumer (Kafka, RabbitMQ, etc.).
    MessageQueue,
    /// Route declaration only (framework routing table entry) —
    /// matching to handlers happens via the handler's symbol qname, not
    /// via Start×Stop pairing.
    Route,
}

/// Whether a connection point is the producing or consuming side of the
/// pairing. Matched pairs have the same `kind` and `key`, one role each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionRole {
    /// Producer / server / handler side.
    Start,
    /// Consumer / client / caller side.
    Stop,
}

/// A cross-file wiring datum emitted by a language plugin during extraction.
///
/// `key` is the comparison anchor used during Stage 3's in-memory match
/// reduce. Canonical shapes per kind:
///   - `Rest`: `"METHOD /path/template"` (e.g. `"GET /users/:id"`).
///   - `Grpc`: `"package.Service/Method"`.
///   - `GraphQL`: `"Query.fieldName"` or `"Mutation.fieldName"`.
///   - `Di`: the registered service's type qname.
///   - `Ipc`: the command / channel name literal.
///   - `Event`: the event topic / type name.
///   - `MessageQueue`: the queue or topic name.
///   - `Route`: the route template, same shape as `Rest` starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionPoint {
    pub kind: ConnectionKind,
    pub role: ConnectionRole,
    pub key: String,
    /// 1-based line number of the emitting construct within its source file.
    pub line: u32,
    /// 1-based column number.
    pub col: u32,
    /// Qualified name of the owning symbol (the function declaration, the
    /// class body, the route constant, etc.). Empty when the connection
    /// point isn't attached to a nameable symbol.
    pub symbol_qname: String,
    /// Optional free-form metadata (e.g. framework name `"gin"`, service
    /// identifier `"UserService"`). Stored as a small map so individual
    /// connectors don't need bespoke struct variants.
    pub meta: HashMap<String, String>,
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
    /// Shell / script payload embedded in a build-tool directive:
    /// Go `//go:generate`, CMake `COMMAND`, Bazel/Starlark `genrule`
    /// `cmd`, Terraform/HCL `user_data` / `provisioner`, Bicep/ARM
    /// `scriptContent`. Produces a region (usually bash or
    /// powershell) whose refs and symbols should count toward
    /// aggregate stats — unlike snippets, these commands actually
    /// run at build/deploy time.
    BuildToolShell,
}

impl ExtractionResult {
    pub fn new(
        symbols: Vec<ExtractedSymbol>,
        refs: Vec<ExtractedRef>,
        has_errors: bool,
    ) -> Self {
        Self {
            symbols,
            refs,
            routes: Vec::new(),
            db_sets: Vec::new(),
            has_errors,
            connection_points: Vec::new(),
            demand_contributions: Vec::new(),
            alias_targets: Vec::new(),
        }
    }

    pub fn with_connectors(
        symbols: Vec<ExtractedSymbol>,
        refs: Vec<ExtractedRef>,
        routes: Vec<ExtractedRoute>,
        db_sets: Vec<ExtractedDbSet>,
        has_errors: bool,
    ) -> Self {
        Self {
            symbols,
            refs,
            routes,
            db_sets,
            has_errors,
            connection_points: Vec::new(),
            demand_contributions: Vec::new(),
            alias_targets: Vec::new(),
        }
    }

    pub fn empty() -> Self {
        Self {
            symbols: Vec::new(),
            refs: Vec::new(),
            routes: Vec::new(),
            db_sets: Vec::new(),
            has_errors: false,
            connection_points: Vec::new(),
            demand_contributions: Vec::new(),
            alias_targets: Vec::new(),
        }
    }
}

/// A detected package within a monorepo / workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfo {
    /// Database row ID (assigned after INSERT).
    pub id: Option<i64>,
    /// Package name (folder-derived key, stable for sort and path matching).
    pub name: String,
    /// Relative path from workspace root to package directory.
    pub path: String,
    /// Ecosystem hint: "npm", "cargo", "dotnet", "go", etc.
    pub kind: Option<String>,
    /// Relative path to the manifest file (e.g., "services/api/package.json").
    pub manifest: Option<String>,
    /// The package name as declared in its own manifest — `package.json`
    /// `name`, `Cargo.toml` `[package].name`, `.csproj` filename stem, etc.
    /// Distinct from `name` which is the folder-derived key. Used by
    /// resolvers to match import specifiers like `@myorg/utils` to the
    /// correct workspace package. `None` when the manifest didn't declare
    /// a name or couldn't be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_name: Option<String>,
}

/// A conditional-narrowing scope captured at extraction time.
///
/// Each `Narrowing` describes a half-open byte range `[byte_start, byte_end)`
/// inside the source file where a local variable named `name` should be
/// treated as having type `narrowed_type` instead of its forward-inferred
/// type. Populated by language plugins via the shared `indexer::flow` query
/// runner (TS `instanceof`, `typeof`, user-defined predicates; Python
/// `isinstance`; Rust `if let`/`match`; etc.).
///
/// Consumed by `LocalTypeCache::lookup` on each chain-walker call — the
/// cursor (the current ref's byte offset) selects which narrowing is active.
#[derive(Debug, Clone)]
pub struct Narrowing {
    pub name: String,
    pub narrowed_type: String,
    pub byte_start: u32,
    pub byte_end: u32,
}

/// Per-file flow-typing metadata, produced by the shared `indexer::flow`
/// query runner and consumed by the resolver/chain-walker pair.
///
/// All fields default to empty — languages that have not wired up
/// `FlowConfig` queries yet pay zero cost, and the resolver loop degrades
/// gracefully (no forward inference, no narrowing).
///
/// Fields:
/// - `narrowings`: conditional-narrowing scopes (see `Narrowing`).
/// - `flow_binding_lhs`: sparse map `ref_idx → lhs_symbol_idx`. Present when
///   a ref is the RHS of `<lhs> = <chain>`; the resolver records the resolved
///   yield type against the named LHS symbol in the file's local-type cache.
/// - `ref_byte_offsets`: parallel to `refs`; the byte offset of each ref's
///   site in the source. Empty means "unknown — treat as 0" (used as a
///   cursor when looking up narrowings). Same convention as
///   `symbol_origin_languages`.
#[derive(Debug, Default, Clone)]
pub struct FlowMeta {
    pub narrowings: Vec<Narrowing>,
    pub flow_binding_lhs: HashMap<usize, usize>,
    pub ref_byte_offsets: Vec<u32>,
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
    /// Origin language per ref (indexed same as `refs`). `None` means the
    /// ref belongs to the host language (`language` field); `Some(lang_id)`
    /// means the ref was extracted from an embedded region of a different
    /// language (e.g. a JS ref from a `<script>` block inside a `.ex` HEEx
    /// file). Always the same length as `refs`, or empty when no embedded
    /// regions were processed (resolver treats empty as all-None / host lang).
    pub ref_origin_languages: Vec<Option<String>>,
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
    /// Per-file flow-typing metadata (forward inference, narrowings, byte
    /// offsets). Default is empty — populated by languages that have wired
    /// up `FlowConfig` queries. See `FlowMeta`.
    pub flow: FlowMeta,
    /// Cross-service wiring points emitted by the language plugin's
    /// `extract_connection_points` path, paired across files by Stage 3 of
    /// the refactored pipeline into `flow_edges`. Empty default; plugins
    /// fill this one framework at a time.
    pub connection_points: Vec<ConnectionPoint>,
    /// `(module_path, symbol_name)` contributions this file makes to the
    /// shared demand accumulator (the refs whose target is an external
    /// import). Stage 2's demand-driven external parser reads this to
    /// decide which external files to pull. Kept on the struct for
    /// diagnostics even after the accumulator consumes it. Empty default.
    pub demand_contributions: Vec<(String, String)>,
    /// Structural shape of every type alias emitted in this file.
    /// Pairs the alias's qualified name with its `AliasTarget`. Consumed
    /// by `SymbolIndex::build_with_context` to build the project-wide
    /// alias_target map used by chain walkers for alias expansion.
    pub alias_targets: Vec<(String, AliasTarget)>,
}

impl ParsedFile {
    /// Drop per-file fields whose only consumers (write / FTS / chunks /
    /// route table / db_set table) have already read them. `symbols`,
    /// `refs`, `flow`, `connection_points`, and the origin vectors stay —
    /// resolution, connector detection, and flow matching read them
    /// downstream. Call this after `write_parsed_files*` returns, for
    /// both internal and external files. Frees hundreds of MB on big
    /// .NET / TS workspaces where external `content` would otherwise
    /// live until the end of resolve.
    pub fn slim_for_resolve(&mut self) {
        self.content = None;
        self.routes = Vec::new();
        self.db_sets = Vec::new();
    }
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
    /// Unresolved refs whose source symbol lives in a project-internal file.
    /// External-origin sources (node_modules .d.ts, Go pkg/mod, etc.) are
    /// excluded so the metric reflects the health of real project code only.
    pub unresolved_ref_count: u32,
    /// Unresolved refs originating from externally-indexed files (informational).
    pub unresolved_ref_count_external: u32,
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
