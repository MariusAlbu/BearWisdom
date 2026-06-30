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

use crate::type_checker::core::types::{TypeArena, TypeId};

// ---------------------------------------------------------------------------
// Enumerations
// ---------------------------------------------------------------------------

/// The syntactic kind of a symbol.
///
/// C# adds Namespace, Field, Event, Delegate that v1 was missing.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    strum::AsRefStr,
    strum::IntoStaticStr,
    strum::EnumString,
    strum::Display,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum SymbolKind {
    Class,
    Struct,
    Interface,
    Trait,
    Enum,
    EnumMember,
    Method,
    Constructor,
    Property,
    Field,
    Namespace,
    Module,
    Event,
    Delegate,
    Function,
    TypeAlias,
    Variable,
    Parameter,
    Test,
}

impl SymbolKind {
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// Kinds of directed edges in the code graph.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    strum::AsRefStr,
    strum::IntoStaticStr,
    strum::EnumString,
    strum::Display,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum EdgeKind {
    /// Function or method invocation.
    Calls,
    /// Class → parent class.
    Inherits,
    /// Class/struct → interface or trait.
    Implements,
    /// Parameter, return type, or field references a type.
    TypeRef,
    /// `new Foo()` / `Foo()` instance construction.
    Instantiates,
    /// `using` / `import` / `require` brings a namespace or module into
    /// scope. `target_name` and `module` carry the full path.
    Imports,
    /// Variable or field read.
    Reads,
    /// Assignment to a variable or field.
    Writes,
    /// HTTP call → backend route. Kept for back-compat alongside
    /// `FlowEdgeKind::HttpCall`; new emitters use FlowEdgeKind.
    HttpCall,
    /// ORM mapping. Kept for back-compat alongside `FlowEdgeKind::DbEntity`.
    DbEntity,
    /// Edge discovered by an LSP server only. Kept for back-compat
    /// alongside `FlowEdgeKind::LspResolved`.
    LspResolved,
}

/// Cross-tier flow edges. Distinct from `EdgeKind` resolution edges: these
/// land in the `flow_edges` table; resolution edges land in `edges`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    strum::AsRefStr,
    strum::IntoStaticStr,
    strum::EnumString,
    strum::Display,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FlowEdgeKind {
    /// Frontend fetch → backend route.
    HttpCall,
    /// GraphQL operation (query, mutation, subscription).
    #[strum(serialize = "graphql_op")]
    #[serde(rename = "graphql_op")]
    GraphQLOp,
    /// gRPC / JSON-RPC caller.
    RpcCall,
    /// gRPC / JSON-RPC handler.
    RpcHandle,
    /// WebSocket message.
    #[strum(serialize = "websocket")]
    #[serde(rename = "websocket")]
    WebSocket,
    /// IPC command (e.g. Tauri `invoke`, Electron `ipcRenderer.invoke`).
    IpcCall,
    /// Background job producer.
    BgJob,
    /// Email / push template dispatch.
    Mailer,
    /// ORM mapping to table.
    DbEntity,
    /// Query or mutation against a known entity.
    DbQuery,
    /// Migration script for a table.
    MigrationTarget,
    /// Event producer.
    EventEmit,
    /// Event consumer.
    EventHandle,
    /// Queue producer.
    QueueProduce,
    /// Queue consumer.
    QueueConsume,
    /// DI interface → implementation binding.
    DiBinding,
    /// Environment variable / config key read.
    ConfigLookup,
    /// Feature flag evaluation.
    FeatureFlag,
    /// Authorization requirement on a handler.
    AuthGuard,
    /// CLI command registration.
    CliCommand,
    /// Scheduled job registration.
    ScheduledJob,
    /// Edge produced by an LSP server.
    LspResolved,
}

impl FlowEdgeKind {
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

impl EdgeKind {
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// C# and TypeScript visibility modifiers.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    strum::AsRefStr,
    strum::IntoStaticStr,
    strum::EnumString,
    strum::Display,
)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum Visibility {
    Public,
    Private,
    Protected,
    Internal,
    PackagePrivate,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// `type Foo = [A, B]` — a tuple type. Each element's head type is stored by
    /// position, so a positional access (an array-destructure `const [a, b] = x`,
    /// emitted as a `tuple_index:N` ComputedAccess segment) selects element N.
    /// A labeled tuple (`[get: A, set: B]`) drops the labels — only the element
    /// types matter for positional access.
    Tuple(Vec<String>),
    /// `type Foo = A & B & { [K in keyof T]: V }` — an intersection that carries
    /// BOTH named branches AND an anonymous mapped branch. Member lookup must try
    /// the named branches (`MockInstance.mockImplementation`) AND the mapped
    /// source (`T`'s own members), so neither half is dropped. `branches` are the
    /// named heads; `source` / `value_template` mirror [`AliasTarget::Mapped`].
    IntersectionMapped {
        branches: Vec<String>,
        source: String,
        value_template: String,
    },
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
    /// `type Foo = keyof T` — produces a union of `T`'s property names
    /// as string-literal types. Not expandable as a chain head (you
    /// can't call a method on a string union), so chain walkers always
    /// miss against a `Keyof` alias. Captured for downstream consumers
    /// (`T[keyof T]` indexed access in PR 12, `{[K in keyof T]: ...}`
    /// mapped types in PR 13) that need to enumerate `T`'s members.
    /// PR 11.
    Keyof(String),
    /// `type Foo = T[K]` — indexed access. Extracts the type of
    /// property `K` from object type `T`. Resolution paths:
    /// - Literal string key (`T["foo"]`): look up
    ///   `field_type(T.foo)` and return that as the head.
    /// - Generic param key (`T[K]` where K is bound in env): resolve
    ///   K via the type environment to a literal, then look up.
    /// - `keyof T` key, union, etc.: union of all member types —
    ///   deferred (no single head). PR 12.
    IndexedAccess { object: String, key: String },
    /// `type Foo<T> = { [K in keyof T]: U }` — mapped type.
    /// `source` is the keyof target (the `T` in `[K in keyof T]`);
    /// empty when the source isn't a `keyof T` shape.
    /// `value_template` is the index-signature value type as written
    /// (e.g. `"T[K]"`, `"V"`, `"boolean"`).
    /// PR 15 expands the *transparent* pattern: when value_template
    /// is syntactically `{source}[{key_var}]` (covers
    /// `Partial<T>` / `Required<T>` / `Readonly<T>`), member access
    /// on the mapped type falls through to the source's members —
    /// so `Partial<User>.name` resolves to `User.name`.
    /// Other shapes (`Record<K, V>`, custom mapped types where the
    /// value template doesn't reference T[K]) return None. PR 13/15.
    Mapped {
        source: String,
        value_template: String,
    },
    /// `type Foo<T> = T extends U ? X : Y` — conditional type. The
    /// four type expressions are stored as written (each reduced to
    /// its head name); the chain walker consults
    /// `is_assignable_to(check, extends)` and picks the deciding
    /// branch, returning None when the subtype check is undecidable.
    ///
    /// `infer_binding` records an `infer` capture in the `extends`
    /// clause: `Some((var, slot))` for
    /// `type Elem<T> = T extends Array<infer U> ? U : never`, where
    /// `var` is the introduced variable (`"U"`) and `slot` is its
    /// positional index in `extends`'s type arguments (`0`). The
    /// expander binds `var` to the checked type's `Apply` arg at
    /// `slot` when that type is a known `Apply { extends-head, args }`
    /// and the true branch IS `var`. `None` for conditionals without
    /// an `infer`, and for multi-`infer` extends clauses (the engine
    /// records only the single-capture case it can resolve).
    Conditional {
        check: String,
        extends: String,
        true_branch: String,
        false_branch: String,
        infer_binding: Option<(String, usize)>,
    },
    /// Anything else — template-literal types, function types,
    /// tuples, infer, type predicates, this — chain walkers must NOT
    /// treat as `Application`.
    Other,
}

/// Pre-interned form of [`AliasTarget`] stored in the Compilation's
/// `alias_target` map. Every type-expression `String` field is replaced by a
/// `TypeId` obtained via the same `TypeArena::intern_type_str` / `class` call
/// the old expand-alias path made at lookup time; non-type fields (value paths,
/// key literals, value-template strings, binding-variable names) stay `String`.
///
/// `Typeof` keeps its string because the payload is a VALUE path (`"users.get"`),
/// not a type expression. `IndexedAccess.key` keeps its string because it is a
/// property-name literal or generic-parameter name, not a standalone type head.
/// `Mapped.value_template` / `IntersectionMapped.value_template` keep their
/// strings because consumers pattern-match them as template expressions.
/// `Conditional.infer_binding.0` keeps its string because it is a binding
/// variable name, not a type expression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AliasTargetIds {
    Application { root: TypeId, args: Vec<TypeId> },
    Union(Vec<TypeId>),
    Intersection(Vec<TypeId>),
    Tuple(Vec<TypeId>),
    IntersectionMapped {
        branches: Vec<TypeId>,
        source: TypeId,
        value_template: String,
    },
    Object,
    Typeof(String),
    Keyof(TypeId),
    IndexedAccess { object: TypeId, key: String },
    Mapped { source: TypeId, value_template: String },
    Conditional {
        check: TypeId,
        extends: TypeId,
        true_branch: TypeId,
        false_branch: TypeId,
        infer_binding: Option<(String, usize)>,
    },
    Other,
}

/// Intern every type-expression component of an [`AliasTarget`] into the
/// arena, producing the pre-interned [`AliasTargetIds`] form stored in the
/// Compilation map. The resulting TypeIds are identical to those the old
/// `expand_alias` path produced by calling `intern_type_str` / `class` at
/// lookup time — the intern is just moved earlier, to map-build.
pub fn intern_alias_target(arena: &TypeArena, t: &AliasTarget) -> AliasTargetIds {
    match t {
        AliasTarget::Application { root, args } => AliasTargetIds::Application {
            root: arena.class(root),
            args: args.iter().map(|a| arena.intern_type_str(a)).collect(),
        },
        AliasTarget::Union(branches) => {
            AliasTargetIds::Union(branches.iter().map(|b| arena.intern_type_str(b)).collect())
        }
        AliasTarget::Intersection(branches) => {
            AliasTargetIds::Intersection(branches.iter().map(|b| arena.intern_type_str(b)).collect())
        }
        AliasTarget::Tuple(elems) => {
            AliasTargetIds::Tuple(elems.iter().map(|e| arena.intern_type_str(e)).collect())
        }
        AliasTarget::IntersectionMapped { branches, source, value_template } => {
            AliasTargetIds::IntersectionMapped {
                branches: branches.iter().map(|b| arena.intern_type_str(b)).collect(),
                source: arena.intern_type_str(source),
                value_template: value_template.clone(),
            }
        }
        AliasTarget::Object => AliasTargetIds::Object,
        AliasTarget::Typeof(s) => AliasTargetIds::Typeof(s.clone()),
        AliasTarget::Keyof(s) => AliasTargetIds::Keyof(arena.intern_type_str(s)),
        AliasTarget::IndexedAccess { object, key } => AliasTargetIds::IndexedAccess {
            object: arena.intern_type_str(object),
            key: key.clone(),
        },
        AliasTarget::Mapped { source, value_template } => AliasTargetIds::Mapped {
            source: arena.intern_type_str(source),
            value_template: value_template.clone(),
        },
        AliasTarget::Conditional { check, extends, true_branch, false_branch, infer_binding } => {
            AliasTargetIds::Conditional {
                check: arena.intern_type_str(check),
                extends: arena.intern_type_str(extends),
                true_branch: arena.intern_type_str(true_branch),
                false_branch: arena.intern_type_str(false_branch),
                infer_binding: infer_binding.clone(),
            }
        }
        AliasTarget::Other => AliasTargetIds::Other,
    }
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
    /// Absolute byte position of the declaration start in the source file.
    pub byte_offset: u32,
    /// Function/method signature string, e.g. "Task<User> GetById(int id)".
    pub signature: Option<String>,
    /// C# XML doc comment or JSDoc, if present.
    pub doc_comment: Option<String>,
    /// Scope path (ancestors, dot-separated) — used for DB `scope_path` column.
    pub scope_path: Option<String>,
    /// Index of this symbol's parent in the same Vec<ExtractedSymbol>.
    pub parent_index: Option<usize>,
    /// Annotated or inferred type for value-holding kinds (Variable, Field,
    /// Property, Parameter). Interned into the workspace TypeArena.
    pub declared_type: Option<crate::type_checker::core::types::TypeId>,
    /// Return type for callable kinds (Function, Method, Constructor) and
    /// the self-TypeId for type-defining kinds (Class, Struct, Interface,
    /// Trait, Enum, TypeAlias).
    pub return_type: Option<crate::type_checker::core::types::TypeId>,
    /// Parameter types in declaration order. Empty for non-callable kinds.
    pub param_types: Vec<crate::type_checker::core::types::TypeId>,
    /// Generic parameter slots bound by this symbol.
    pub generic_params: Vec<crate::type_checker::core::types::GenericParamId>,
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
    /// Absolute byte position of this segment's identifier in the source file.
    pub byte_offset: u32,
    /// Canonical TypeId form of `declared_type`. Populated post-extract by
    /// interning the string into the workspace TypeArena. Consumers
    /// progressively migrate from `declared_type: Option<String>` to this.
    pub declared_type_id: Option<crate::type_checker::core::types::TypeId>,
    /// Canonical TypeId forms of `type_args`. Populated post-extract.
    pub type_arg_ids: Vec<crate::type_checker::core::types::TypeId>,
    /// Whether this segment is invoked (`f()`). Set by the chain builder when
    /// the segment is the function of a `call_expression`. The walker yields a
    /// function-typed member's return type instead of the function value.
    pub is_call: bool,
    /// Call arguments captured at the invoked segment (`obj.find(x).y` — the
    /// `find` segment owns `[x]`). Drives mid-chain argument-driven generic
    /// inference: when this segment is invoked the walker unifies its callee's
    /// declared parameter types against these args to bind type parameters its
    /// return resolves through. Empty unless the chain builder populates it on
    /// an invoked mid-chain segment; the terminal call's args live on
    /// `ExtractedRef.call_args`, never here.
    pub call_args: Vec<CallArg>,
}

/// A structured member access chain built from tree-sitter AST nodes.
#[derive(Debug, Clone)]
pub struct MemberChain {
    pub segments: Vec<ChainSegment>,
}

// ---------------------------------------------------------------------------
// Extracted types (parser output, pre-resolution)
// ---------------------------------------------------------------------------

/// A single argument in a call expression, captured at extract time.
///
/// Populated only for call-site refs (`EdgeKind::Calls`, `EdgeKind::Imports`)
/// when the extractor walks the argument list. The `Other` arm covers any
/// argument shape not worth preserving: function references, complex
/// expressions, spread elements, etc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallArg {
    /// Plain string literal: `"users"`, `'users'`.
    StringLit(String),
    /// Template literal with interpolation holes replaced by `{}`:
    /// `` `/api/users/${id}` `` → `"/api/users/{}"`.
    TemplateLit(String),
    /// Bare identifier reference — carries the name so the consumer can
    /// chase it to the binding's own `call_args`.
    Ident(String),
    /// Tagged template: `` gql`query Foo { users { id } }` ``.
    /// `tag` is the tag identifier (`"gql"`, `"sql"`, `"html"`).
    /// `body` is the raw inner text with interpolation holes removed.
    TaggedTemplate { tag: String, body: String },
    /// Numeric, boolean, null, or simple array/object literal — stored as
    /// its source text.
    Literal(String),
    /// Object literal whose property names are statically determinable, e.g.
    /// `{ template: 'welcome', subject: s }`. Captured as the ordered list of
    /// `(key, optional string-literal value)` pairs. The value is `Some(_)`
    /// only when the property's value is a plain string or template literal
    /// (no interpolation) — identifiers, function references, computed
    /// expressions all produce `None`. Used by mailer detectors that need
    /// the `template:` value AND by handler-registration detectors that only
    /// need the keys (`server.addService(SvcDef, { m1: h, m2: h })`).
    ObjectKeys(Vec<(String, Option<String>)>),
    /// Conditional (ternary) expression: `cond ? then_branch : else_branch`.
    /// The condition is discarded; both value branches are preserved for
    /// downstream typing of the result type.
    Ternary {
        then_branch: Box<CallArg>,
        else_branch: Box<CallArg>,
    },
    /// Array literal: `[elem0, elem1, ...]`. Each element is a nested `CallArg`
    /// so spread elements inside are represented as `Spread` children.
    ArrayLiteral { elements: Vec<CallArg> },
    /// Awaited expression: `await expr`. Carries the inner expression so the
    /// resolver can unwrap the promise type.
    Await { expr: Box<CallArg> },
    /// Spread element: `...expr`. Carries the inner expression.
    Spread { expr: Box<CallArg> },
    /// Subscript / index access: `container[index]`.
    IndexAccess {
        container: Box<CallArg>,
        index: Box<CallArg>,
    },
    /// Binary expression: `left op right`. `op` is the operator source text
    /// (e.g. `"+"`, `"&&"`).
    Binary {
        op: String,
        left: Box<CallArg>,
        right: Box<CallArg>,
    },
    /// Arrow-function or function-expression argument (`x => x.foo`,
    /// `function (a, b) { ... }`). `params` are the lambda's own positional
    /// parameter identifiers in declaration order. A parameter whose binding
    /// is not a plain identifier (destructuring / rest pattern) contributes an
    /// empty string in its slot so positions stay aligned with the callback
    /// signature. Carries the names only — they are the keys the chain walker
    /// seeds the local-type cache under after typing each param from the
    /// higher-order method's callback-parameter signature.
    Lambda { params: Vec<String> },
    /// Any argument shape not covered by the above variants.
    Other,
}

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
    /// 0-based column of the reference site identifier.
    pub col: u32,
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
    /// Positional arguments at this call site, populated only by extractors
    /// that walk call-expression argument lists. Empty for non-call refs
    /// (TypeRef, Inherits, Implements) and for extractors that haven't been
    /// wired yet. The resolver's flow-emission hook uses the first argument
    /// for URL pattern extraction (HTTP calls, IPC commands, gql tags, etc.).
    pub call_args: Vec<CallArg>,
    /// True only on an import STATEMENT's own binding ref — the `Foo` in
    /// `import { Foo } from 'pkg'` — whose `source_symbol_index` is a positional
    /// artifact (the next-to-be-defined symbol), not a real type attribution.
    /// The per-symbol type-map build excludes these so they never pollute a
    /// field/return type; module-tagged USAGE `TypeRef`s are kept.
    pub is_import_binding: bool,
    /// True when an `Imports` ref is a re-export (`export ... from`, Rust
    /// `pub use`, a Python package `__init__` re-export) rather than a private
    /// import. The re-export map is built only from these, so a name imported
    /// through a re-export hop follows to its declaring symbol while a private
    /// `use`/`import` does not.
    pub is_reexport: bool,
}

