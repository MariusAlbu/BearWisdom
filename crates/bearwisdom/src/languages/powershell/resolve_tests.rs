// =============================================================================
// languages/powershell/resolve_tests.rs — Unit tests for PowerShell resolver
//
// Tests for .NET local-variable type binding (Parts 1/2/3).
// Per project convention, #[cfg(test)] blocks for ecosystem/indexer modules
// live in a separate <module>_tests.rs file.
// =============================================================================

use super::extract::{
    is_dotnet_type_name, try_parse_cmdlet_result_chain, try_parse_new_object,
    try_parse_propagation, try_parse_type_new, try_parse_typed_param, DOTNET_BINDING_SENTINEL,
};

// ---------------------------------------------------------------------------
// try_parse_new_object — Pattern 1
// ---------------------------------------------------------------------------

#[test]
fn test_new_object_simple() {
    let b = try_parse_new_object("$border = New-Object Windows.Controls.Border");
    assert!(b.is_some());
    let (var, ty) = b.unwrap();
    assert_eq!(var, "border");
    assert_eq!(ty, "Windows.Controls.Border");
}

#[test]
fn test_new_object_system_namespace() {
    let b = try_parse_new_object("$packages = New-Object System.Collections.Hashtable");
    assert!(b.is_some());
    let (var, ty) = b.unwrap();
    assert_eq!(var, "packages");
    assert_eq!(ty, "System.Collections.Hashtable");
}

#[test]
fn test_new_object_typename_flag() {
    let b = try_parse_new_object("$grid = New-Object -TypeName Windows.Controls.Grid");
    assert!(b.is_some());
    let (var, ty) = b.unwrap();
    assert_eq!(var, "grid");
    assert_eq!(ty, "Windows.Controls.Grid");
}

#[test]
fn test_new_object_with_constructor_arg() {
    // `New-Object Windows.CornerRadius(10)` — the `(10)` should be stripped.
    let b = try_parse_new_object("$cr = New-Object Windows.CornerRadius(10)");
    assert!(b.is_some());
    let (var, ty) = b.unwrap();
    assert_eq!(var, "cr");
    assert_eq!(ty, "Windows.CornerRadius");
}

#[test]
fn test_new_object_bare_name_not_dotnet() {
    // PSObject has no dot → not a .NET framework type.
    let b = try_parse_new_object("$obj = New-Object PSObject");
    if let Some((_, ty)) = b {
        assert!(!is_dotnet_type_name(&ty), "bare PSObject should not be a dotnet type");
    }
}

#[test]
fn test_new_object_none_for_non_new_object() {
    assert!(try_parse_new_object("$x = Get-Item foo").is_none());
    assert!(try_parse_new_object("$x = [Type]::new()").is_none());
}

// ---------------------------------------------------------------------------
// try_parse_type_new — Pattern 2
// ---------------------------------------------------------------------------

#[test]
fn test_type_new_hashtable() {
    let b = try_parse_type_new("$packages = [System.Collections.Hashtable]::new()");
    assert!(b.is_some());
    let (var, ty) = b.unwrap();
    assert_eq!(var, "packages");
    assert_eq!(ty, "System.Collections.Hashtable");
}

#[test]
fn test_type_new_arraylist() {
    let b = try_parse_type_new("$packagesWinget = [System.Collections.ArrayList]::new()");
    assert!(b.is_some());
    let (var, ty) = b.unwrap();
    assert_eq!(var, "packagesWinget");
    assert_eq!(ty, "System.Collections.ArrayList");
}

#[test]
fn test_type_new_generic_list() {
    // Generic type: [System.Collections.Generic.List[string]]::new()
    // Type args are stripped → base type is stored.
    let b = try_parse_type_new("$script_content = [System.Collections.Generic.List[string]]::new()");
    assert!(b.is_some());
    let (var, ty) = b.unwrap();
    assert_eq!(var, "script_content");
    assert_eq!(ty, "System.Collections.Generic.List");
}

#[test]
fn test_type_new_string_no_dot() {
    // `[string]::new()` — `string` has no dot, is_dotnet_type_name returns false.
    let b = try_parse_type_new("$s = [string]::new()");
    if let Some((_, ty)) = b {
        assert!(!is_dotnet_type_name(&ty));
    }
}

