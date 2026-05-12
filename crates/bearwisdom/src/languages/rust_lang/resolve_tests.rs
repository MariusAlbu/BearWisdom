use super::predicates;
use super::resolve::RustResolver;
use crate::ecosystem::manifest::{ManifestData, ManifestKind};
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{build_scope_chain, LanguageResolver, RefContext};
use crate::types::*;

fn make_symbol(
    name: &str,
    qname: &str,
    kind: SymbolKind,
    vis: Visibility,
    scope: Option<&str>,
) -> ExtractedSymbol {
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
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn make_file(path: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "rust".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols,
        refs,
        routes: vec![],
        db_sets: vec![],
        symbol_origin_languages: vec![],
        ref_origin_languages: vec![],
        symbol_from_snippet: vec![],
        flow: crate::types::FlowMeta::default(),
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
    }
}

fn cargo_ctx_with(deps: &[&str]) -> ProjectContext {
    let mut ctx = ProjectContext::default();
    let mut cargo = ManifestData::default();
    for d in deps {
        cargo.dependencies.insert((*d).to_string());
    }
    ctx.manifests.insert(ManifestKind::Cargo, cargo);
    ctx
}

// ---------------------------------------------------------------------------
// External namespace inference: bare `crate::path` attribution via Cargo.toml
// ---------------------------------------------------------------------------

#[test]
fn bare_anyhow_path_attributed_to_anyhow_not_std() {
    // Pre-fix bug: `is_rust_builtin` returned true for any name whose first
    // `::` segment matched a hardcoded crate list ("anyhow", "tokio", ...).
    // The caller then unconditionally returned `Some("std")`, silently
    // misattributing every match. Now the manifest is consulted directly.
    let ctx = cargo_ctx_with(&["anyhow"]);
    let file = make_file(
        "src/lib.rs",
        vec![make_symbol("root", "root", SymbolKind::Function, Visibility::Public, None)],
        vec![make_ref(0, "anyhow::anyhow", EdgeKind::Calls, 5)],
    );

    let resolver = RustResolver;
    let file_ctx = resolver.build_file_context(&file, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
        file_package_id: None,
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, Some(&ctx));
    assert_eq!(ns.as_deref(), Some("anyhow"));
}

#[test]
fn bare_hyphenated_crate_normalized_to_underscore() {
    // Cargo.toml declares `serde-json` (hypothetically); source uses
    // `serde_json::json`. Attribution should still match.
    let ctx = cargo_ctx_with(&["serde-json"]);
    let file = make_file(
        "src/lib.rs",
        vec![make_symbol("root", "root", SymbolKind::Function, Visibility::Public, None)],
        vec![make_ref(0, "serde_json::json", EdgeKind::Calls, 5)],
    );

    let resolver = RustResolver;
    let file_ctx = resolver.build_file_context(&file, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
        file_package_id: None,
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, Some(&ctx));
    assert_eq!(ns.as_deref(), Some("serde_json"));
}

#[test]
fn bare_stdlib_path_still_routes_to_std() {
    let ctx = cargo_ctx_with(&[]);
    let file = make_file(
        "src/lib.rs",
        vec![make_symbol("root", "root", SymbolKind::Function, Visibility::Public, None)],
        vec![make_ref(0, "std::collections::HashMap", EdgeKind::TypeRef, 5)],
    );

    let resolver = RustResolver;
    let file_ctx = resolver.build_file_context(&file, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
        file_package_id: None,
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, Some(&ctx));
    assert_eq!(ns.as_deref(), Some("std"));
}

#[test]
fn bare_crate_path_internal_not_attributed_external() {
    let ctx = cargo_ctx_with(&["anyhow"]);
    let file = make_file(
        "src/lib.rs",
        vec![make_symbol("root", "root", SymbolKind::Function, Visibility::Public, None)],
        vec![make_ref(0, "crate::models::User", EdgeKind::TypeRef, 5)],
    );

    let resolver = RustResolver;
    let file_ctx = resolver.build_file_context(&file, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
        file_package_id: None,
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, Some(&ctx));
    assert!(ns.is_none(), "crate:: paths should not be classified external");
}

#[test]
fn unknown_bare_path_not_attributed_when_not_in_manifest() {
    // A name that used to be in the hardcoded list but isn't in the manifest
    // should NOT be silently classified as external. Without the list, we
    // require the manifest entry to claim attribution.
    let ctx = cargo_ctx_with(&["serde"]); // anyhow NOT declared
    let file = make_file(
        "src/lib.rs",
        vec![make_symbol("root", "root", SymbolKind::Function, Visibility::Public, None)],
        vec![make_ref(0, "anyhow::anyhow", EdgeKind::Calls, 5)],
    );

    let resolver = RustResolver;
    let file_ctx = resolver.build_file_context(&file, Some(&ctx));
    let ref_ctx = RefContext {
        extracted_ref: &file.refs[0],
        source_symbol: &file.symbols[0],
        scope_chain: build_scope_chain(file.symbols[0].scope_path.as_deref()),
        file_package_id: None,
    };

    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, Some(&ctx));
    assert!(
        ns.is_none(),
        "anyhow::* without a manifest declaration must not auto-classify"
    );
}
