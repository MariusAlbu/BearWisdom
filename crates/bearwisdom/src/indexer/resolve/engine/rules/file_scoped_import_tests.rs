use super::*;
use crate::indexer::resolve::engine::contract::{FileContext, ImportEntry, Symbol, SymbolLookup, SymbolSet};
use crate::indexer::resolve::engine::testkit::{accept_any, call_ref, ref_ctx, source_symbol, sym};
use crate::indexer::resolve::engine::{BinderContext, LookupResult};
use crate::type_checker::profile::language_profile::{
    AliasDecode, FileScopedImports, LanguageProfile, DEFAULT_PROFILE,
};

/// Minimal lookup with `in_file` support. The testkit `Lookup.in_file` always
/// returns empty, so this double is needed for pass-1 and pass-2 probes.
struct FileLookup {
    by_file: std::collections::HashMap<String, Vec<Symbol>>,
    empty: Vec<Symbol>,
    empty_pairs: Vec<(String, String)>,
}

impl FileLookup {
    fn new() -> Self {
        Self {
            by_file: Default::default(),
            empty: Vec::new(),
            empty_pairs: Vec::new(),
        }
    }

    fn with(mut self, s: Symbol) -> Self {
        self.by_file
            .entry(s.file_path.to_string())
            .or_default()
            .push(s);
        self
    }
}