#[test]
fn test_type_new_none_for_missing_new() {
    assert!(try_parse_type_new("$x = [Type]::SomeStaticMethod()").is_none());
}

// ---------------------------------------------------------------------------
// try_parse_typed_param — Pattern 3
// ---------------------------------------------------------------------------

#[test]
fn test_typed_param_wrappanel() {
    let b = try_parse_typed_param("    [Windows.Controls.WrapPanel]$TargetElement,");
    // trim() is done in the caller (emit_dotnet_binding_sentinels), not here
    // — test the trimmed form.
    let b2 = try_parse_typed_param("[Windows.Controls.WrapPanel]$TargetElement,");
    assert!(b2.is_some());
    let (var, ty) = b2.unwrap();
    assert_eq!(var, "TargetElement");
    assert_eq!(ty, "Windows.Controls.WrapPanel");
    // Non-trimmed should return None (starts with space, not `[`).
    assert!(b.is_none());
}

#[test]
fn test_typed_param_string_not_dotnet() {
    let b = try_parse_typed_param("[string]$SearchString");
    if let Some((_, ty)) = b {
        assert!(!is_dotnet_type_name(&ty));
    }
}

#[test]
fn test_typed_param_system_namespace() {
    let b = try_parse_typed_param("[System.Windows.Window]$window = $null");
    assert!(b.is_some());
    let (var, ty) = b.unwrap();
    assert_eq!(var, "window");
    assert_eq!(ty, "System.Windows.Window");
}

// ---------------------------------------------------------------------------
// is_dotnet_type_name
// ---------------------------------------------------------------------------

#[test]
fn test_is_dotnet_type_name_windows() {
    assert!(is_dotnet_type_name("Windows.Controls.Border"));
    assert!(is_dotnet_type_name("System.Collections.Hashtable"));
    assert!(is_dotnet_type_name("Microsoft.Win32.Registry"));
}

#[test]
fn test_is_dotnet_type_name_rejects_bare() {
    assert!(!is_dotnet_type_name("PSObject"));
    assert!(!is_dotnet_type_name("string"));
    assert!(!is_dotnet_type_name("ArrayList"));
}

#[test]
fn test_is_dotnet_type_name_accepts_generic_syntax() {
    // Generic types like List[string] are now accepted — the generic args are
    // stripped before checking the namespace root.
    assert!(is_dotnet_type_name("System.Collections.Generic.List[string]"));
    assert!(is_dotnet_type_name("System.Collections.Generic.Dictionary<string,int>"));
}

// ---------------------------------------------------------------------------
// sentinel constant
// ---------------------------------------------------------------------------

#[test]
fn test_sentinel_constant() {
    assert_eq!(DOTNET_BINDING_SENTINEL, "dotnet-stdlib");
}

// ---------------------------------------------------------------------------
// infer_external_namespace — integration via FileContext
// ---------------------------------------------------------------------------

use crate::indexer::resolve::engine::{FileContext, ImportEntry, RefContext, LanguageResolver};
use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};
use super::resolve::PowerShellResolver;

fn make_file_ctx_with_binding(var_name: &str) -> FileContext {
    FileContext {
        file_path: "test.ps1".to_string(),
        language: "powershell".to_string(),
        imports: vec![ImportEntry {
            imported_name: var_name.to_string(),
            module_path: Some(DOTNET_BINDING_SENTINEL.to_string()),
            alias: None,
            is_wildcard: false,
        }],
        file_namespace: None,
    }
}

fn make_member_ref(target: &str, module: &str, kind: EdgeKind) -> ExtractedRef {
    ExtractedRef {
        source_symbol_index: 0,
        target_name: target.to_string(),
        kind,
        line: 5,
        module: Some(module.to_string()),
        chain: None,
        byte_offset: 0,
    }
}

fn make_source_sym() -> ExtractedSymbol {
    ExtractedSymbol {
        name: "TestFn".to_string(),
        qualified_name: "TestFn".to_string(),
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 20,
        start_col: 0,
        end_col: 0,
        signature: None,
        doc_comment: None,
        scope_path: None,
        parent_index: None,
    }
}

#[test]
fn test_infer_external_ns_dotnet_property() {
    let resolver = PowerShellResolver;
    let file_ctx = make_file_ctx_with_binding("border");
    let r = make_member_ref("Style", "border", EdgeKind::TypeRef);
    let sym = make_source_sym();
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
    assert_eq!(ns, Some("dotnet-stdlib".to_string()));
}

