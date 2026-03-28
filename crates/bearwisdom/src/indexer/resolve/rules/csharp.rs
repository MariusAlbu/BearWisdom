// =============================================================================
// indexer/resolve/rules/csharp.rs — C# resolution rules
//
// Scope rules for C# (all versions through C# 13):
//
//   1. Scope chain walk: innermost scope → outermost, try {scope}.{target}
//   2. Same-namespace: types in the same namespace are visible without `using`
//   3. Using directives: `using Namespace;` makes all public types visible
//   4. Fully qualified: dotted names resolve directly
//   5. Visibility: public/internal/protected/private enforcement
//
// Adding new C# features:
//   - New syntax that introduces scope (e.g., file-scoped namespaces) →
//     update the extractor in parser/extractors/csharp.rs to emit the
//     correct scope_path, then this resolver handles it automatically.
//   - New import forms (e.g., global using) → add to build_file_context.
// =============================================================================

use super::super::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::types::{EdgeKind, ParsedFile};

/// C# language resolver.
pub struct CSharpResolver;

impl LanguageResolver for CSharpResolver {
    fn language_ids(&self) -> &[&str] {
        &["csharp"]
    }

    fn build_file_context(&self, file: &ParsedFile) -> FileContext {
        let mut imports = Vec::new();
        let mut file_namespace = None;

        // Extract namespace from the first Namespace symbol.
        for sym in &file.symbols {
            if sym.kind == crate::types::SymbolKind::Namespace {
                file_namespace = Some(sym.qualified_name.clone());
                break;
            }
        }

        // Extract using directives from refs with EdgeKind::Imports.
        for r in &file.refs {
            if r.kind == EdgeKind::Imports {
                let module = r.module.as_deref().unwrap_or(&r.target_name);
                imports.push(ImportEntry {
                    imported_name: r.target_name.clone(),
                    module_path: Some(module.to_string()),
                    alias: None,
                    // C# `using Namespace;` is a wildcard import — all public types
                    // in that namespace become visible.
                    is_wildcard: module.contains('.'),
                });
            }
        }

        FileContext {
            file_path: file.path.clone(),
            language: "csharp".to_string(),
            imports,
            file_namespace,
        }
    }

    fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        // Skip import refs themselves — they're not symbol references.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Step 1: Scope chain walk (innermost → outermost).
        // e.g., scope_chain = ["NS.Cls.Method", "NS.Cls", "NS"]
        // Try "NS.Cls.Method.Target", "NS.Cls.Target", "NS.Target"
        for scope in &ref_ctx.scope_chain {
            let candidate = format!("{scope}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "csharp_scope_chain",
                    });
                }
            }
        }

        // Step 2: Same-namespace resolution.
        // In C#, types in the same namespace are visible without a `using` directive.
        if let Some(ns) = &file_ctx.file_namespace {
            let candidate = format!("{ns}.{target}");
            if let Some(sym) = lookup.by_qualified_name(&candidate) {
                if self.is_visible(file_ctx, ref_ctx, sym)
                    && kind_compatible(edge_kind, &sym.kind)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "csharp_same_namespace",
                    });
                }
            }
        }

        // Step 3: Using directive resolution.
        // `using eShop.Catalog.API.Model;` → try "eShop.Catalog.API.Model.{target}"
        for import in &file_ctx.imports {
            if import.is_wildcard {
                if let Some(module) = &import.module_path {
                    let candidate = format!("{module}.{target}");
                    if let Some(sym) = lookup.by_qualified_name(&candidate) {
                        if self.is_visible(file_ctx, ref_ctx, sym)
                            && kind_compatible(edge_kind, &sym.kind)
                        {
                            return Some(Resolution {
                                target_symbol_id: sym.id,
                                confidence: 1.0,
                                strategy: "csharp_using_directive",
                            });
                        }
                    }
                }
            }
        }

        // Step 4: Fully qualified name (target contains dots).
        if target.contains('.') {
            if let Some(sym) = lookup.by_qualified_name(target) {
                if kind_compatible(edge_kind, &sym.kind) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "csharp_qualified_name",
                    });
                }
            }
        }

        // Step 5: Base type member resolution.
        // If the source symbol has a scope_path pointing to a class, and that class
        // inherits from a base, try resolving in the base class scope.
        // (Handled implicitly by scope_chain if extractor builds qualified names correctly.)

        // Could not resolve deterministically — fall back to heuristic.
        None
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
    ) -> Option<String> {
        // Skip import refs — they ARE the namespace declarations.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            return None;
        }

        // If the file has using directives, the unresolved ref likely comes
        // from one of them. Return the most specific namespace that could
        // contain this symbol.
        //
        // Heuristic: prefer external-looking namespaces (Microsoft.*, System.*,
        // third-party) over project namespaces that we should have indexed.
        let mut best: Option<&str> = None;

        for import in &file_ctx.imports {
            if !import.is_wildcard {
                continue;
            }
            let ns = import.module_path.as_deref().unwrap_or("");
            if ns.is_empty() {
                continue;
            }

            // Prefer framework/library namespaces
            let is_external = ns.starts_with("System")
                || ns.starts_with("Microsoft")
                || ns.starts_with("Newtonsoft")
                || ns.starts_with("AutoMapper")
                || ns.starts_with("MediatR")
                || ns.starts_with("FluentValidation")
                || ns.starts_with("Serilog")
                || ns.starts_with("Npgsql")
                || ns.starts_with("Dapper")
                || ns.starts_with("Polly")
                || ns.starts_with("Grpc")
                || ns.starts_with("Google")
                || ns.starts_with("Amazon")
                || ns.starts_with("Azure");

            if is_external {
                // Pick the longest matching external namespace (most specific)
                if best.is_none() || ns.len() > best.unwrap().len() {
                    best = Some(ns);
                }
            }
        }

        // If no external namespace matched, use the first available using
        if best.is_none() {
            for import in &file_ctx.imports {
                if import.is_wildcard {
                    if let Some(ns) = &import.module_path {
                        if !ns.is_empty() {
                            best = Some(ns.as_str());
                            break;
                        }
                    }
                }
            }
        }

        best.map(|s| s.to_string())
    }

    fn is_visible(
        &self,
        file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        target: &SymbolInfo,
    ) -> bool {
        let vis = target.visibility.as_deref().unwrap_or("public");
        match vis {
            "public" => true,
            "internal" => {
                // Approximate: visible if in the same project (same top-level directory).
                // For a proper check we'd need assembly information.
                true
            }
            "protected" => {
                // Approximate: visible if in the same class hierarchy.
                // Full check would require walking the inheritance chain.
                true
            }
            "private" => {
                // Private: only visible within the same file.
                target.file_path == file_ctx.file_path
            }
            _ => true,
        }
    }
}