/// An HTTP route attribute extracted from C#.
///
/// Built up during extraction: `[HttpGet("/api/catalog/{id}")]` produces one.
/// The connector later matches these against TS fetch/axios calls.
#[derive(Debug, Clone)]
pub struct ExtractedRoute {
    /// Index into Vec<ExtractedSymbol> for the handler method.
    pub handler_symbol_index: usize,
    pub http_method: String, // "GET", "POST", "PUT", "DELETE", "PATCH"
    pub template: String,    // e.g. "/api/catalog/items/{id:int}"
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
    pub fn new(symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>, has_errors: bool) -> Self {
        Self {
            symbols,
            refs,
            routes: Vec::new(),
            db_sets: Vec::new(),
            has_errors,
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
    /// Whether the package's public surface is reachable from outside the
    /// workspace. Drives the dead-code `exported_api` entry-point
    /// contributor: a workspace-internal helper crate (`publish = false`
    /// in `Cargo.toml`, `"private": true` in `package.json`) does NOT
    /// auto-root its public symbols as reachability anchors. Default
    /// `true` preserves prior behavior on manifests without a private
    /// signal.
    #[serde(default = "default_is_publishable")]
    pub is_publishable: bool,
}

fn default_is_publishable() -> bool {
    true
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

/// A discriminated-union narrowing captured at extraction time.
///
/// `if (shape.kind === "circle") { ... }` narrows `shape` to the union branch
/// whose `prop` discriminant property carries the literal `literal`. Only the
/// resolver — which has the type index — can map a literal to a branch, so
/// extraction records the receiver `name`, the discriminant property `prop`,
/// the matched `literal` (with its source quotes, as the branch's literal-typed
/// field is stored), and the half-open byte range `[byte_start, byte_end)` of
/// the guarded block. The chain walker selects the branch when it resolves a
/// union-typed receiver whose ref cursor falls in range.
#[derive(Debug, Clone)]
pub struct DiscriminantNarrowing {
    pub name: String,
    pub prop: String,
    pub literal: String,
    pub byte_start: u32,
    pub byte_end: u32,
    /// When true the guard is negated (`if (x.kind !== "lit") return;`): the
    /// narrowing holds for the range AFTER the early-exit guard, and selects
    /// the union branches whose discriminant is NOT `literal`.
    pub negate: bool,
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
/// - `flow_binding_destructure`: sparse map `ref_idx → [(lhs_symbol_idx,
///   field_key)]`. Present when a ref is the RHS of an object-destructure
///   `const { a, b: c } = <chain>`; the resolver types each binding from the
///   FIELD `field_key` on the resolved yield type, not from the whole object.
/// - `flow_binding_decl_type`: sparse map `lhs_symbol_idx → declared type text`.
///   Present when a binding carries an explicit annotation (`let x: T`). Unlike
///   `flow_binding_lhs` this needs no RHS resolution — the annotation is the
///   type — so the resolver seeds it into the local-type cache directly.
/// - `flow_binding_unwrap`: set of `lhs_symbol_idx` whose initializer applies a
///   fallible-unwrap operator (Rust `?`). The operator guarantees the RHS is a
///   single-arg fallible wrapper (`Result<T>` / `Option<T>`), so the resolver
///   peels one generic layer off the resolved yield before recording it.
/// - `flow_binding_await`: set of `lhs_symbol_idx` whose initializer is an
///   `await` expression. The resolver strips one async-wrapper layer off the
///   resolved yield type before recording it: `Promise<T>` → `T` (head in
///   `LanguageProfile::async_wrappers`), bare wrapper with no arg → unchanged.
/// - `flow_return_lhs`: sparse map `ref_idx → fn_symbol_idx`. Present when a
///   ref is a `return <expr>` expression in a function/method body; the
///   resolver records the resolved yield type as a candidate return type for
///   the named function symbol, joining candidates across the function's
///   returns and propagating the inferred type through the resolve fixpoint
///   (INFER-3 / INFER-2). The mirror of `flow_binding_lhs` for returns.
/// - `ref_byte_offsets`: parallel to `refs`; the byte offset of each ref's
///   site in the source. Empty means "unknown — treat as 0" (used as a
///   cursor when looking up narrowings). Same convention as
///   `symbol_origin_languages`.
#[derive(Debug, Default, Clone)]
pub struct FlowMeta {
    pub narrowings: Vec<Narrowing>,
    pub discriminant_narrowings: Vec<DiscriminantNarrowing>,
    pub flow_binding_lhs: HashMap<usize, usize>,
    /// Destructured bindings of an RHS expression: `const { a, b: c } = f()`.
    /// Maps the RHS `ref_idx` to each binding's `(lhs_symbol_idx, field_key)` —
    /// `a` → `(idx_a, "a")`, `b: c` → `(idx_c, "b")`. Distinct from
    /// `flow_binding_lhs` because each binding types from the named FIELD on the
    /// RHS's yield type (`R["a"]`), not from the whole object `R`. A single RHS
    /// ref carries one entry per destructured binding.
    pub flow_binding_destructure: HashMap<usize, Vec<(usize, String)>>,
    pub flow_binding_decl_type: HashMap<usize, String>,
    pub flow_binding_unwrap: std::collections::HashSet<usize>,
    pub flow_binding_await: std::collections::HashSet<usize>,
    pub flow_return_lhs: HashMap<usize, usize>,
    /// `(fn_symbol_idx, identifier)` for a `return <bare-identifier>` whose
    /// expression carries no ref — `return queryClient` / `return client`. The
    /// ref-based `flow_return_lhs` misses these (a bare param/local read emits no
    /// ref), so the resolver types the identifier against the function's
    /// parameters / locals and records the result as a return-type candidate.
    pub flow_return_ident: Vec<(usize, String)>,
    /// `(fn_symbol_idx, member_names)` for a function whose body returns an object
    /// literal (`return { info, error }`). A synthetic `{fn}$Ret` object type
    /// carrying these members is materialized post-extract, and a call to the
    /// function yields that type — so `fn().info` resolves to the synthesized member.
    pub flow_return_object: Vec<(usize, Vec<String>)>,
    pub ref_byte_offsets: Vec<u32>,
    /// Per-function control-flow graphs for the file, built at extract time
    /// from the same tree the query runner uses. Empty when the language has
    /// no `CfgNodeKinds` table wired yet — the consumer falls back to the
    /// interval `narrowings` path. Queried by `LocalTypeCache::lookup` via
    /// `fact_string_at(name, cursor)`.
    pub cfg: crate::indexer::flow_cfg::FileCfg,
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
    /// Angular `@Component` selector metadata extracted from TypeScript files.
    ///
    /// Each entry is `(raw_selector, class_qualified_name)`. Raw selector is
    /// the string as it appears in the decorator argument:
    ///   - element selector: `"app-user-card"` → class `"UserCardComponent"`
    ///   - attribute selector: `"appHighlight"` (brackets stripped)
    ///   - class selector: `"my-thing"` (dot stripped)
    ///   - comma list: split into multiple entries.
    ///
    /// Consumed by `SymbolIndex::build_with_context` to build the
    /// project-wide `angular_selectors` map.  Empty for non-TypeScript files
    /// and TypeScript files that contain no `@Component` decorators.
    pub component_selectors: Vec<(String, String)>,
    /// Extractor-time `FlowEmission`s for plugins whose flow detection is
    /// file-structure-based (SDL parsing, .proto file scan) and can't move
    /// to chain-walk resolver-time emission. Accumulated by
    /// `indexer/resolve/mod.rs` alongside resolver-emitted flows.
    pub plugin_flow_emissions: Vec<(u32, crate::indexer::resolve::flow_emit::FlowEmission)>,
}

impl ParsedFile {
    /// Drop per-file fields whose only consumers (write / FTS / chunks /
    /// route table / db_set table) have already read them. `symbols`,
    /// `refs`, `flow`, and the origin vectors stay — resolution and flow
    /// matching read them downstream. Call this after
    /// `write_parsed_files*` returns, for
    /// both internal and external files. Frees hundreds of MB on big
    /// .NET / TS workspaces where external `content` would otherwise
    /// live until the end of resolve.
    pub fn slim_for_resolve(&mut self) {
        self.content = None;
        self.routes = Vec::new();
        self.db_sets = Vec::new();
        self.component_selectors = Vec::new();
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
// Entry points (reachability-based dead-code: L1)
// ---------------------------------------------------------------------------

/// One row to insert into the `entry_points` SQL table.
///
/// Contributors (language plugins, connectors, manifest walkers, user
/// overrides) produce these. The reachability BFS treats every distinct
/// `symbol_id` here as a root regardless of how many `(kind, source)`
/// rows reference it.
#[derive(Debug, Clone)]
pub struct EntryPointRow {
    pub symbol_id: i64,
    /// One of: main | route | event | di | exported | lifecycle | test |
    /// user | reflection.
    pub kind: &'static str,
    /// Contributor identifier. Convention: `<area>-<contributor>`, e.g.
    /// `rust-plugin`, `axum-connector`, `manifest-cargo`, `user-roots-toml`.
    pub source: &'static str,
    /// Confidence the symbol is *actually* externally-reachable. Defaults
    /// to 1.0; lowered for heuristic contributors (e.g. lifecycle-name
    /// matches with no annotation evidence).
    pub confidence: f64,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
