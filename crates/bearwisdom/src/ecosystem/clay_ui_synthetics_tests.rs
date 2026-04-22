// Tests for clay_ui_synthetics — in sibling file per feedback_tests_in_separate_files.md

use super::*;

#[test]
fn synthesized_file_parallel_vecs_consistent() {
    let pf = synthesize_file();
    assert_eq!(pf.symbols.len(), pf.symbol_origin_languages.len());
    assert_eq!(pf.symbols.len(), pf.symbol_from_snippet.len());
}

#[test]
fn int32_array_family_fully_expanded() {
    let pf = synthesize_file();
    let names: Vec<&str> = pf.symbols.iter().map(|s| s.name.as_str()).collect();

    // Core array struct
    assert!(names.contains(&"Clay__int32_tArray"), "Clay__int32_tArray struct must be synthesized");
    // Standard operations
    assert!(names.contains(&"Clay__int32_tArray_Add"));
    assert!(names.contains(&"Clay__int32_tArray_Get"));
    assert!(names.contains(&"Clay__int32_tArray_Allocate_Arena"));
    // int32_t-specific extras
    assert!(names.contains(&"Clay__int32_tArray_GetValue"));
    assert!(names.contains(&"Clay__int32_tArray_RemoveSwapback"));
    assert!(names.contains(&"Clay__int32_tArray_Set_DontTouchLength"));
    assert!(names.contains(&"Clay__int32_tArray_GetCheckCapacity"));
    assert!(names.contains(&"Clay__int32_tArray_Set"));
}

#[test]
fn clay_layout_element_array_family() {
    let pf = synthesize_file();
    let names: Vec<&str> = pf.symbols.iter().map(|s| s.name.as_str()).collect();

    assert!(names.contains(&"Clay_LayoutElementArray"));
    assert!(names.contains(&"Clay_LayoutElementArray_Add"));
    assert!(names.contains(&"Clay_LayoutElementArray_Get"));
    assert!(names.contains(&"Clay_LayoutElementArray_Allocate_Arena"));
    assert!(names.contains(&"Clay_LayoutElementArray_GetCheckCapacity"));
    assert!(names.contains(&"Clay_LayoutElementArray_Set_DontTouchLength"));
}

#[test]
fn wrapped_text_line_slice_variant() {
    let pf = synthesize_file();
    let names: Vec<&str> = pf.symbols.iter().map(|s| s.name.as_str()).collect();

    assert!(names.contains(&"Clay__WrappedTextLineArray"));
    assert!(names.contains(&"Clay__WrappedTextLineArraySlice"), "slice struct must be synthesized");
    assert!(names.contains(&"Clay__WrappedTextLineArraySlice_Get"), "slice _Get must be synthesized");
}

#[test]
fn clay_enum_constants_present() {
    let pf = synthesize_file();
    let names: Vec<&str> = pf.symbols.iter().map(|s| s.name.as_str()).collect();

    for expected in [
        "CLAY_ALIGN_X_LEFT",
        "CLAY_ALIGN_Y_TOP",
        "CLAY_ATTACH_POINT_LEFT_TOP",
        "CLAY_ATTACH_TO_NONE",
        "CLAY_TEXT_WRAP_WORDS",
        "CLAY_TEXT_ALIGN_CENTER",
        "CLAY_LEFT_TO_RIGHT",
        "CLAY__SIZING_TYPE_FIT",
        "CLAY_POINTER_CAPTURE_MODE_CAPTURE",
    ] {
        assert!(names.contains(&expected), "{expected} enum constant must be synthesized");
    }
}

#[test]
fn clay_enum_constants_are_enum_variant_kind() {
    let pf = synthesize_file();
    for sym in &pf.symbols {
        if sym.name.starts_with("CLAY_") {
            assert_eq!(
                sym.kind,
                crate::types::SymbolKind::EnumMember,
                "{} should be EnumVariant",
                sym.name
            );
        }
    }
}

#[test]
fn array_structs_are_struct_kind() {
    let pf = synthesize_file();
    let struct_syms: Vec<&str> = pf
        .symbols
        .iter()
        .filter(|s| s.kind == crate::types::SymbolKind::Struct)
        .map(|s| s.name.as_str())
        .collect();

    assert!(struct_syms.contains(&"Clay__int32_tArray"));
    assert!(struct_syms.contains(&"Clay_LayoutElementArray"));
    assert!(struct_syms.contains(&"Clay_RectangleElementConfig"));
}

#[test]
fn internal_functions_present() {
    let pf = synthesize_file();
    let names: Vec<&str> = pf.symbols.iter().map(|s| s.name.as_str()).collect();

    assert!(names.contains(&"Clay__RenderDebugView"));
    assert!(names.contains(&"Clay__RenderDebugViewColor"));
    assert!(names.contains(&"Clay__RenderElementConfigTypeLabel"));
    assert!(names.contains(&"Clay__DebugViewRenderElementConfigHeader"));
}

#[test]
fn activation_covers_c_and_cpp() {
    let eco = ClayUiSyntheticsEcosystem;
    assert!(eco.languages().contains(&"c"));
    assert!(eco.languages().contains(&"cpp"));
    assert_eq!(eco.kind(), crate::ecosystem::EcosystemKind::Stdlib);
    assert!(eco.uses_demand_driven_parse());
}

#[test]
fn no_clay_header_returns_empty_roots() {
    use std::path::Path;
    // A temp-dir with no clay.h — locate_roots should return empty.
    let tmp = std::env::temp_dir();
    let eco = ClayUiSyntheticsEcosystem;
    let roots = ExternalSourceLocator::locate_roots(&eco, &tmp);
    assert!(
        roots.is_empty(),
        "locate_roots must return empty for projects without clay.h"
    );
}

#[test]
fn clay_header_present_returns_root() {
    use std::io::Write as _;
    let dir = tempfile::tempdir().expect("tempdir");
    let clay_h = dir.path().join("clay.h");
    std::fs::File::create(&clay_h)
        .unwrap()
        .write_all(b"/* synthetic clay.h */")
        .unwrap();
    let eco = ClayUiSyntheticsEcosystem;
    let roots = ExternalSourceLocator::locate_roots(&eco, dir.path());
    assert_eq!(roots.len(), 1, "locate_roots must return the synthetic dep root when clay.h exists");
    let parsed = ExternalSourceLocator::parse_metadata_only(&eco, dir.path());
    assert!(parsed.is_some());
}