#[test]
fn test_infer_external_ns_dotnet_method() {
    let resolver = PowerShellResolver;
    let file_ctx = make_file_ctx_with_binding("border");
    let r = make_member_ref("Add_MouseLeftButtonUp", "border", EdgeKind::Calls);
    let sym = make_source_sym();
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
    assert_eq!(ns, Some("dotnet-stdlib".to_string()));
}

#[test]
fn test_infer_external_ns_unbound_var() {
    let resolver = PowerShellResolver;
    let file_ctx = make_file_ctx_with_binding("border");
    // `sync` is NOT bound to a .NET type.
    let r = make_member_ref("Form", "sync", EdgeKind::TypeRef);
    let sym = make_source_sym();
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
    assert_ne!(ns, Some("dotnet-stdlib".to_string()));
}

#[test]
fn test_infer_external_ns_cmdlet_no_module() {
    let resolver = PowerShellResolver;
    let file_ctx = FileContext {
        file_path: "test.ps1".to_string(),
        language: "powershell".to_string(),
        imports: vec![],
        file_namespace: None,
    };
    // Write-Host with no module — hits cmdlet branch.
    let r = ExtractedRef {
        source_symbol_index: 0,
        target_name: "Write-Host".to_string(),
        kind: EdgeKind::Calls,
        line: 1,
        module: None,
        chain: None,
        byte_offset: 0,
    };
    let sym = make_source_sym();
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
    assert_eq!(ns, Some("powershell-stdlib".to_string()));
}

// ===========================================================================
// Pass 2 — Part 1: hashtable-indexer registry ($sync["Key"].Member)
// ===========================================================================

/// Binding for registry var `sync` → is_dotnet_bound_var("sync") should be true.
#[test]
fn test_part1_sync_registry_var_classifies_as_dotnet() {
    let resolver = PowerShellResolver;
    let file_ctx = make_file_ctx_with_binding("sync");
    // Ref: `$sync["WPFKey"].Dispatcher` → module="sync", target="Dispatcher"
    let r = make_member_ref("Dispatcher", "sync", EdgeKind::TypeRef);
    let sym = make_source_sym();
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
    assert_eq!(ns, Some("dotnet-stdlib".to_string()));
}

#[test]
fn test_part1_sync_invoke_classifies_as_dotnet() {
    let resolver = PowerShellResolver;
    let file_ctx = make_file_ctx_with_binding("sync");
    let r = make_member_ref("Invoke", "sync", EdgeKind::Calls);
    let sym = make_source_sym();
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
    assert_eq!(ns, Some("dotnet-stdlib".to_string()));
}

#[test]
fn test_part1_sync_text_visibility_findname() {
    let resolver = PowerShellResolver;
    let file_ctx = make_file_ctx_with_binding("sync");
    let sym = make_source_sym();
    for name in &["Text", "Visibility", "FindName", "IsChecked", "Count"] {
        let r = make_member_ref(name, "sync", EdgeKind::TypeRef);
        let ref_ctx = RefContext {
            extracted_ref: &r,
            source_symbol: &sym,
            scope_chain: vec![],
            file_package_id: None,
        };
        let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
        assert_eq!(
            ns,
            Some("dotnet-stdlib".to_string()),
            "{name} on sync should be dotnet-stdlib",
        );
    }
}

// ===========================================================================
// Pass 2 — Part 2: pipeline variable `$_`
// ===========================================================================

#[test]
fn test_part2_pipeline_var_visibility_classifies_as_dotnet() {
    let resolver = PowerShellResolver;
    let file_ctx = make_file_ctx_with_binding("_");
    let r = make_member_ref("Visibility", "_", EdgeKind::TypeRef);
    let sym = make_source_sym();
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
    assert_eq!(ns, Some("dotnet-stdlib".to_string()));
}

#[test]
fn test_part2_pipeline_var_text_classifies_as_dotnet() {
    let resolver = PowerShellResolver;
    let file_ctx = make_file_ctx_with_binding("_");
    let r = make_member_ref("Text", "_", EdgeKind::TypeRef);
    let sym = make_source_sym();
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
    assert_eq!(ns, Some("dotnet-stdlib".to_string()));
}

