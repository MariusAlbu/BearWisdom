// =============================================================================
// indexer/resolve/engine.rs — Resolution engine with per-language rule plugins
//
// The engine dispatches reference resolution to language-specific resolvers
// that apply deterministic scope rules (1.0 confidence). When no language
// resolver can resolve a reference, it falls back to the heuristic resolver.
// =============================================================================

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::type_env::TypeEnvironment;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use rustc_hash::FxHashMap;
use std::collections::HashMap;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Public types used by LanguageResolver implementations
// ---------------------------------------------------------------------------

/// Normalized import entry, built from ExtractedRef data.
#[derive(Debug, Clone)]
pub struct ImportEntry {
    /// The name brought into scope (e.g., "CatalogItem", "Foo").
    pub imported_name: String,
    /// The module/namespace path (e.g., "eShop.Catalog.API.Model", "./foo").
    pub module_path: Option<String>,
    /// Optional alias (e.g., `import { Foo as Bar }` → alias = "Bar").
    pub alias: Option<String>,
    /// Whether this is a wildcard/namespace import (e.g., `using NS;`).
    pub is_wildcard: bool,
}

/// Context for the file being resolved. Built once per file by the resolver.
#[derive(Debug)]
pub struct FileContext {
    /// The file path (relative to project root).
    pub file_path: String,
    /// The language identifier.
    pub language: String,
    /// Imports in this file.
    pub imports: Vec<ImportEntry>,
    /// The namespace/package this file belongs to.
    pub file_namespace: Option<String>,
}

/// Context for a single reference being resolved.
pub struct RefContext<'a> {
    /// The reference itself.
    pub extracted_ref: &'a ExtractedRef,
    /// The source symbol that contains this reference.
    pub source_symbol: &'a ExtractedSymbol,
    /// The scope chain at the reference site, innermost first.
    /// Built from scope_path: e.g., ["Foo.Bar.Baz", "Foo.Bar", "Foo"].
    pub scope_chain: Vec<String>,
}

/// The result of a successful resolution.
#[derive(Debug)]
pub struct Resolution {
    /// The DB ID of the resolved target symbol.
    pub target_symbol_id: i64,
    /// Confidence level (1.0 for deterministic, lower for heuristic).
    pub confidence: f64,
    /// Which strategy produced this resolution (for diagnostics).
    pub strategy: &'static str,
}

/// Flattened symbol info used during resolution lookups.
#[derive(Debug, Clone)]
pub struct SymbolInfo {
    pub id: i64,
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub visibility: Option<String>,
    pub file_path: String,
    pub scope_path: Option<String>,
}

// ---------------------------------------------------------------------------
// SymbolLookup trait — decouples resolvers from index internals
// ---------------------------------------------------------------------------

/// Read-only access to the global symbol index.
pub trait SymbolLookup {
    /// Find all symbols with the given simple name.
    fn by_name(&self, name: &str) -> &[SymbolInfo];

    /// Find a symbol by exact qualified name.
    fn by_qualified_name(&self, qname: &str) -> Option<&SymbolInfo>;

    /// Find all symbols whose qualified name starts with the given prefix + ".".
    fn in_namespace(&self, namespace: &str) -> Vec<&SymbolInfo>;

    /// Find all symbols defined in a specific file.
    fn in_file(&self, file_path: &str) -> &[SymbolInfo];

    /// Get the annotated type name for a property/field symbol.
    /// e.g., "AlbumService.db" → Some("DatabaseRepository")
    fn field_type_name(&self, property_qname: &str) -> Option<&str>;

    /// Get the annotated return type for a method/function symbol.
    /// e.g., "UserRepo.findOne" → Some("User")
    fn return_type_name(&self, method_qname: &str) -> Option<&str>;

    /// Get the generic type arguments for a field's type annotation.
    /// e.g., "UserService.repo" → Some(["User"]) for `repo: Repository<User>`
    fn field_type_args(&self, property_qname: &str) -> Option<&[String]>;

