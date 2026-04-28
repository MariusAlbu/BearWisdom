//! Tests for `angular::resolve::AngularResolver`.

use super::*;

#[test]
fn paired_ts_for_component_template() {
    assert_eq!(
        paired_ts_for_template("src/app/foo.component.html").as_deref(),
        Some("src/app/foo.component.ts")
    );
}

#[test]
fn paired_ts_for_container_template() {
    assert_eq!(
        paired_ts_for_template("src/app/bar.container.html").as_deref(),
        Some("src/app/bar.container.ts")
    );
}

#[test]
fn paired_ts_for_dialog_template() {
    assert_eq!(
        paired_ts_for_template("src/app/baz.dialog.html").as_deref(),
        Some("src/app/baz.dialog.ts")
    );
}

#[test]
fn paired_ts_returns_none_for_plain_html() {
    assert_eq!(paired_ts_for_template("index.html"), None);
    assert_eq!(paired_ts_for_template("src/app/foo.component.ts"), None);
}

#[test]
fn companion_file_for_imports_delegates_to_paired_ts() {
    let r = AngularResolver;
    assert_eq!(
        r.companion_file_for_imports("src/app/foo.component.html").as_deref(),
        Some("src/app/foo.component.ts")
    );
    assert_eq!(
        r.companion_file_for_imports("src/app/unrelated.html"),
        None
    );
}

// ---------------------------------------------------------------------------
// Selector-map resolution (PR 18)
// ---------------------------------------------------------------------------

/// Minimal `SymbolLookup` stub for testing the selector-map path.
struct SelectorMapLookup {
    selectors: std::collections::HashMap<String, String>,
    symbols: Vec<crate::indexer::resolve::engine::SymbolInfo>,
}

impl SelectorMapLookup {
    fn new() -> Self {
        Self {
            selectors: std::collections::HashMap::new(),
            symbols: Vec::new(),
        }
    }

    fn with_selector(mut self, raw: &str, qname: &str) -> Self {
        self.selectors.insert(raw.to_string(), qname.to_string());
        self
    }

    fn with_symbol(mut self, id: i64, name: &str, qname: &str) -> Self {
        use std::sync::Arc;
        self.symbols.push(crate::indexer::resolve::engine::SymbolInfo {
            id,
            name: name.to_string(),
            qualified_name: qname.to_string(),
            kind: "class".to_string(),
            visibility: Some("public".to_string()),
            file_path: Arc::from("src/app/user-card.component.ts"),
            scope_path: None,
            package_id: None,
        });
        self
    }
}

impl crate::indexer::resolve::engine::SymbolLookup for SelectorMapLookup {
    fn by_name(&self, name: &str) -> &[crate::indexer::resolve::engine::SymbolInfo] {
        let _ = name;
        &self.symbols
    }

    fn by_qualified_name(&self, qname: &str) -> Option<&crate::indexer::resolve::engine::SymbolInfo> {
        self.symbols.iter().find(|s| s.qualified_name == qname)
    }

    fn members_of(&self, _p: &str) -> &[crate::indexer::resolve::engine::SymbolInfo] { &[] }
    fn types_by_name(&self, _n: &str) -> &[crate::indexer::resolve::engine::SymbolInfo] { &[] }
    fn in_namespace(&self, _n: &str) -> Vec<&crate::indexer::resolve::engine::SymbolInfo> { vec![] }
    fn has_in_namespace(&self, _n: &str) -> bool { false }
    fn in_file(&self, _f: &str) -> &[crate::indexer::resolve::engine::SymbolInfo] { &[] }
    fn field_type_name(&self, _q: &str) -> Option<&str> { None }
    fn return_type_name(&self, _q: &str) -> Option<&str> { None }
    fn field_type_args(&self, _q: &str) -> Option<&[String]> { None }
    fn generic_params(&self, _n: &str) -> Option<&[String]> { None }
    fn reexports_from(&self, _f: &str) -> &[(String, String)] { &[] }
    fn is_external_name(&self, _n: &str, _l: &str) -> bool { false }

    fn angular_selector(&self, raw_selector: &str) -> Option<&str> {
        self.selectors.get(raw_selector).map(|s| s.as_str())
    }
}

#[test]
fn selector_map_hit_resolves_to_class() {
    use crate::indexer::resolve::engine::{FileContext, RefContext, LanguageResolver};
    use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};

    let lookup = SelectorMapLookup::new()
        .with_selector("app-user-card", "src/app/user-card.UserCardComponent")
        .with_symbol(42, "UserCardComponent", "src/app/user-card.UserCardComponent");

    let host_sym = ExtractedSymbol {
        name: "parent".to_string(),
        qualified_name: "parent".to_string(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: 0, end_line: 0, start_col: 0, end_col: 0,
        signature: None, doc_comment: None, scope_path: None, parent_index: None,
    };

    let extracted = ExtractedRef {
        source_symbol_index: 0,
        target_name: "AppUserCard".to_string(),
        kind: EdgeKind::Calls,
        line: 5,
        // Raw selector stored by template extractor.
        module: Some("app-user-card".to_string()),
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
    };

    let file_ctx = FileContext {
        file_path: "src/app/parent.component.html".to_string(),
        language: "angular_template".to_string(),
        imports: Vec::new(),
        file_namespace: None,
    };

    let ref_ctx = RefContext {
        extracted_ref: &extracted,
        source_symbol: &host_sym,
        scope_chain: Vec::new(),
        file_package_id: None,
    };

    let resolution = AngularResolver.resolve(&file_ctx, &ref_ctx, &lookup);
    assert!(resolution.is_some(), "selector map hit should resolve");
    let res = resolution.unwrap();
    assert_eq!(res.target_symbol_id, 42);
    assert_eq!(res.strategy, "angular_selector_map");
    assert!((res.confidence - 1.0).abs() < f64::EPSILON);
}

#[test]
fn selector_map_miss_falls_through() {
    use crate::indexer::resolve::engine::{FileContext, RefContext, LanguageResolver};
    use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};

    // No selectors in the map — TypeScriptResolver should handle it.
    let lookup = SelectorMapLookup::new();

    let host_sym = ExtractedSymbol {
        name: "parent".to_string(),
        qualified_name: "parent".to_string(),
        kind: SymbolKind::Class,
        visibility: Some(Visibility::Public),
        start_line: 0, end_line: 0, start_col: 0, end_col: 0,
        signature: None, doc_comment: None, scope_path: None, parent_index: None,
    };

    let extracted = ExtractedRef {
        source_symbol_index: 0,
        target_name: "AppUserCard".to_string(),
        kind: EdgeKind::Calls,
        line: 5,
        module: Some("app-user-card".to_string()),
        chain: None,
        byte_offset: 0,
        namespace_segments: Vec::new(),
    };

    let file_ctx = FileContext {
        file_path: "src/app/parent.component.html".to_string(),
        language: "angular_template".to_string(),
        imports: Vec::new(),
        file_namespace: None,
    };

    let ref_ctx = RefContext {
        extracted_ref: &extracted,
        source_symbol: &host_sym,
        scope_chain: Vec::new(),
        file_package_id: None,
    };

    // Should not panic, resolution may be None (no imports to resolve against).
    let _result = AngularResolver.resolve(&file_ctx, &ref_ctx, &lookup);
    // We just verify it doesn't error — the TS resolver may return None here
    // since the lookup has no symbols and no imports are set up.
}
