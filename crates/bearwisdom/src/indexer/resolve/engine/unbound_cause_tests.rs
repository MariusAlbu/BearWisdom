use std::collections::HashMap;
use std::sync::Arc;

use super::{classify_unbound_root, classify_untyped_root};
use crate::indexer::resolve::engine::cause::CauseKind;
use crate::indexer::resolve::engine::contract::{
    FileContext, ImportEntry, Symbol as ContractSymbol, SymbolLookup, SymbolSet,
};

#[derive(Default)]
struct FakeLookup {
    by_name: HashMap<String, Vec<ContractSymbol>>,
    members: HashMap<String, Vec<ContractSymbol>>,
    external_names: Vec<String>,
    declared_deps: Vec<(Option<i64>, String)>,
    module_link: Option<String>,
    empty: Vec<ContractSymbol>,
    empty_pairs: Vec<(String, String)>,
}

fn sym(id: i64, name: &str, qname: &str) -> ContractSymbol {
    ContractSymbol {
        id,
        name: name.to_string(),
        qualified_name: qname.to_string(),
        kind: "method".to_string(),
        visibility: None,
        file_path: Arc::from("src/a.pas"),
        scope_path: None,
        package_id: None,
        signature: None,
    }
}

impl crate::indexer::resolve::engine::contract::FlowCacheLookup for FakeLookup {}

impl SymbolLookup for FakeLookup {
    fn by_name(&self, name: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(self.by_name.get(name).unwrap_or(&self.empty))
    }
    fn by_qualified_name(&self, _: &str) -> Option<&ContractSymbol> {
        None
    }
    fn members_of(&self, parent: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(self.members.get(parent).unwrap_or(&self.empty))
    }
    fn types_by_name(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
    }
    fn in_namespace(&self, _: &str) -> Vec<&ContractSymbol> {
        Vec::new()
    }
    fn has_in_namespace(&self, _: &str) -> bool {
        false
    }
    fn in_file(&self, _: &str) -> SymbolSet<'_> {
        SymbolSet::Borrowed(&self.empty)
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
    fn is_external_name(&self, name: &str, _: &str) -> bool {
        self.external_names.iter().any(|n| n == name)
    }
    fn is_declared_dependency(&self, package_id: Option<i64>, spec: &str) -> bool {
        self.declared_deps
            .iter()
            .any(|(p, s)| *p == package_id && s == spec)
    }
    fn resolve_module_from(&self, _: &str, _: &str) -> Option<&str> {
        self.module_link.as_deref()
    }
}

fn file_ctx(imports: Vec<ImportEntry>) -> FileContext {
    FileContext {
        file_path: "src/a.pas".to_string(),
        language: "pascal".to_string(),
        imports,
        file_namespace: None,
    }
}

fn named_import(name: &str) -> ImportEntry {
    ImportEntry {
        imported_name: name.to_string(),
        module_path: Some("./mod".to_string()),
        alias: None,
        is_wildcard: false,
    }
}

#[test]
fn matching_named_import_wins_over_everything() {
    let mut lookup = FakeLookup::default();
    lookup.by_name.insert("Foo".into(), vec![sym(7, "Foo", "m.Foo")]);
    lookup.external_names.push("Foo".into());
    let cause = classify_unbound_root("Foo", &[], &file_ctx(vec![named_import("Foo")]), &lookup, None);
    assert_eq!(cause.kind, CauseKind::ImportUnlinked);
    assert_eq!(cause.symbol_id, None);
}

#[test]
fn aliased_import_matches_on_bound_name_only() {
    let mut imp = named_import("Orig");
    imp.alias = Some("Bound".to_string());
    let lookup = FakeLookup::default();
    let ctx = file_ctx(vec![imp]);
    assert_eq!(
        classify_unbound_root("Bound", &[], &ctx, &lookup, None).kind,
        CauseKind::ImportUnlinked
    );
    assert_eq!(
        classify_unbound_root("Orig", &[], &ctx, &lookup, None).kind,
        CauseKind::NameUnknown
    );
}

#[test]
fn declared_unlinked_module_refines_to_declared_unsupplied() {
    let mut lookup = FakeLookup::default();
    lookup.declared_deps.push((None, "lodash".to_string()));
    let mut imp = named_import("debounce");
    imp.module_path = Some("lodash".to_string());
    let cause = classify_unbound_root("debounce", &[], &file_ctx(vec![imp]), &lookup, None);
    assert_eq!(cause.kind, CauseKind::ImportDeclaredUnsupplied);
    assert_eq!(cause.symbol_id, None);
}

#[test]
fn declared_probe_is_scoped_to_the_file_package() {
    let mut lookup = FakeLookup::default();
    lookup.declared_deps.push((Some(5), "lodash".to_string()));
    let mut imp = named_import("debounce");
    imp.module_path = Some("lodash".to_string());
    let ctx = file_ctx(vec![imp]);
    assert_eq!(
        classify_unbound_root("debounce", &[], &ctx, &lookup, Some(5)).kind,
        CauseKind::ImportDeclaredUnsupplied
    );
    assert_eq!(
        classify_unbound_root("debounce", &[], &ctx, &lookup, Some(6)).kind,
        CauseKind::ImportUnlinked
    );
}

#[test]
fn declared_module_that_links_to_a_file_stays_import_unlinked() {
    // Supply exists (the specifier resolves to an indexed file); the failure
    // is inside the link, not missing supply.
    let mut lookup = FakeLookup::default();
    lookup.declared_deps.push((None, "lodash".to_string()));
    lookup.module_link = Some("ext:ts:lodash/index.d.ts".to_string());
    let mut imp = named_import("debounce");
    imp.module_path = Some("lodash".to_string());
    assert_eq!(
        classify_unbound_root("debounce", &[], &file_ctx(vec![imp]), &lookup, None).kind,
        CauseKind::ImportUnlinked
    );
}