// ===========================================================================
// Pass 2 — Part 3: cmdlet-result chain ((Get-Date).ToString())
// ===========================================================================

#[test]
fn test_part3_try_parse_cmdlet_result_chain_get_date() {
    let tag = try_parse_cmdlet_result_chain("    $ts = (Get-Date).ToString(\"HH:mm:ss\")");
    assert_eq!(tag, Some("__cmdlet_get_date".to_string()));
}

#[test]
fn test_part3_try_parse_cmdlet_result_chain_get_childitem() {
    let tag = try_parse_cmdlet_result_chain("Get-ChildItem . | ForEach-Object { $_.FullName }");
    // No `(Get-ChildItem).` pattern on this line — cmdlet not wrapped in parens.
    // Should return None.
    assert_eq!(tag, None);
}

#[test]
fn test_part3_try_parse_cmdlet_result_chain_parenthesized() {
    let tag = try_parse_cmdlet_result_chain(
        "    if ((Get-ChildItem \".\").IsReadOnly) {",
    );
    assert_eq!(tag, Some("__cmdlet_get_childitem".to_string()));
}

#[test]
fn test_part3_infer_external_ns_cmdlet_result() {
    use crate::ecosystem::powershell_cmdlet_types::cmdlet_result_module_tag;
    let resolver = PowerShellResolver;
    let tag = cmdlet_result_module_tag("Get-Date");
    let file_ctx = make_file_ctx_with_binding(&tag);
    let r = make_member_ref("ToString", &tag, EdgeKind::Calls);
    let sym = make_source_sym();
    let ref_ctx = RefContext {
        extracted_ref: &r,
        source_symbol: &sym,
        scope_chain: vec![],
        file_package_id: None,
    };
    let ns = resolver.infer_external_namespace(&file_ctx, &ref_ctx, None);
    assert_eq!(ns, Some("dotnet-stdlib".to_string()));
}

#[test]
fn test_part3_unknown_cmdlet_returns_none() {
    // `Write-Host` is not in the type table — no sentinel emitted.
    let tag = try_parse_cmdlet_result_chain("(Write-Host).Something");
    assert_eq!(tag, None);
}

// ===========================================================================
// Part 4 — propagation through member/index access
// ===========================================================================

#[test]
fn propagation_member_access() {
    let p = try_parse_propagation("$Tweaks = $sync.selectedTweaks");
    assert_eq!(p, Some(("Tweaks".to_string(), "sync".to_string())));
}

#[test]
fn propagation_index_access() {
    let p = try_parse_propagation("$dns = $sync[\"WPFchangedns\"].text");
    assert_eq!(p, Some(("dns".to_string(), "sync".to_string())));
}

#[test]
fn propagation_scope_prefix() {
    let p = try_parse_propagation("$script:list = $store.entries");
    assert_eq!(p, Some(("list".to_string(), "store".to_string())));
}

#[test]
fn propagation_rejects_plain_copy() {
    // `$a = $b` without member access carries no type info; skip.
    assert_eq!(try_parse_propagation("$a = $b"), None);
}

#[test]
fn propagation_rejects_non_var_rhs() {
    assert_eq!(try_parse_propagation("$a = Get-Something"), None);
    assert_eq!(try_parse_propagation("$a = 42.Foo"), None);
}

#[test]
fn propagation_rejects_equality() {
    assert_eq!(try_parse_propagation("$a == $b.Foo"), None);
}

#[test]
fn propagation_from_registry_binds_lhs() {
    // Full end-to-end: scan a source snippet where `$Tweaks = $sync.foo`, then
    // verify `Tweaks` shows up as a .NET-bound var via the emitted sentinels.
    use crate::languages::powershell::extract::extract;
    let src = r#"
function Invoke-X {
    $Tweaks = $sync.selectedTweaks
    $Tweaks.Count
}
"#;
    let result = extract(src);
    let bound: Vec<_> = result
        .refs
        .iter()
        .filter(|r| r.target_name == DOTNET_BINDING_SENTINEL)
        .filter_map(|r| r.module.clone())
        .collect();
    assert!(bound.contains(&"sync".to_string()), "sync registry binding missing; got {bound:?}");
    assert!(bound.contains(&"Tweaks".to_string()),
        "Tweaks should inherit binding from $sync.selectedTweaks; got {bound:?}");
}
