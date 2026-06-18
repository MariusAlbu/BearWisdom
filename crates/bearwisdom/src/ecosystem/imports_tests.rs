use super::*;
use crate::types::ExtractedRef;

fn typeref(target: &str) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.into(),
        kind: EdgeKind::TypeRef,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
        chain: None,
        byte_offset: 0,
    }
}

fn ref_of(target: &str, kind: EdgeKind) -> ExtractedRef {
    let mut r = typeref(target);
    r.kind = kind;
    r
}

fn ns(local: &str, module: &str) -> ImportEntry {
    ImportEntry {
        local_name: local.into(),
        module: module.into(),
        kind: ImportKind::Namespace,
    }
}

fn named(local: &str, exported: &str, module: &str) -> ImportEntry {
    ImportEntry {
        local_name: local.into(),
        module: module.into(),
        kind: ImportKind::Named {
            exported_name: exported.into(),
        },
    }
}

#[test]
fn namespace_qualified_typeref_splits_prefix() {
    let mut refs = vec![typeref("Oazapfts.RequestOpts")];
    let imports: HashMap<_, _> =
        [("Oazapfts".to_string(), ns("Oazapfts", "@oazapfts/runtime"))]
            .into_iter()
            .collect();
    resolve_import_refs(&mut refs, &imports);
    assert_eq!(refs[0].target_name, "RequestOpts");
    assert_eq!(refs[0].module.as_deref(), Some("@oazapfts/runtime"));
    assert!(refs[0].namespace_segments.is_empty());
}

#[test]
fn three_segment_namespace_carries_intermediate() {
    let mut refs = vec![typeref("Express.Multer.File")];
    let imports: HashMap<_, _> = [("Express".to_string(), ns("Express", "express"))]
        .into_iter()
        .collect();
    resolve_import_refs(&mut refs, &imports);
    assert_eq!(refs[0].target_name, "File");
    assert_eq!(refs[0].namespace_segments, vec!["Multer".to_string()]);
    assert_eq!(refs[0].module.as_deref(), Some("express"));
}

#[test]
fn renamed_named_import_substitutes_target() {
    // import { foo as bar } from 'pkg'; ... bar() — ref carries
    // target_name="bar", needs to become "foo".
    let mut refs = vec![ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: "bar".into(),
        kind: EdgeKind::Calls,
        line: 0,
        col: 0,
        module: None,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
        chain: None,
        byte_offset: 0,
    }];
    let imports: HashMap<_, _> = [("bar".to_string(), named("bar", "foo", "pkg"))]
        .into_iter()
        .collect();
    resolve_import_refs(&mut refs, &imports);
    assert_eq!(refs[0].target_name, "foo");
    assert_eq!(refs[0].module.as_deref(), Some("pkg"));
}

#[test]
fn unmapped_target_left_alone() {
    let mut refs = vec![typeref("LocalThing")];
    let imports: HashMap<_, _> = [("Other".to_string(), ns("Other", "other-pkg"))]
        .into_iter()
        .collect();
    resolve_import_refs(&mut refs, &imports);
    assert_eq!(refs[0].target_name, "LocalThing");
    assert!(refs[0].module.is_none());
}

#[test]
fn already_canonicalized_skipped() {
    let mut refs = vec![ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: "X".into(),
        kind: EdgeKind::TypeRef,
        line: 0,
        col: 0,
        module: Some("preset".into()),
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
        chain: None,
        byte_offset: 0,
    }];
    let imports: HashMap<_, _> = [("X".to_string(), named("X", "Y", "different"))]
        .into_iter()
        .collect();
    resolve_import_refs(&mut refs, &imports);
    assert_eq!(refs[0].target_name, "X");
    assert_eq!(refs[0].module.as_deref(), Some("preset"));
}

#[test]
fn idempotent_double_apply() {
    let mut refs = vec![typeref("Foo.Bar")];
    let imports: HashMap<_, _> = [("Foo".to_string(), ns("Foo", "pkg"))]
        .into_iter()
        .collect();
    resolve_import_refs(&mut refs, &imports);
    let after_first = refs.clone();
    resolve_import_refs(&mut refs, &imports);
    assert_eq!(refs[0].target_name, after_first[0].target_name);
    assert_eq!(refs[0].module, after_first[0].module);
    assert_eq!(
        refs[0].namespace_segments,
        after_first[0].namespace_segments
    );
}

#[test]
fn bare_non_renamed_typeref_gets_module_from_import() {
    // `import { QueryClient } from '@tanstack/query-core'; let q: QueryClient` —
    // the bare usage ref must carry the import's module so the use site's package
    // scope reaches the resolver, not just the binding ref.
    let mut refs = vec![typeref("QueryClient")];
    let imports: HashMap<_, _> = [(
        "QueryClient".to_string(),
        named("QueryClient", "QueryClient", "@tanstack/query-core"),
    )]
    .into_iter()
    .collect();
    resolve_import_refs(&mut refs, &imports);
    assert_eq!(refs[0].target_name, "QueryClient");
    assert_eq!(refs[0].module.as_deref(), Some("@tanstack/query-core"));
}

#[test]
fn bare_instantiates_gets_module_from_import() {
    // `new QueryClient()` emits an Instantiates ref whose local name is imported;
    // it too must carry the importing package's specifier.
    let mut refs = vec![ref_of("QueryClient", EdgeKind::Instantiates)];
    let imports: HashMap<_, _> = [(
        "QueryClient".to_string(),
        named("QueryClient", "QueryClient", "@tanstack/query-core"),
    )]
    .into_iter()
    .collect();
    resolve_import_refs(&mut refs, &imports);
    assert_eq!(refs[0].module.as_deref(), Some("@tanstack/query-core"));
}
