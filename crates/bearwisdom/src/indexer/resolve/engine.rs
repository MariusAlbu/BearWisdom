// =============================================================================
// indexer/resolve/engine.rs — Resolution engine with per-language rule plugins
//
// The engine dispatches reference resolution to language-specific resolvers
// that apply deterministic scope rules (1.0 confidence). When no language
// resolver can resolve a reference, it falls back to the heuristic resolver.
// =============================================================================

use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
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
}

// ---------------------------------------------------------------------------
// SymbolIndex — concrete implementation of SymbolLookup
// ---------------------------------------------------------------------------

/// In-memory index of all symbols, built once from parsed data.
pub struct SymbolIndex {
    by_name: HashMap<String, Vec<SymbolInfo>>,
    by_qname: HashMap<String, SymbolInfo>,
    by_file: HashMap<String, Vec<SymbolInfo>>,
    /// Maps property qualified_name → type name from TypeRef annotations.
    /// e.g., "AlbumService.db" → "DatabaseRepository"
    field_type: HashMap<String, String>,
    /// Maps method/function qualified_name → return type name from annotations.
    /// e.g., "UserRepo.findOne" → "User"
    return_type: HashMap<String, String>,
    empty: Vec<SymbolInfo>,
}

impl SymbolIndex {
    /// Build the index from parsed files and the symbol-to-ID mapping.
    pub fn build(
        parsed: &[ParsedFile],
        symbol_id_map: &HashMap<(String, String), i64>,
    ) -> Self {
        let mut by_name: HashMap<String, Vec<SymbolInfo>> = HashMap::new();
        let mut by_qname: HashMap<String, SymbolInfo> = HashMap::new();
        let mut by_file: HashMap<String, Vec<SymbolInfo>> = HashMap::new();

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

        // Build field_type and return_type maps from TypeRef annotations.
        let mut field_type: HashMap<String, String> = HashMap::new();
        let mut return_type: HashMap<String, String> = HashMap::new();

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
                    SymbolKind::Property | SymbolKind::Field => {
                        field_type
                            .insert(sym.qualified_name.clone(), type_refs[0].to_string());
                    }
                    // Methods/functions: last TypeRef is likely the return type
                    // (parameters come first in source order, return type annotation is last).
                    SymbolKind::Method
                    | SymbolKind::Function
                    | SymbolKind::Constructor => {
                        if let Some(&last) = type_refs.last() {
                            return_type
                                .insert(sym.qualified_name.clone(), last.to_string());
                        }
                    }
                    _ => {}
                }
            }
        }

        Self {
            by_name,
            by_qname,
            by_file,
            field_type,
            return_type,
            empty: Vec::new(),
        }
    }
}

impl SymbolLookup for SymbolIndex {
    fn by_name(&self, name: &str) -> &[SymbolInfo] {
        self.by_name.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }

    fn by_qualified_name(&self, qname: &str) -> Option<&SymbolInfo> {
        self.by_qname.get(qname)
    }

    fn in_namespace(&self, namespace: &str) -> Vec<&SymbolInfo> {
        let prefix = format!("{namespace}.");
        self.by_qname
            .values()
            .filter(|s| s.qualified_name.starts_with(&prefix))
            .collect()
    }

    fn in_file(&self, file_path: &str) -> &[SymbolInfo] {
        self.by_file
            .get(file_path)
            .map(|v| v.as_slice())
            .unwrap_or(&self.empty)
    }

    fn field_type_name(&self, property_qname: &str) -> Option<&str> {
        self.field_type.get(property_qname).map(|s| s.as_str())
    }

    fn return_type_name(&self, method_qname: &str) -> Option<&str> {
        self.return_type.get(method_qname).map(|s| s.as_str())
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
    resolvers: HashMap<String, Arc<dyn LanguageResolver>>,
}

impl ResolutionEngine {
    /// Create a new engine with the default set of language resolvers.
    pub fn new() -> Self {
        let mut engine = Self {
            resolvers: HashMap::new(),
        };
        for resolver in super::rules::default_resolvers() {
            for &lang_id in resolver.language_ids() {
                engine
                    .resolvers
                    .insert(lang_id.to_string(), Arc::clone(&resolver));
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
}
