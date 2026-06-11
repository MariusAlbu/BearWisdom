// =============================================================================
// hcl/profile_tests.rs — profile-axis and full-ladder bind tests.
//
// Terraform `var`/`local`/resource names are module-flat: every `.tf` in a
// directory shares one namespace, so a `var.X` ref binds cross-file. The
// `var`/`local` sigil head is a self-keyword the bare-name rung strips, then
// `namespaceless_global_type_lookup == Global` first-match-binds the project's
// own `X`; a same-named ext: stub declines and stays external.
// =============================================================================

use super::HCL_PROFILE;
use crate::indexer::resolve::engine::{FileContext, RefContext, Resolution, SymbolIndex};
use crate::type_checker::core::DefaultResolver;
use crate::type_checker::profile::language_profile::{HeadAliasBind, NamespaceScope};
use crate::types::*;
use std::collections::HashMap;

#[test]
fn hcl_profile_carries_resolution_data() {
    assert_eq!(HCL_PROFILE.id, "hcl");
    // `var.X` / `local.X` heads are stripped by the bare-name probes.
    assert_eq!(HCL_PROFILE.self_keywords, &["var", "local"]);
    // Terraform meta-references decline before the ladder.
    let skip = HCL_PROFILE.builtin_skip.expect("builtin_skip set");
    assert!(skip("each.value"));
    assert!(skip("count.index"));
    assert!(!skip("aws_instance.web"));
    // Provider-alias heads bind to an in-file `provider` class.
    assert_eq!(
        HCL_PROFILE.head_alias,
        HeadAliasBind::OnSameFile {
            require_kind: Some("class"),
        }
    );
}

#[test]
fn hcl_namespaceless_global_is_on() {
    // Terraform `var`/`local`/resource names are module-flat, so a `var.X` ref
    // binds via the dead-last first-match-by-name rung.
    assert_eq!(
        HCL_PROFILE.namespaceless_global_type_lookup,
        NamespaceScope::Global
    );
}

// ---------------------------------------------------------------------------
// Full-ladder bind tests through resolve_all_with_profile(&HCL_PROFILE).
// ---------------------------------------------------------------------------

fn accept_any(_edge: EdgeKind, _sym_kind: &str) -> bool {
    true
}

fn make_sym(name: &str, kind: SymbolKind) -> ExtractedSymbol {
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name: name.to_string(),
        kind,
        visibility: Some(Visibility::Public),
        start_line: 1,
        end_line: 5,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

fn make_ref(target: &str, kind: EdgeKind) -> ExtractedRef {
    ExtractedRef {
        is_import_binding: false,
        is_reexport: false,
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind,
        line: 2,
        col: 0,
        module: None,
        chain: None,
        byte_offset: 1,
        namespace_segments: Vec::new(),
        call_args: Vec::new(),
    }
}

fn make_file(path: &str, symbols: Vec<ExtractedSymbol>, refs: Vec<ExtractedRef>) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "hcl".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 10,
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
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

fn clone_pf(f: &ParsedFile) -> ParsedFile {
    make_file(&f.path, f.symbols.clone(), f.refs.clone())
}

fn build_index(files: &[&ParsedFile]) -> (SymbolIndex, HashMap<(String, String), i64>) {
    let mut id_map = HashMap::new();
    let mut next_id = 1i64;
    for pf in files {
        for sym in &pf.symbols {
            id_map.insert((pf.path.clone(), sym.qualified_name.clone()), next_id);
            next_id += 1;
        }
    }
    let owned: Vec<ParsedFile> = files.iter().map(|f| clone_pf(f)).collect();
    let index = SymbolIndex::build(&owned, &id_map);
    (index, id_map)
}

fn sym_id(id_map: &HashMap<(String, String), i64>, file: &str, name: &str) -> i64 {
    *id_map
        .get(&(file.to_string(), name.to_string()))
        .unwrap_or_else(|| panic!("symbol not found: {file}::{name}"))
}

fn resolve_ref(file_path: &str, target: &str, kind: EdgeKind, all: &[&ParsedFile]) -> Option<Resolution> {
    let (index, _) = build_index(all);
    let caller = make_file(
        file_path,
        vec![make_sym("caller", SymbolKind::Function)],
        vec![make_ref(target, kind)],
    );
    let file_ctx = FileContext {
        file_path: file_path.to_string(),
        language: "hcl".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    let r = &caller.refs[0];
    let ref_ctx = RefContext {
        extracted_ref: r,
        source_symbol: &caller.symbols[0],
        scope_chain: vec![],
        file_package_id: None,
    };
    DefaultResolver {
        file_ctx: &file_ctx,
        ref_ctx: &ref_ctx,
        lookup: &index,
        kind_compatible: accept_any,
    }
    .resolve_all_with_profile(&HCL_PROFILE)
}

#[test]
fn hcl_var_ref_binds_internal_over_external() {
    // `app_name` is the project's own variable (declared in two sibling `.tf`
    // files) plus a same-named ext: module stub. A `var.app_name` ref strips
    // the `var` sigil and binds an INTERNAL declaration first-match; the
    // external stub loses.
    let a = make_file(
        "infra/variables.tf",
        vec![make_sym("app_name", SymbolKind::Variable)],
        vec![],
    );
    let b = make_file(
        "infra/locals.tf",
        vec![make_sym("app_name", SymbolKind::Variable)],
        vec![],
    );
    let ext = make_file(
        "ext:hcl:module/variables.tf",
        vec![make_sym("app_name", SymbolKind::Variable)],
        vec![],
    );
    let (id_a, id_b, ext_id) = {
        let (_, id_map) = build_index(&[&a, &b, &ext]);
        (
            sym_id(&id_map, "infra/variables.tf", "app_name"),
            sym_id(&id_map, "infra/locals.tf", "app_name"),
            sym_id(&id_map, "ext:hcl:module/variables.tf", "app_name"),
        )
    };
    let res = resolve_ref("infra/main.tf", "var.app_name", EdgeKind::TypeRef, &[&a, &b, &ext])
        .expect("var ref strips the sigil and binds an internal variable");
    assert_eq!(res.strategy, "default_namespaceless_global");
    assert_ne!(res.target_symbol_id, ext_id, "must not bind the ext module stub");
    assert!(
        res.target_symbol_id == id_a || res.target_symbol_id == id_b,
        "binds an internal app_name (got {})",
        res.target_symbol_id
    );
}

#[test]
fn hcl_external_only_var_stays_unresolved() {
    // `region` is owned ONLY by an external module file — no project
    // declaration. The internal-only rung declines, leaving it for external
    // classification.
    let ext = make_file(
        "ext:hcl:module/variables.tf",
        vec![make_sym("region", SymbolKind::Variable)],
        vec![],
    );
    let res = resolve_ref("infra/main.tf", "var.region", EdgeKind::TypeRef, &[&ext]);
    assert!(
        res.is_none(),
        "external-only var must not bind an internal symbol; got: {res:?}"
    );
}