/// Check that the edge kind is compatible with the symbol kind.
fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "property"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class" | "struct"),
        EdgeKind::Implements => matches!(sym_kind, "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "struct" | "interface" | "enum" | "type_alias" | "namespace" | "delegate"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "struct"),
        _ => true,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::resolve::engine::{SymbolIndex, build_scope_chain};
    use crate::types::*;
    use std::collections::HashMap;

    fn make_symbol(name: &str, qname: &str, kind: SymbolKind, vis: Visibility, scope: Option<&str>) -> ExtractedSymbol {
        ExtractedSymbol {
            name: name.to_string(),
            qualified_name: qname.to_string(),
            kind,
            visibility: Some(vis),
            start_line: 1,
            end_line: 10,
            start_col: 0,
            end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path: scope.map(|s| s.to_string()),
            parent_index: None,
        }
    }

    fn make_ref(source_idx: usize, target: &str, kind: EdgeKind, line: u32) -> ExtractedRef {
        ExtractedRef {
            source_symbol_index: source_idx,
            target_name: target.to_string(),
            kind,
            line,
            module: None,
        }
    }

    fn make_file(path: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
        ParsedFile {
            path: path.to_string(),
            language: "csharp".to_string(),
            content_hash: String::new(),
            size: 0,
            line_count: 0,
            content: None,
            has_errors: false,
            symbols,
            refs,
            routes: vec![],
            db_sets: vec![],
        }
    }

    /// Build index from files, returning (index, id_map). Files are borrowed.
    fn build_test_env(files: &[&ParsedFile]) -> (SymbolIndex, HashMap<(String, String), i64>) {
        let mut id_map = HashMap::new();
        let mut next_id = 1i64;
        for pf in files {
            for sym in &pf.symbols {
                id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next_id);
                next_id += 1;
            }
        }
        let file_refs: Vec<&ParsedFile> = files.to_vec();
        let owned: Vec<ParsedFile> = file_refs.iter().map(|f| ParsedFile {
            path: f.path.clone(),
            language: f.language.clone(),
            content_hash: String::new(),
            size: 0,
            line_count: 0,
            content: None,
            has_errors: false,
            symbols: f.symbols.clone(),
            refs: f.refs.clone(),
            routes: vec![],
            db_sets: vec![],
        }).collect();
        let index = SymbolIndex::build(&owned, &id_map);
        (index, id_map)
    }

    #[test]
    fn test_scope_chain_resolution() {
        let file = make_file("src/foo.cs", vec![
            make_symbol("NS", "NS", SymbolKind::Namespace, Visibility::Public, None),
            make_symbol("Foo", "NS.Foo", SymbolKind::Class, Visibility::Public, Some("NS")),
            make_symbol("Bar", "NS.Foo.Bar", SymbolKind::Method, Visibility::Public, Some("NS.Foo")),
            make_symbol("Baz", "NS.Foo.Baz", SymbolKind::Method, Visibility::Public, Some("NS.Foo")),
        ], vec![
            make_ref(2, "Baz", EdgeKind::Calls, 5),
        ]);

        let (index, id_map) = build_test_env(&[&file]);
        let resolver = CSharpResolver;
        let file_ctx = resolver.build_file_context(&file);

        let ref_ctx = RefContext {
            extracted_ref: &file.refs[0],
            source_symbol: &file.symbols[2],
            scope_chain: build_scope_chain(file.symbols[2].scope_path.as_deref()),
        };

        let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
        assert!(result.is_some(), "Should resolve Baz via scope chain");
        let res = result.unwrap();
        assert_eq!(res.confidence, 1.0);
        assert_eq!(res.strategy, "csharp_scope_chain");
        assert_eq!(res.target_symbol_id, *id_map.get(&("src/foo.cs".to_string(), "NS.Foo.Baz".to_string())).unwrap());
    }

    #[test]
    fn test_same_namespace_resolution() {
        let file1 = make_file("src/Product.cs", vec![
            make_symbol("Models", "App.Models", SymbolKind::Namespace, Visibility::Public, None),
            make_symbol("Product", "App.Models.Product", SymbolKind::Class, Visibility::Public, Some("App.Models")),
        ], vec![]);

        let file2 = make_file("src/ProductService.cs", vec![
            make_symbol("Models", "App.Models", SymbolKind::Namespace, Visibility::Public, None),
            make_symbol("ProductService", "App.Models.ProductService", SymbolKind::Class, Visibility::Public, Some("App.Models")),
        ], vec![
            make_ref(1, "Product", EdgeKind::TypeRef, 3),
        ]);

        let (index, id_map) = build_test_env(&[&file1, &file2]);
        let resolver = CSharpResolver;
        let file_ctx = resolver.build_file_context(&file2);

        let ref_ctx = RefContext {
            extracted_ref: &file2.refs[0],
            source_symbol: &file2.symbols[1],
            scope_chain: build_scope_chain(file2.symbols[1].scope_path.as_deref()),
        };

        let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
        assert!(result.is_some(), "Should resolve Product");
        let res = result.unwrap();
        assert_eq!(res.confidence, 1.0);
        // May resolve via scope_chain (scope_path = "App.Models") or same_namespace — both correct
        assert!(res.strategy == "csharp_scope_chain" || res.strategy == "csharp_same_namespace");
        assert_eq!(res.target_symbol_id, *id_map.get(&("src/Product.cs".to_string(), "App.Models.Product".to_string())).unwrap());
    }

    #[test]
    fn test_using_directive_resolution() {
        let file1 = make_file("src/Product.cs", vec![
            make_symbol("Product", "App.Models.Product", SymbolKind::Class, Visibility::Public, Some("App.Models")),
        ], vec![]);

        let mut file2 = make_file("src/Controller.cs", vec![
            make_symbol("Controllers", "App.Controllers", SymbolKind::Namespace, Visibility::Public, None),
            make_symbol("ProductController", "App.Controllers.ProductController", SymbolKind::Class, Visibility::Public, Some("App.Controllers")),
        ], vec![
            make_ref(1, "Product", EdgeKind::TypeRef, 5),
        ]);
        file2.refs.push(ExtractedRef {
            source_symbol_index: 0,
            target_name: "App.Models".to_string(),
            kind: EdgeKind::Imports,
            line: 1,
            module: Some("App.Models".to_string()),
        });

        let (index, id_map) = build_test_env(&[&file1, &file2]);
        let resolver = CSharpResolver;
        let file_ctx = resolver.build_file_context(&file2);

        let ref_ctx = RefContext {
            extracted_ref: &file2.refs[0],
            source_symbol: &file2.symbols[1],
            scope_chain: build_scope_chain(file2.symbols[1].scope_path.as_deref()),
        };

        let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
        assert!(result.is_some(), "Should resolve Product via using directive");
        let res = result.unwrap();
        assert_eq!(res.confidence, 1.0);
        assert_eq!(res.strategy, "csharp_using_directive");
        assert_eq!(res.target_symbol_id, *id_map.get(&("src/Product.cs".to_string(), "App.Models.Product".to_string())).unwrap());
    }

    #[test]
    fn test_qualified_name_resolution() {
        let file1 = make_file("src/Utils.cs", vec![
            make_symbol("Helper", "App.Utils.Helper", SymbolKind::Class, Visibility::Public, Some("App.Utils")),
        ], vec![]);

        let file2 = make_file("src/Main.cs", vec![
            make_symbol("Main", "App.Main", SymbolKind::Class, Visibility::Public, Some("App")),
        ], vec![
            make_ref(0, "App.Utils.Helper", EdgeKind::TypeRef, 10),
        ]);

        let (index, _) = build_test_env(&[&file1, &file2]);
        let resolver = CSharpResolver;
        let file_ctx = resolver.build_file_context(&file2);

        let ref_ctx = RefContext {
            extracted_ref: &file2.refs[0],
            source_symbol: &file2.symbols[0],
            scope_chain: build_scope_chain(file2.symbols[0].scope_path.as_deref()),
        };

        let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
        assert!(result.is_some(), "Should resolve via fully qualified name");
        assert_eq!(result.unwrap().confidence, 1.0);
    }

    #[test]
    fn test_private_visibility_cross_file() {
        let file1 = make_file("src/Internal.cs", vec![
            make_symbol("Secret", "App.Internal.Secret", SymbolKind::Method, Visibility::Private, Some("App.Internal")),
        ], vec![]);

        let mut file2 = make_file("src/External.cs", vec![
            make_symbol("App", "App", SymbolKind::Namespace, Visibility::Public, None),
            make_symbol("External", "App.External", SymbolKind::Class, Visibility::Public, Some("App")),
        ], vec![
            make_ref(1, "Secret", EdgeKind::Calls, 5),
        ]);
        file2.refs.push(ExtractedRef {
            source_symbol_index: 0,
            target_name: "App.Internal".to_string(),
            kind: EdgeKind::Imports,
            line: 1,
            module: Some("App.Internal".to_string()),
        });

        let (index, _) = build_test_env(&[&file1, &file2]);
        let resolver = CSharpResolver;
        let file_ctx = resolver.build_file_context(&file2);

        let ref_ctx = RefContext {
            extracted_ref: &file2.refs[0],
            source_symbol: &file2.symbols[1],
            scope_chain: build_scope_chain(file2.symbols[1].scope_path.as_deref()),
        };

        assert!(resolver.resolve(&file_ctx, &ref_ctx, &index).is_none(), "Private cross-file should not resolve");
    }

    #[test]
    fn test_private_visibility_same_file() {
        let file = make_file("src/MyClass.cs", vec![
            make_symbol("NS", "NS", SymbolKind::Namespace, Visibility::Public, None),
            make_symbol("MyClass", "NS.MyClass", SymbolKind::Class, Visibility::Public, Some("NS")),
            make_symbol("PublicMethod", "NS.MyClass.PublicMethod", SymbolKind::Method, Visibility::Public, Some("NS.MyClass")),
            make_symbol("PrivateHelper", "NS.MyClass.PrivateHelper", SymbolKind::Method, Visibility::Private, Some("NS.MyClass")),
        ], vec![
            make_ref(2, "PrivateHelper", EdgeKind::Calls, 8),
        ]);

        let (index, id_map) = build_test_env(&[&file]);
        let resolver = CSharpResolver;
        let file_ctx = resolver.build_file_context(&file);

        let ref_ctx = RefContext {
            extracted_ref: &file.refs[0],
            source_symbol: &file.symbols[2],
            scope_chain: build_scope_chain(file.symbols[2].scope_path.as_deref()),
        };

        let result = resolver.resolve(&file_ctx, &ref_ctx, &index);
        assert!(result.is_some(), "Private same-file should resolve");
        assert_eq!(result.unwrap().target_symbol_id, *id_map.get(&("src/MyClass.cs".to_string(), "NS.MyClass.PrivateHelper".to_string())).unwrap());
    }

    #[test]
    fn test_falls_back_for_unknown() {
        let file = make_file("src/Test.cs", vec![
            make_symbol("Test", "App.Test", SymbolKind::Class, Visibility::Public, Some("App")),
        ], vec![
            make_ref(0, "NonExistentType", EdgeKind::TypeRef, 5),
        ]);

        let (index, _) = build_test_env(&[&file]);
        let resolver = CSharpResolver;
        let file_ctx = resolver.build_file_context(&file);

        let ref_ctx = RefContext {
            extracted_ref: &file.refs[0],
            source_symbol: &file.symbols[0],
            scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
        };

        assert!(resolver.resolve(&file_ctx, &ref_ctx, &index).is_none(), "Unknown should fall back");
    }
}