#[test]
fn wildcard_import_does_not_claim_the_name() {
    let mut imp = named_import("NS");
    imp.is_wildcard = true;
    let lookup = FakeLookup::default();
    assert_eq!(
        classify_unbound_root("NS", &[], &file_ctx(vec![imp]), &lookup, None).kind,
        CauseKind::NameUnknown
    );
}

#[test]
fn enclosing_scope_member_blames_the_member() {
    let mut lookup = FakeLookup::default();
    lookup
        .members
        .insert("TForm".into(), vec![sym(42, "Render", "TForm.Render")]);
    let scopes = vec!["TForm.Setup".to_string(), "TForm".to_string()];
    let cause = classify_unbound_root("Render", &scopes, &file_ctx(vec![]), &lookup, None);
    assert_eq!(cause.kind, CauseKind::ScopeMemberRoot);
    assert_eq!(cause.symbol_id, Some(42));
}

#[test]
fn external_known_beats_project_declarations() {
    let mut lookup = FakeLookup::default();
    lookup.external_names.push("useQuery".into());
    lookup
        .by_name
        .insert("useQuery".into(), vec![sym(9, "useQuery", "x.useQuery")]);
    assert_eq!(
        classify_unbound_root("useQuery", &[], &file_ctx(vec![]), &lookup, None).kind,
        CauseKind::ExternalKnownUnbound
    );
}

#[test]
fn unique_project_declaration_is_blamed() {
    let mut lookup = FakeLookup::default();
    lookup
        .by_name
        .insert("helper".into(), vec![sym(11, "helper", "util.helper")]);
    let cause = classify_unbound_root("helper", &[], &file_ctx(vec![]), &lookup, None);
    assert_eq!(cause.kind, CauseKind::DefinedUnimported);
    assert_eq!(cause.symbol_id, Some(11));
}

#[test]
fn ambiguous_project_declarations_blame_nothing() {
    let mut lookup = FakeLookup::default();
    lookup.by_name.insert(
        "init".into(),
        vec![sym(1, "init", "a.init"), sym(2, "init", "b.init")],
    );
    let cause = classify_unbound_root("init", &[], &file_ctx(vec![]), &lookup, None);
    assert_eq!(cause.kind, CauseKind::DefinedUnimported);
    assert_eq!(cause.symbol_id, None);
}

#[test]
fn nothing_anywhere_is_name_unknown() {
    let lookup = FakeLookup::default();
    let cause = classify_unbound_root("Integer", &[], &file_ctx(vec![]), &lookup, None);
    assert_eq!(cause.kind, CauseKind::NameUnknown);
    assert_eq!(cause.symbol_id, None);
}

// ---------------------------------------------------------------------------
// Chain-root classification: an untypable root is a value expression
// ---------------------------------------------------------------------------

fn value_sym(id: i64, name: &str, qname: &str, kind: &str, file: &str) -> ContractSymbol {
    let mut s = sym(id, name, qname);
    s.kind = kind.to_string();
    s.file_path = Arc::from(file);
    s
}

#[test]
fn untyped_root_blames_the_unique_same_file_value_binding() {
    let mut lookup = FakeLookup::default();
    lookup
        .by_name
        .insert("cfg".into(), vec![value_sym(9, "cfg", "a.run.cfg", "parameter", "src/a.pas")]);
    let cause = classify_untyped_root("cfg", &[], &file_ctx(vec![]), &lookup, None);
    assert_eq!(cause.kind, CauseKind::UntypedBinding);
    assert_eq!(cause.symbol_id, Some(9));
}

#[test]
fn untyped_root_field_binding_is_an_uncaptured_field() {
    let mut lookup = FakeLookup::default();
    lookup
        .by_name
        .insert("db".into(), vec![value_sym(4, "db", "a.Repo.db", "field", "src/a.pas")]);
    let cause = classify_untyped_root("db", &[], &file_ctx(vec![]), &lookup, None);
    assert_eq!(cause.kind, CauseKind::UncapturedField);
    assert_eq!(cause.symbol_id, Some(4));
}

#[test]
fn untyped_root_with_nothing_to_blame_is_untyped_root() {
    let lookup = FakeLookup::default();
    let cause = classify_untyped_root("local", &[], &file_ctx(vec![]), &lookup, None);
    assert_eq!(cause.kind, CauseKind::UntypedRoot);
    assert_eq!(cause.symbol_id, None);
}

/// Same-name declarations that are not same-file value bindings — a method in
/// this file, a variable elsewhere — do not turn a root into a name death.
#[test]
fn untyped_root_ignores_non_binding_and_foreign_candidates() {
    let mut lookup = FakeLookup::default();
    lookup.by_name.insert(
        "cfg".into(),
        vec![
            value_sym(1, "cfg", "a.cfg", "method", "src/a.pas"),
            value_sym(2, "cfg", "b.cfg", "variable", "src/b.pas"),
        ],
    );
    let cause = classify_untyped_root("cfg", &[], &file_ctx(vec![]), &lookup, None);
    assert_eq!(cause.kind, CauseKind::UntypedRoot);
    assert_eq!(cause.symbol_id, None);
}

/// The evidence probes precede the value floor for a chain root as for a bare
/// name: an import binding the root explains it better than "untyped".
#[test]
fn untyped_root_bound_by_an_import_is_import_unlinked() {
    let lookup = FakeLookup::default();
    let cause =
        classify_untyped_root("client", &[], &file_ctx(vec![named_import("client")]), &lookup, None);
    assert_eq!(cause.kind, CauseKind::ImportUnlinked);
}