    /// Get the generic type parameter names for a type declaration.
    /// e.g., "Repository" → Some(["T"]) for `interface Repository<T>`
    fn generic_params(&self, type_name: &str) -> Option<&[String]>;
}

// ---------------------------------------------------------------------------
// TypeInfo — unified per-symbol type metadata
// ---------------------------------------------------------------------------

/// All type metadata for a single symbol, stored in a single map keyed by
/// the symbol's qualified name (or simple name for generic_params).
#[derive(Debug, Default, Clone)]
pub struct TypeInfo {
    /// Field/property type (e.g., "UserRepository").
    pub field_type: Option<String>,
    /// Generic type arguments (e.g., ["User"] for `Repository<User>`).
    pub type_args: Vec<String>,
    /// Method return type (e.g., "User").
    pub return_type: Option<String>,
    /// Generic parameter names for type declarations (e.g., ["T"] for `interface Repository<T>`).
    pub generic_params: Vec<String>,
}

// ---------------------------------------------------------------------------
// SymbolIndex — concrete implementation of SymbolLookup
// ---------------------------------------------------------------------------

/// In-memory index of all symbols, built once from parsed data.
pub struct SymbolIndex {
    by_name: FxHashMap<String, Vec<SymbolInfo>>,
    by_qname: FxHashMap<String, SymbolInfo>,
    by_file: FxHashMap<String, Vec<SymbolInfo>>,
    /// Sorted by qualified_name for O(log N) prefix (in_namespace) queries.
    sorted_qnames: Vec<(String, SymbolInfo)>,
    /// Unified per-symbol type metadata (replaces 4 separate maps).
    type_info: FxHashMap<String, TypeInfo>,
    empty: Vec<SymbolInfo>,
}

