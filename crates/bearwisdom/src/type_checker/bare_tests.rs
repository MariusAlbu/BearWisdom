// Tests for the engine bare-name resolver. Each test exercises one
// strategy in isolation by wiring a synthetic SymbolLookup and minimal
// FileContext / RefContext.

use std::sync::Arc;

use super::resolve_bare;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, SymbolInfo, SymbolLookup,
};
use crate::type_checker::profile::language_profile::DEFAULT_PROFILE;
use crate::types::{
    AliasTarget, EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility,
};

#[derive(Default)]
struct StubLookup {
    by_qname: Vec<SymbolInfo>,
    by_file: Vec<SymbolInfo>,
    empty: Vec<SymbolInfo>,
    empty_reexports: Vec<(String, String)>,
}

impl StubLookup {
    fn push_sym(
        &mut self,
        id: i64,
        name: &str,
        qualified_name: &str,
        kind: &str,
        file_path: &str,
        target: BucketTarget,
    ) {
        let sym = SymbolInfo {
            id,
            name: name.into(),
            qualified_name: qualified_name.into(),
            kind: kind.into(),
            visibility: None,
            file_path: Arc::from(file_path),
            scope_path: None,
            package_id: None,
            signature: None,
        };
        match target {
            BucketTarget::Qname => self.by_qname.push(sym),
            BucketTarget::File => self.by_file.push(sym),
        }
    }
}

enum BucketTarget {
    Qname,
    File,
}

impl SymbolLookup for StubLookup {
    fn by_name(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn by_qualified_name(&self, qname: &str) -> Option<&SymbolInfo> {
        self.by_qname.iter().find(|s| s.qualified_name == qname)
    }
    fn all_by_qualified_name(&self, qname: &str) -> &[SymbolInfo] {
        // Caller iterates; return a one-slot slice when a match exists.
        for sym in &self.by_qname {
            if sym.qualified_name == qname {
                return std::slice::from_ref(sym);
            }
        }
        &self.empty
    }
    fn members_of(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn types_by_name(&self, _: &str) -> &[SymbolInfo] {
        &self.empty
    }
    fn in_namespace(&self, _: &str) -> Vec<&SymbolInfo> {
        Vec::new()
    }
    fn has_in_namespace(&self, _: &str) -> bool {
        false
    }
    fn in_file(&self, file_path: &str) -> &[SymbolInfo] {
        if self
            .by_file
            .iter()
            .any(|s| s.file_path.as_ref() == file_path)
        {
            &self.by_file
        } else {
            &self.empty
        }
    }
    fn field_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn return_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn field_type_args(&self, _: &str) -> Option<&[String]> {
        None
    }
    fn generic_params(&self, _: &str) -> Option<&[String]> {
        None
    }
    fn alias_target(&self, _: &str) -> Option<&AliasTarget> {
        None
    }
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &self.empty_reexports
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
}

fn mk_ref(target: &str, kind: EdgeKind) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind,
        line: 1,
        col: 1,
        module: None,
        namespace_segments: Vec::new(),
        chain: None,
        byte_offset: 0,
        call_args: Vec::new(),
    }
}