impl SymbolLookup for FileLookup {
    fn by_name(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn by_qualified_name(&self, _: &str) -> Option<&Symbol> {
        None
    }
    fn all_by_qualified_name(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn members_of(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn types_by_name(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn in_namespace(&self, _: &str) -> Vec<&Symbol> {
        Vec::new()
    }
    fn has_in_namespace(&self, _: &str) -> bool {
        false
    }
    fn in_file(&self, file_path: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(
            self.by_file
                .get(file_path)
                .map(|v| v.as_slice())
                .unwrap_or(&self.empty),
        )
    }
    fn field_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn return_type_name(&self, _: &str) -> Option<&str> {
        None
    }
    fn generic_params(&self, _: &str) -> Option<Vec<String>> {
        None
    }
    fn reexports_from(&self, _: &str) -> &[(String, String)] {
        &self.empty_pairs
    }
    fn is_external_name(&self, _: &str, _: &str) -> bool {
        false
    }
}

fn make_ctx<'a>(
    lookup: &'a FileLookup,
    target: &'a str,
    imports: Vec<ImportEntry>,
    profile: &'a LanguageProfile,
) -> (crate::types::ExtractedRef, crate::types::ExtractedSymbol, FileContext) {
    let r = call_ref(target);
    let s = source_symbol("caller");
    let fc = FileContext {
        file_path: "src/main.robot".to_string(),
        language: "robot".to_string(),
        imports,
        file_namespace: None,
    };
    (r, s, fc)
}

// profile: file_scoped_imports ON, every import scanned (wildcard_only: false)
const ON_PROFILE: LanguageProfile = LanguageProfile {
    implicit_root_types: &[],
    file_scoped_imports: FileScopedImports::On {
        wildcard_only: false,
        alias_decode: None,
    },
    ..DEFAULT_PROFILE
};

// profile: file_scoped_imports ON, wildcard_only
const WILDCARD_PROFILE: LanguageProfile = LanguageProfile {
    implicit_root_types: &[],
    file_scoped_imports: FileScopedImports::On {
        wildcard_only: true,
        alias_decode: None,
    },
    ..DEFAULT_PROFILE
};

// profile: file_scoped_imports ON with alias_decode
const ALIAS_DECODE_PROFILE: LanguageProfile = LanguageProfile {
    implicit_root_types: &[],
    file_scoped_imports: FileScopedImports::On {
        wildcard_only: false,
        alias_decode: Some(AliasDecode {
            separator: ".",
            fallback_kind: Some("class"),
        }),
    },
    ..DEFAULT_PROFILE
};

#[test]
fn binds_symbol_by_name_in_imported_file_pass1() {
    let lookup =
        FileLookup::new().with(sym(50, "MyKeyword", "MyKeyword", "function", "lib/my_lib.py"));
    let imports = vec![ImportEntry {
        imported_name: "MyLibrary".to_string(),
        module_path: Some("lib/my_lib.py".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    let r = call_ref("MyKeyword");
    let s = source_symbol("caller");
    let fc = FileContext {
        file_path: "src/test.robot".to_string(),
        language: "robot".to_string(),
        imports,
        file_namespace: None,
    };
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &ON_PROFILE,
    };
    match FileScopedImportRule.apply(&ctx) {
        LookupResult::Resolved(res) => assert_eq!(res.target_symbol_id, 50),
        _ => panic!("expected Resolved"),
    }
}

#[test]
fn declines_when_gate_is_off() {
    let lookup =
        FileLookup::new().with(sym(51, "MyKeyword", "MyKeyword", "function", "lib/my_lib.py"));
    let imports = vec![ImportEntry {
        imported_name: "MyLibrary".to_string(),
        module_path: Some("lib/my_lib.py".to_string()),
        alias: None,
        is_wildcard: false,
    }];
    let r = call_ref("MyKeyword");
    let s = source_symbol("caller");
    let fc = FileContext {
        file_path: "src/test.robot".to_string(),
        language: "robot".to_string(),
        imports,
        file_namespace: None,
    };
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &DEFAULT_PROFILE, // FileScopedImports::Off
    };
    assert!(matches!(FileScopedImportRule.apply(&ctx), LookupResult::Pass));
}

#[test]
fn declines_non_wildcard_import_when_wildcard_only() {
    let lookup =
        FileLookup::new().with(sym(52, "MyKeyword", "MyKeyword", "function", "lib/my_lib.py"));
    let imports = vec![ImportEntry {
        imported_name: "MyLibrary".to_string(),
        module_path: Some("lib/my_lib.py".to_string()),
        alias: None,
        is_wildcard: false, // not a wildcard import
    }];
    let r = call_ref("MyKeyword");
    let s = source_symbol("caller");
    let fc = FileContext {
        file_path: "src/test.robot".to_string(),
        language: "robot".to_string(),
        imports,
        file_namespace: None,
    };
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &WILDCARD_PROFILE,
    };
    assert!(matches!(FileScopedImportRule.apply(&ctx), LookupResult::Pass));
}

#[test]
fn binds_via_alias_decode_type_member() {
    // Import entry: imported_name="MyKeyword", alias="DispatchClass.my_method",
    // module_path="lib/dispatch.py" → bind sym named "my_method" in the file.
    let lookup = FileLookup::new()
        .with(sym(53, "my_method", "DispatchClass.my_method", "function", "lib/dispatch.py"))
        .with(sym(54, "DispatchClass", "DispatchClass", "class", "lib/dispatch.py"));
    let imports = vec![ImportEntry {
        imported_name: "MyKeyword".to_string(),
        module_path: Some("lib/dispatch.py".to_string()),
        alias: Some("DispatchClass.my_method".to_string()),
        is_wildcard: false,
    }];
    let r = call_ref("MyKeyword");
    let s = source_symbol("caller");
    let fc = FileContext {
        file_path: "src/test.robot".to_string(),
        language: "robot".to_string(),
        imports,
        file_namespace: None,
    };
    let rc = ref_ctx(&r, &s, vec![]);
    let kind = accept_any;
    let ctx = BinderContext {
        file_ctx: &fc,
        ref_ctx: &rc,
        lookup: &lookup,
        kind: &kind,
        profile: &ALIAS_DECODE_PROFILE,
    };
    match FileScopedImportRule.apply(&ctx) {
        LookupResult::Resolved(res) => assert_eq!(res.target_symbol_id, 53),
        _ => panic!("expected Resolved to my_method"),
    }
}