impl SymbolIndex {
    /// Build the index from parsed files and the symbol-to-ID mapping.
    pub fn build(
        parsed: &[ParsedFile],
        symbol_id_map: &HashMap<(String, String), i64>,
    ) -> Self {
        let mut by_name: FxHashMap<String, Vec<SymbolInfo>> = FxHashMap::default();
        let mut by_qname: FxHashMap<String, SymbolInfo> = FxHashMap::default();
        let mut by_file: FxHashMap<String, Vec<SymbolInfo>> = FxHashMap::default();

        for pf in parsed {
            for sym in &pf.symbols {
                let Some(&id) = symbol_id_map.get(&(pf.path.clone(), sym.qualified_name.clone()))
                else {
                    continue;
                };

                let info = SymbolInfo {
                    id,
                    name: sym.name.clone(),
                    qualified_name: sym.qualified_name.clone(),
                    kind: sym.kind.as_str().to_string(),
                    visibility: sym.visibility.as_ref().map(|v| format!("{v:?}").to_lowercase()),
                    file_path: pf.path.clone(),
                    scope_path: sym.scope_path.clone(),
                };

                // Simple name index
                let simple = sym.name.clone();
                by_name.entry(simple).or_default().push(info.clone());

                // Qualified name index (first wins for duplicates)
                by_qname
                    .entry(sym.qualified_name.clone())
                    .or_insert_with(|| info.clone());

                // File index
                by_file
                    .entry(pf.path.clone())
                    .or_default()
                    .push(info);
            }
        }

        // Build field_type, field_type_args, return_type, and generic_params maps.
        let mut field_type: FxHashMap<String, String> = FxHashMap::default();
        let mut field_type_args: FxHashMap<String, Vec<String>> = FxHashMap::default();
        let mut return_type: FxHashMap<String, String> = FxHashMap::default();
        let mut generic_params: FxHashMap<String, Vec<String>> = FxHashMap::default();

        for pf in parsed {
            for (sym_idx, sym) in pf.symbols.iter().enumerate() {
                // Collect TypeRef refs from this symbol (no module = not an import).
                let type_refs: Vec<&str> = pf
                    .refs
                    .iter()
                    .filter(|r| {
                        r.source_symbol_index == sym_idx
                            && r.kind == EdgeKind::TypeRef
                            && r.module.is_none()
                    })
                    .map(|r| r.target_name.as_str())
                    .collect();

                if type_refs.is_empty() {
                    continue;
                }

                match sym.kind {
                    // Properties/fields: first TypeRef is the field type.
                    // Subsequent TypeRefs from the same symbol may be generic type args.
                    SymbolKind::Property | SymbolKind::Field => {
                        field_type
                            .insert(sym.qualified_name.clone(), type_refs[0].to_string());
                        // If there are additional TypeRefs, they're generic type arguments.
                        // e.g., `repo: Repository<User>` emits ["Repository", "User"]
                        if type_refs.len() > 1 {
                            field_type_args.insert(
                                sym.qualified_name.clone(),
                                type_refs[1..].iter().map(|s| s.to_string()).collect(),
                            );
                        }
                    }
                    // Methods/functions: last TypeRef is likely the return type.
                    SymbolKind::Method
                    | SymbolKind::Function
                    | SymbolKind::Constructor => {
                        if let Some(&last) = type_refs.last() {
                            return_type
                                .insert(sym.qualified_name.clone(), last.to_string());
                        }
                    }
                    // Classes/interfaces/structs with TypeRef to themselves may have
                    // generic type parameters in the signature.
                    _ => {}
                }
            }

            // Build generic_params: for class/interface/struct symbols that have
            // type_parameters in their signature (e.g., `interface Repository<T>`).
            // We detect this by looking at the symbol's signature text.
            for sym in &pf.symbols {
                if !matches!(
                    sym.kind,
                    SymbolKind::Class
                        | SymbolKind::Interface
                        | SymbolKind::Struct
                        | SymbolKind::TypeAlias
                ) {
                    continue;
                }
                if let Some(sig) = &sym.signature {
                    // Parse generic params from signature: "interface Repository<T>" → ["T"]
                    // "class Map<K, V>" → ["K", "V"]
                    if let Some(start) = sig.find('<') {
                        if let Some(end) = sig.find('>') {
                            let params_str = &sig[start + 1..end];
                            let params: Vec<String> = params_str
                                .split(',')
                                .map(|s| {
                                    s.trim()
                                        .split_whitespace()
                                        .next()
                                        .unwrap_or("")
                                        .to_string()
                                })
                                .filter(|s| !s.is_empty())
                                .collect();
                            if !params.is_empty() {
                                generic_params.insert(sym.name.clone(), params.clone());
                                generic_params
                                    .insert(sym.qualified_name.clone(), params);
                            }
                        }
                    }
                }
            }
        }

        // Merge the four local maps into the unified type_info map.
        let mut type_info: FxHashMap<String, TypeInfo> = FxHashMap::default();
        for (qname, ft) in field_type {
            type_info.entry(qname).or_default().field_type = Some(ft);
        }
        for (qname, args) in field_type_args {
            type_info.entry(qname).or_default().type_args = args;
        }
        for (qname, rt) in return_type {
            type_info.entry(qname).or_default().return_type = Some(rt);
        }
        for (name_or_qname, params) in generic_params {
            type_info.entry(name_or_qname).or_default().generic_params = params;
        }

        // Variable type inference pass: for Variable symbols without an explicit
        // type annotation, try to infer the type from chain-bearing TypeRef refs.
        // These are emitted by the extractor for `const x = this.repo.findOne()`.
        // We resolve the chain to get the method's return type.
        for pf in parsed {
            for (sym_idx, sym) in pf.symbols.iter().enumerate() {
                if sym.kind != SymbolKind::Variable {
                    continue;
                }
                // Skip if already has an explicit type.
                if type_info
                    .get(&sym.qualified_name)
                    .and_then(|ti| ti.field_type.as_ref())
                    .is_some()
                {
                    continue;
                }
                // Find a chain-bearing TypeRef from this variable.
                for r in &pf.refs {
                    if r.source_symbol_index != sym_idx
                        || r.kind != EdgeKind::TypeRef
                        || r.chain.is_none()
                    {
                        continue;
                    }
                    let chain = r.chain.as_ref().unwrap();
                    // Walk the chain to infer the type.
                    if let Some(inferred) =
                        infer_type_from_chain(chain, &sym.scope_path, &type_info, &by_name, &by_qname)
                    {
                        type_info
                            .entry(sym.qualified_name.clone())
                            .or_default()
                            .field_type = Some(inferred);
                        break;
                    }
                }
            }
        }

        // Build sorted_qnames for O(log N) prefix (in_namespace) queries.
        let mut sorted_qnames: Vec<(String, SymbolInfo)> = by_qname
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        sorted_qnames.sort_by(|a, b| a.0.cmp(&b.0));

        Self {
            by_name,
            by_qname,
            by_file,
            sorted_qnames,
            type_info,
            empty: Vec::new(),
        }
    }
}

