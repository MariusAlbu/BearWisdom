// Attribute-blob decoding and name projection over synthetic ECMA-335
// fixtures: a CompilationSourceName value blob yields the source name, a
// ModuleSuffix flag strips the compiled `Module` tail, a CompilationMapping
// value classifies a module.

use super::*;

/// `[<CompiledName("Map")>] let map …` emits `CompilationSourceNameAttribute`
/// whose single fixed argument is the source name.
#[test]
fn source_name_blob_decodes_to_source_name() {
    // prolog 0x0001, SerString len 3 "map", zero named args.
    let blob = [0x01, 0x00, 0x03, b'm', b'a', b'p', 0x00, 0x00];
    assert_eq!(decode_string_arg(&blob).as_deref(), Some("map"));
}

#[test]
fn source_name_blob_without_prolog_declines() {
    let blob = [0x00, 0x02, 0x03, b'm', b'a', b'p'];
    assert_eq!(decode_string_arg(&blob), None);
}

#[test]
fn null_ser_string_declines() {
    let blob = [0x01, 0x00, 0xFF, 0x00, 0x00];
    assert_eq!(decode_string_arg(&blob), None);
}

#[test]
fn two_byte_packed_length_decodes() {
    // 0x81 0x30 → (0x01 << 8) | 0x30 = 304.
    assert_eq!(read_packed_len(&[0x81, 0x30]), Some((304, 2)));
    assert_eq!(read_packed_len(&[0x7F]), Some((127, 1)));
}

#[test]
fn representation_blob_decodes_module_suffix_flag() {
    // CompilationRepresentationFlags.ModuleSuffix = 4, i32 little-endian.
    let blob = [0x01, 0x00, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00];
    let flags = decode_i32_arg(&blob).expect("i32 arg");
    assert_ne!(flags & MODULE_SUFFIX_FLAG, 0);
}

#[test]
fn mapping_blob_classifies_module_kind() {
    // SourceConstructFlags.Module = 7, i32 little-endian.
    let blob = [0x01, 0x00, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00];
    let flags = decode_i32_arg(&blob).expect("i32 arg");
    assert_eq!(flags & SOURCE_CONSTRUCT_KIND_MASK, SOURCE_CONSTRUCT_MODULE);
}

#[test]
fn suffixed_type_projects_to_unsuffixed_source_name() {
    let mut p = SourceNameProjection::default();
    p.suffixed_types.insert(0x0200_0001);
    assert_eq!(p.type_name(0x0200_0001, "ListModule"), "List");
    // A type without the flag keeps its IL name even when `Module`-suffixed.
    assert_eq!(p.type_name(0x0200_0002, "ListModule"), "ListModule");
    // A name that is nothing but the suffix cannot strip to empty.
    assert_eq!(p.type_name(0x0200_0001, "Module"), "Module");
}

#[test]
fn method_projects_through_source_name_map() {
    let mut p = SourceNameProjection::default();
    p.method_names.insert(0x0600_0001, "map".to_string());
    assert_eq!(p.method_name(0x0600_0001, "Map"), "map");
    assert_eq!(p.method_name(0x0600_0002, "Map"), "Map");
}

#[test]
fn module_kind_membership() {
    let mut p = SourceNameProjection::default();
    p.module_types.insert(0x0200_0009);
    assert!(p.is_module(0x0200_0009));
    assert!(!p.is_module(0x0200_000A));
}