fn mk_source_symbol(qname: &str) -> ExtractedSymbol {
    let (scope, name) = match qname.rsplit_once('.') {
        Some((s, n)) => (Some(s.to_string()), n.to_string()),
        None => (None, qname.to_string()),
    };
    ExtractedSymbol {
        name,
        qualified_name: qname.to_string(),
        kind: SymbolKind::Method,
        visibility: Some(Visibility::Public),
        start_line: 1,
        end_line: 1,
        start_col: 1,
        end_col: 1,
        byte_offset: 0,
        signature: None,
        doc_comment: None,
        scope_path: scope,
        parent_index: None,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

#[test]
fn scope_chain_walk_resolves_method_in_enclosing_class() {
    let mut lookup = StubLookup::default();
    lookup.push_sym(
        42,
        "buildUserRO",
        "UserService.buildUserRO",
        "method",
        "src/user/user.service.ts",
        BucketTarget::Qname,
    );

    let source_sym = mk_source_symbol("UserService.findOne");
    let extracted = mk_ref("buildUserRO", EdgeKind::Calls);
    let ref_ctx = RefContext {
        extracted_ref: &extracted,
        source_symbol: &source_sym,
        scope_chain: vec![
            "UserService.findOne".to_string(),
            "UserService".to_string(),
        ],
        file_package_id: None,
    };
    let file_ctx = FileContext {
        file_path: "src/user/user.service.ts".to_string(),
        language: "typescript".to_string(),
        imports: Vec::new(),
        file_namespace: None,
    };

    let res = resolve_bare(&ref_ctx, &file_ctx, &lookup, &DEFAULT_PROFILE)
        .expect("scope-chain walk resolves UserService.buildUserRO");
    assert_eq!(res.target_symbol_id, 42);
    assert_eq!(res.strategy, "engine_bare_scope");
}

#[test]
fn same_file_resolves_top_level_function() {
    let mut lookup = StubLookup::default();
    lookup.push_sym(
        7,
        "formatDate",
        "formatDate",
        "function",
        "src/utils.ts",
        BucketTarget::File,
    );

    let source_sym = mk_source_symbol("renderPage");
    let extracted = mk_ref("formatDate", EdgeKind::Calls);
    let ref_ctx = RefContext {
        extracted_ref: &extracted,
        source_symbol: &source_sym,
        scope_chain: vec!["renderPage".to_string()],
        file_package_id: None,
    };
    let file_ctx = FileContext {
        file_path: "src/utils.ts".to_string(),
        language: "typescript".to_string(),
        imports: Vec::new(),
        file_namespace: None,
    };

    let res = resolve_bare(&ref_ctx, &file_ctx, &lookup, &DEFAULT_PROFILE)
        .expect("same-file lookup resolves formatDate");
    assert_eq!(res.target_symbol_id, 7);
    assert_eq!(res.strategy, "engine_bare_same_file");
}

#[test]
fn fully_qualified_target_resolves_directly() {
    let mut lookup = StubLookup::default();
    lookup.push_sym(
        99,
        "INSTANCE",
        "Logger.INSTANCE",
        "variable",
        "src/log.ts",
        BucketTarget::Qname,
    );

    let source_sym = mk_source_symbol("App.main");
    let extracted = mk_ref("Logger.INSTANCE", EdgeKind::Reads);
    let ref_ctx = RefContext {
        extracted_ref: &extracted,
        source_symbol: &source_sym,
        scope_chain: vec!["App.main".to_string(), "App".to_string()],
        file_package_id: None,
    };
    let file_ctx = FileContext {
        file_path: "src/main.ts".to_string(),
        language: "typescript".to_string(),
        imports: Vec::new(),
        file_namespace: None,
    };

    let res = resolve_bare(&ref_ctx, &file_ctx, &lookup, &DEFAULT_PROFILE)
        .expect("dotted target hits qname path");
    assert_eq!(res.target_symbol_id, 99);
    assert_eq!(res.strategy, "engine_bare_qname");
}

#[test]
fn import_qualified_resolves_external_symbol() {
    let mut lookup = StubLookup::default();
    lookup.push_sym(
        14,
        "createContext",
        "react.createContext",
        "function",
        "ext:ts:react/index.d.ts",
        BucketTarget::Qname,
    );

    let source_sym = mk_source_symbol("AppShell.render");
    let extracted = mk_ref("createContext", EdgeKind::Calls);
    let ref_ctx = RefContext {
        extracted_ref: &extracted,
        source_symbol: &source_sym,
        scope_chain: vec!["AppShell.render".to_string(), "AppShell".to_string()],
        file_package_id: None,
    };
    let file_ctx = FileContext {
        file_path: "src/app.tsx".to_string(),
        language: "typescript".to_string(),
        imports: vec![ImportEntry {
            imported_name: "createContext".to_string(),
            module_path: Some("react".to_string()),
            alias: None,
            is_wildcard: false,
        }],
        file_namespace: None,
    };

    let res = resolve_bare(&ref_ctx, &file_ctx, &lookup, &DEFAULT_PROFILE)
        .expect("import-qualified lookup resolves react.createContext");
    assert_eq!(res.target_symbol_id, 14);
    assert_eq!(res.strategy, "engine_bare_import_qname");
}

#[test]
fn imports_edge_kind_short_circuits() {
    let lookup = StubLookup::default();
    let source_sym = mk_source_symbol("AppShell.render");
    let extracted = mk_ref("react", EdgeKind::Imports);
    let ref_ctx = RefContext {
        extracted_ref: &extracted,
        source_symbol: &source_sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    let file_ctx = FileContext {
        file_path: "src/app.tsx".to_string(),
        language: "typescript".to_string(),
        imports: Vec::new(),
        file_namespace: None,
    };

    assert!(resolve_bare(&ref_ctx, &file_ctx, &lookup, &DEFAULT_PROFILE).is_none());
}

#[test]
fn miss_returns_none_so_legacy_fallback_runs() {
    let lookup = StubLookup::default();
    let source_sym = mk_source_symbol("AppShell.render");
    let extracted = mk_ref("unknownThing", EdgeKind::Calls);
    let ref_ctx = RefContext {
        extracted_ref: &extracted,
        source_symbol: &source_sym,
        scope_chain: vec!["AppShell".to_string()],
        file_package_id: None,
    };
    let file_ctx = FileContext {
        file_path: "src/app.tsx".to_string(),
        language: "typescript".to_string(),
        imports: Vec::new(),
        file_namespace: None,
    };

    assert!(resolve_bare(&ref_ctx, &file_ctx, &lookup, &DEFAULT_PROFILE).is_none());
}