/// Lightweight chain resolution for variable type inference during index building.
/// Uses the already-built type_info map (not the full SymbolLookup trait).
fn infer_type_from_chain(
    chain: &crate::types::MemberChain,
    scope_path: &Option<String>,
    type_info: &FxHashMap<String, TypeInfo>,
    by_name: &FxHashMap<String, Vec<SymbolInfo>>,
    by_qname: &FxHashMap<String, SymbolInfo>,
) -> Option<String> {
    use crate::types::SegmentKind;

    let segments = &chain.segments;
    if segments.is_empty() {
        return None;
    }

    // Build a minimal scope chain from scope_path.
    let scopes: Vec<String> = if let Some(sp) = scope_path {
        let mut scope_chain = Vec::new();
        let mut current = sp.clone();
        scope_chain.push(current.clone());
        while let Some(dot) = current.rfind('.') {
            current.truncate(dot);
            scope_chain.push(current.clone());
        }
        scope_chain
    } else {
        Vec::new()
    };

    // Phase 1: Root type.
    let root_type = match segments[0].kind {
        SegmentKind::SelfRef => {
            // Find enclosing class from scope.
            scopes
                .iter()
                .find_map(|s| {
                    by_qname.get(s).and_then(|sym| {
                        if matches!(sym.kind.as_str(), "class" | "struct" | "interface") {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                })
                .or_else(|| scopes.last().cloned())
        }
        SegmentKind::Identifier => {
            let name = &segments[0].name;
            scopes
                .iter()
                .find_map(|scope| {
                    let qname = format!("{scope}.{name}");
                    type_info.get(&qname).and_then(|ti| ti.field_type.clone())
                })
                .or_else(|| segments[0].declared_type.clone())
        }
        _ => None,
    }?;

    let mut current_type = root_type;

    // Build a TypeEnvironment for generic substitution.
    let mut env = TypeEnvironment::new();

    // Look up initial generic args for the root field.
    for scope in &scopes {
        let key = format!("{scope}.{}", segments[0].name);
        if let Some(ti) = type_info.get(&key) {
            if !ti.type_args.is_empty() {
                // Enter the generic context for the root type.
                let args = ti.type_args.clone();
                env.enter_generic_context(&current_type, &args, |name| {
                    // Look up by qualified name first, then by simple name.
                    type_info
                        .get(name)
                        .map(|ti| ti.generic_params.clone())
                        .filter(|p| !p.is_empty())
                        .or_else(|| {
                            let simple = name.rsplit('.').next().unwrap_or(name);
                            type_info
                                .get(simple)
                                .map(|ti| ti.generic_params.clone())
                                .filter(|p| !p.is_empty())
                        })
                });
                break;
            }
        }
    }

    // Phase 2: Walk remaining segments.
    for seg in &segments[1..] {
        let member_qname = format!("{current_type}.{}", seg.name);

        if let Some(ti) = type_info.get(&member_qname) {
            if let Some(ft) = &ti.field_type {
                let new_args = ti.type_args.clone();
                let resolved_type = env.resolve(ft);
                env.push_scope();
                if !new_args.is_empty() {
                    env.enter_generic_context(&resolved_type, &new_args, |name| {
                        type_info
                            .get(name)
                            .map(|ti| ti.generic_params.clone())
                            .filter(|p| !p.is_empty())
                            .or_else(|| {
                                let simple = name.rsplit('.').next().unwrap_or(name);
                                type_info
                                    .get(simple)
                                    .map(|ti| ti.generic_params.clone())
                                    .filter(|p| !p.is_empty())
                            })
                    });
                }
                current_type = resolved_type;
                continue;
            }
            if let Some(raw_return) = &ti.return_type {
                // Use TypeEnvironment for generic substitution (T → User, E → Error, etc).
                let resolved = env.resolve(raw_return);
                env.push_scope();
                current_type = resolved;
                continue;
            }
        }

        // Can't follow further.
        let _ = by_name; // retained for future use
        return None;
    }

    // The final current_type is the inferred type of the chain.
    Some(current_type)
}

impl SymbolLookup for SymbolIndex {
    fn by_name(&self, name: &str) -> &[SymbolInfo] {
        self.by_name.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }

    fn by_qualified_name(&self, qname: &str) -> Option<&SymbolInfo> {
        self.by_qname.get(qname)
    }

    /// O(log N) prefix search using the sorted Vec.
    fn in_namespace(&self, namespace: &str) -> Vec<&SymbolInfo> {
        let prefix = format!("{namespace}.");
        let start = self
            .sorted_qnames
            .partition_point(|(k, _)| k.as_str() < prefix.as_str());
        let end = self.sorted_qnames[start..]
            .partition_point(|(k, _)| k.starts_with(&prefix));
        self.sorted_qnames[start..start + end]
            .iter()
            .map(|(_, info)| info)
            .collect()
    }

    fn in_file(&self, file_path: &str) -> &[SymbolInfo] {
        self.by_file
            .get(file_path)
            .map(|v| v.as_slice())
            .unwrap_or(&self.empty)
    }

    fn field_type_name(&self, property_qname: &str) -> Option<&str> {
        self.type_info
            .get(property_qname)
            .and_then(|ti| ti.field_type.as_deref())
    }

    fn return_type_name(&self, method_qname: &str) -> Option<&str> {
        self.type_info
            .get(method_qname)
            .and_then(|ti| ti.return_type.as_deref())
    }

    fn field_type_args(&self, property_qname: &str) -> Option<&[String]> {
        self.type_info.get(property_qname).and_then(|ti| {
            if ti.type_args.is_empty() {
                None
            } else {
                Some(ti.type_args.as_slice())
            }
        })
    }

    fn generic_params(&self, type_name: &str) -> Option<&[String]> {
        self.type_info.get(type_name).and_then(|ti| {
            if ti.generic_params.is_empty() {
                None
            } else {
                Some(ti.generic_params.as_slice())
            }
        })
    }
}

// ---------------------------------------------------------------------------
// LanguageResolver trait
// ---------------------------------------------------------------------------

/// Per-language resolution rules. Each language implements this trait
/// in a separate file under `resolve/rules/`.
pub trait LanguageResolver: Send + Sync {
    /// The language identifier(s) this resolver handles.
    /// Must match the language strings from file detection.
    fn language_ids(&self) -> &[&str];

    /// Build the file context for a parsed file.
    /// `project_ctx` provides global usings and external prefix data.
    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> FileContext;

    /// Attempt to resolve a reference using language-specific scope rules.
    ///
    /// Returns `Some(Resolution)` if deterministically resolved.
    /// Returns `None` to fall back to the heuristic resolver.
    fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution>;

    /// Check whether a target symbol is visible from the reference site.
    /// Default: always visible (no filtering).
    fn is_visible(
        &self,
        _file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        _target: &SymbolInfo,
    ) -> bool {
        true
    }

    /// When a reference can't be resolved (not in the index), try to infer
    /// which external namespace/package it likely comes from based on the
    /// file's import statements.
    ///
    /// Returns `Some("Microsoft.EntityFrameworkCore")` if the target probably
    /// comes from that namespace. Returns `None` if no guess can be made.
    ///
    /// Default: no inference.
    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        None
    }
}

// ---------------------------------------------------------------------------
// ResolutionEngine
// ---------------------------------------------------------------------------

/// The engine that dispatches resolution to language-specific resolvers.
pub struct ResolutionEngine {
    resolvers: FxHashMap<String, Arc<dyn LanguageResolver>>,
}

impl ResolutionEngine {
    /// Create a new engine with the default set of language resolvers.
    pub fn new() -> Self {
        let mut engine = Self {
            resolvers: FxHashMap::default(),
        };
        // Source resolvers from the language plugin system.
        // During migration, the old rules::default_resolvers() is also available
        // but this is the primary source now.
        for resolver in crate::languages::default_resolvers() {
            for &lang_id in resolver.language_ids() {
                engine
                    .resolvers
                    .insert(lang_id.to_string(), Arc::clone(&resolver));
            }
        }
        // Also load any remaining resolvers from the old rules module
        // that haven't been migrated yet (temporary compatibility).
        for resolver in super::rules::default_resolvers() {
            for &lang_id in resolver.language_ids() {
                // Don't overwrite — languages:: takes priority
                engine
                    .resolvers
                    .entry(lang_id.to_string())
                    .or_insert_with(|| Arc::clone(&resolver));
            }
        }
        engine
    }

    /// Get the resolver for a language, if one is registered.
    pub fn resolver_for(&self, language: &str) -> Option<&dyn LanguageResolver> {
        self.resolvers.get(language).map(|r| r.as_ref())
    }
}

// ---------------------------------------------------------------------------
// Chain-aware external inference (shared by all resolvers)
// ---------------------------------------------------------------------------

/// If a ref has a MemberChain, walk it to see if we can determine a type
/// that isn't in our index — meaning the chain leads to an external type.
///
/// For `this.repo.findMany()` where repo has type `PrismaClient`:
/// 1. `this` → `UserService` (from scope chain)
/// 2. `repo` → field_type = `PrismaClient`
/// 3. `PrismaClient` not in index → return Some("PrismaClient")
///
/// This classifies the entire chain call as external with the unresolved
/// type name as the namespace.
pub fn infer_external_from_chain(
    chain: &crate::types::MemberChain,
    scope_chain: &[String],
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    use crate::types::SegmentKind;

    let segments = &chain.segments;
    if segments.len() < 2 {
        return None;
    }

    // Phase 1: Determine root type.
    let root_type = match segments[0].kind {
        SegmentKind::SelfRef => {
            // Find enclosing class.
            let mut found = None;
            for scope in scope_chain {
                if let Some(sym) = lookup.by_qualified_name(scope) {
                    if matches!(sym.kind.as_str(), "class" | "struct" | "interface") {
                        found = Some(scope.clone());
                        break;
                    }
                }
            }
            found.or_else(|| scope_chain.last().cloned())
        }
        SegmentKind::Identifier => {
            let name = &segments[0].name;
            // Field on enclosing class?
            let mut found = None;
            for scope in scope_chain {
                let field_qname = format!("{scope}.{name}");
                if let Some(type_name) = lookup.field_type_name(&field_qname) {
                    found = Some(type_name.to_string());
                    break;
                }
            }
            found.or_else(|| segments[0].declared_type.clone())
        }
        _ => None,
    };

    // Phase 2: Walk the chain checking if the current type is external.
    // If no root type was determined, check if the root identifier itself
    // is external (not in the index, or a variable with no resolvable type).
    let mut current_type = match root_type {
        Some(t) => t,
        None => {
            if segments[0].kind == SegmentKind::Identifier {
                let name = &segments[0].name;
                let symbols = lookup.by_name(name);
                let has_type_or_class = symbols.iter().any(|s| {
                    matches!(
                        s.kind.as_str(),
                        "class" | "struct" | "interface" | "enum"
                            | "type_alias" | "function" | "method"
                    )
                });
                if has_type_or_class {
                    // It's a known type/function — can't determine as external.
                    return None;
                }
                // Either not in index at all, or only a variable/namespace
                // with no resolvable type → treat as external.
                return Some(name.clone());
            }
            return None;
        }
    };

    for seg in &segments[1..] {
        // If the current type isn't in the index → it's external.
        let type_in_index = lookup.by_qualified_name(&current_type).is_some()
            || lookup.by_name(&current_type).iter().any(|s| {
                matches!(
                    s.kind.as_str(),
                    "class" | "struct" | "interface" | "enum" | "type_alias"
                        | "trait" | "module" | "namespace"
                )
            });

        if !type_in_index {
            return Some(current_type);
        }

        // Try to follow to the next type.
        let member_qname = format!("{current_type}.{}", seg.name);
        if let Some(next) = lookup.field_type_name(&member_qname) {
            current_type = next.to_string();
            continue;
        }
        if let Some(next) = lookup.return_type_name(&member_qname) {
            current_type = next.to_string();
            continue;
        }

        // Can't follow further — the member exists on a known type but
        // we don't know its return type. Not enough info to classify.
        break;
    }

    None
}

// ---------------------------------------------------------------------------
// Helpers for building RefContext
// ---------------------------------------------------------------------------

/// Build the scope chain from a symbol's scope_path.
///
/// scope_path = "A.B.C" → ["A.B.C", "A.B", "A"]
pub fn build_scope_chain(scope_path: Option<&str>) -> Vec<String> {
    let Some(path) = scope_path else {
        return Vec::new();
    };
    if path.is_empty() {
        return Vec::new();
    }

    let mut chain = Vec::new();
    let mut current = path.to_string();
    chain.push(current.clone());

    while let Some(dot_pos) = current.rfind('.') {
        current.truncate(dot_pos);
        chain.push(current.clone());
    }

    chain
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scope_chain_from_path() {
        let chain = build_scope_chain(Some("A.B.C"));
        assert_eq!(chain, vec!["A.B.C", "A.B", "A"]);
    }

    #[test]
    fn test_scope_chain_single() {
        let chain = build_scope_chain(Some("Namespace"));
        assert_eq!(chain, vec!["Namespace"]);
    }

    #[test]
    fn test_scope_chain_empty() {
        assert!(build_scope_chain(None).is_empty());
        assert!(build_scope_chain(Some("")).is_empty());
    }

    #[test]
    fn test_symbol_index_by_name() {
        // Build a minimal ParsedFile + symbol_id_map
        let pf = ParsedFile {
            path: "src/foo.cs".to_string(),
            language: "csharp".to_string(),
            content_hash: String::new(),
            size: 0,
            line_count: 0,
            content: None,
            has_errors: false,
            symbols: vec![
                ExtractedSymbol {
                    name: "Foo".to_string(),
                    qualified_name: "NS.Foo".to_string(),
                    kind: SymbolKind::Class,
                    visibility: Some(Visibility::Public),
                    start_line: 1,
                    end_line: 10,
                    start_col: 0,
                    end_col: 0,
                    signature: None,
                    doc_comment: None,
                    scope_path: Some("NS".to_string()),
                    parent_index: None,
                },
            ],
            refs: vec![],
            routes: vec![],
            db_sets: vec![],
        };

        let mut id_map = HashMap::new();
        id_map.insert(("src/foo.cs".to_string(), "NS.Foo".to_string()), 42);

        let index = SymbolIndex::build(&[pf], &id_map);

        // by_name
        let results = index.by_name("Foo");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, 42);
        assert_eq!(results[0].qualified_name, "NS.Foo");

        // by_qualified_name
        let result = index.by_qualified_name("NS.Foo");
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, 42);

        // in_namespace
        let ns_results = index.in_namespace("NS");
        assert_eq!(ns_results.len(), 1);

        // in_file
        let file_results = index.in_file("src/foo.cs");
        assert_eq!(file_results.len(), 1);

        // missing
        assert!(index.by_name("Bar").is_empty());
        assert!(index.by_qualified_name("NS.Bar").is_none());
    }

    fn make_class_sym(name: &str, qname: &str) -> ExtractedSymbol {
        ExtractedSymbol {
            name: name.to_string(),
            qualified_name: qname.to_string(),
            kind: SymbolKind::Class,
            visibility: None,
            start_line: 1,
            end_line: 2,
            start_col: 0,
            end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path: None,
            parent_index: None,
        }
    }

    fn make_pf(path: &str, syms: Vec<ExtractedSymbol>) -> ParsedFile {
        ParsedFile {
            path: path.to_string(),
            language: "csharp".to_string(),
            content_hash: String::new(),
            size: 0,
            line_count: 0,
            content: None,
            has_errors: false,
            symbols: syms,
            refs: vec![],
            routes: vec![],
            db_sets: vec![],
        }
    }

    #[test]
    fn test_in_namespace_sorted_multiple() {
        let pf = make_pf(
            "src/a.cs",
            vec![
                make_class_sym("Foo", "NS.Foo"),
                make_class_sym("Bar", "NS.Bar"),
                make_class_sym("Baz", "Other.Baz"),
            ],
        );

        let mut id_map = HashMap::new();
        id_map.insert(("src/a.cs".to_string(), "NS.Foo".to_string()), 1);
        id_map.insert(("src/a.cs".to_string(), "NS.Bar".to_string()), 2);
        id_map.insert(("src/a.cs".to_string(), "Other.Baz".to_string()), 3);

        let index = SymbolIndex::build(&[pf], &id_map);

        let ns_results = index.in_namespace("NS");
        assert_eq!(ns_results.len(), 2);
        let mut ids: Vec<i64> = ns_results.iter().map(|s| s.id).collect();
        ids.sort_unstable();
        assert_eq!(ids, vec![1, 2]);

        let other_results = index.in_namespace("Other");
        assert_eq!(other_results.len(), 1);
        assert_eq!(other_results[0].id, 3);
    }

    #[test]
    fn test_in_namespace_no_prefix_bleed() {
        // "NS" must not match "NSX.Thing"
        let pf = make_pf(
            "src/a.cs",
            vec![
                make_class_sym("Foo", "NS.Foo"),
                make_class_sym("Thing", "NSX.Thing"),
            ],
        );

        let mut id_map = HashMap::new();
        id_map.insert(("src/a.cs".to_string(), "NS.Foo".to_string()), 1);
        id_map.insert(("src/a.cs".to_string(), "NSX.Thing".to_string()), 2);

        let index = SymbolIndex::build(&[pf], &id_map);

        let ns_results = index.in_namespace("NS");
        assert_eq!(ns_results.len(), 1);
        assert_eq!(ns_results[0].qualified_name, "NS.Foo");
    }

    #[test]
    fn test_in_namespace_empty() {
        let pf = make_pf("src/a.cs", vec![make_class_sym("Foo", "NS.Foo")]);
        let mut id_map = HashMap::new();
        id_map.insert(("src/a.cs".to_string(), "NS.Foo".to_string()), 1);
        let index = SymbolIndex::build(&[pf], &id_map);

        assert!(index.in_namespace("Missing").is_empty());
        assert!(index.in_namespace("N").is_empty()); // "N" is a prefix of "NS" but not "NS."
    }
}
