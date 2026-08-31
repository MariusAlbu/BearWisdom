// =============================================================================
// nuget/type_symbols.rs — ExtractedSymbols for one ECMA-335 type definition.
//
// One emission path shared by the eager whole-DLL parse and the demanded
// single-type crack: the type row, its public methods (source-name
// projected), Property rows recovered from get_/set_ accessors, and public
// static fields. Base-class / interface refs ride along so the inheritance
// map can climb DLL types.
// =============================================================================

use dotscope::metadata::method::MethodAccessFlags;
use dotscope::metadata::typesystem::CilType;
use dotscope::prelude::CilObject;

use crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};

use super::clr_projection::SourceNameProjection;
use super::signature_format::{
    format_generic_suffix, format_method_signature, strip_backtick_arity,
};
use super::type_qname::qualified_type_name;

/// FieldAttributes (ECMA-335 §II.23.1.5).
const FIELD_ACCESS_MASK: u32 = 0x7;
const FIELD_PUBLIC: u32 = 0x6;
const FIELD_STATIC: u32 = 0x10;
const FIELD_SPECIAL_NAME: u32 = 0x200;

/// ECMA-335 TypeAttributes: `Abstract` | `Sealed` together mark a STATIC
/// class — the only container extension methods live in, and the compiled
/// shape of a source-level module.
pub(super) fn is_static_class(type_def: &CilType) -> bool {
    type_def.flags & 0x80 != 0 && type_def.flags & 0x100 != 0
}

/// TypeAttributes visibility: public or nested-public.
pub(super) fn is_public_type(type_def: &CilType) -> bool {
    let mask = type_def.flags & 0x07;
    mask == 1 || mask == 2
}

/// `(display_name, qualified_name, scope_path)` for one type row under
/// source-name projection: arity stripped, module suffix shed.
pub(super) fn projected_type_identity(
    type_def: &CilType,
    projection: &SourceNameProjection,
) -> (String, String, Option<String>) {
    let stripped = strip_backtick_arity(&type_def.name);
    let display = projection
        .type_name(type_def.token.value(), stripped)
        .to_string();
    let (qualified, scope_path) = qualified_type_name(type_def, &display);
    (display, qualified, scope_path)
}

/// Emit the symbol rows for one public type definition: the type itself, its
/// public methods, accessor-derived Property rows, and public static Field
/// rows — all parented to the type — plus Inherits/Implements refs for its
/// supertype list.
pub(super) fn emit_type_symbols(
    type_def: &CilType,
    assembly: &CilObject,
    projection: &SourceNameProjection,
    symbols: &mut Vec<ExtractedSymbol>,
    refs: &mut Vec<ExtractedRef>,
) {
    let (display, qualified, scope_path) = projected_type_identity(type_def, projection);
    let is_static = is_static_class(type_def);
    let is_interface = type_def.flags & 0x20 != 0;
    let kind = if is_interface {
        SymbolKind::Interface
    } else {
        SymbolKind::Class
    };
    let type_generic_names: Vec<String> = type_def
        .generic_params
        .iter()
        .map(|(_, gp)| gp.name.clone())
        .collect();
    let type_gp_suffix = format_generic_suffix(&type_generic_names);
    let type_sym_idx = symbols.len();
    emit_supertype_refs(type_def, type_sym_idx, refs);
    symbols.push(bare_symbol(
        display.clone(),
        qualified.clone(),
        kind,
        Some(format!(
            "{} {}{}",
            if is_interface { "interface" } else { "class" },
            display,
            type_gp_suffix
        )),
        scope_path,
        None,
    ));

    // Ordered first-seen property names recovered from accessor methods.
    let mut properties: Vec<String> = Vec::new();
    for (_, method_ref) in type_def.methods.iter() {
        let Some(method) = method_ref.upgrade() else {
            continue;
        };
        if method.name.starts_with('<') || method.name.starts_with('.') {
            continue;
        }
        if method.flags_access != MethodAccessFlags::PUBLIC {
            continue;
        }
        if let Some(prop) = accessor_property_name(&method.name) {
            if !properties.iter().any(|p| p == prop) {
                properties.push(prop.to_string());
            }
        }
        let method_name = projection
            .method_name(method.token.value(), &method.name)
            .to_string();
        let method_generic_names: Vec<String> = method
            .generic_params
            .iter()
            .map(|(_, gp)| gp.name.clone())
            .collect();
        let is_extension_candidate = is_static
            && method
                .flags_modifiers
                .contains(dotscope::metadata::method::MethodModifiers::STATIC)
            && !method.signature.params.is_empty();
        let signature = format_method_signature(
            &method_name,
            &method.signature,
            &type_generic_names,
            &method_generic_names,
            assembly,
            is_extension_candidate,
        );
        symbols.push(bare_symbol(
            method_name.clone(),
            format!("{qualified}.{method_name}"),
            SymbolKind::Method,
            Some(signature),
            Some(qualified.clone()),
            Some(type_sym_idx),
        ));
    }
    for prop in properties {
        symbols.push(bare_symbol(
            prop.clone(),
            format!("{qualified}.{prop}"),
            SymbolKind::Property,
            None,
            Some(qualified.clone()),
            Some(type_sym_idx),
        ));
    }
    emit_static_fields(type_def, &qualified, type_sym_idx, symbols);
}

/// Property name behind a `get_` / `set_` accessor's IL name.
pub(super) fn accessor_property_name(method_name: &str) -> Option<&str> {
    let rest = method_name
        .strip_prefix("get_")
        .or_else(|| method_name.strip_prefix("set_"))?;
    if rest.is_empty() {
        None
    } else {
        Some(rest)
    }
}

/// Public static fields become Field rows parented to the type — value-type
/// wells (`TimeSpan.Zero`) and enum members are reached this way. Special-name
/// rows (`value__`) are runtime bookkeeping, not API surface.
pub(super) fn emit_static_fields(
    type_def: &CilType,
    qualified: &str,
    type_sym_idx: usize,
    symbols: &mut Vec<ExtractedSymbol>,
) {
    for (_, field) in type_def.fields.iter() {
        if !emittable_static_field(field.flags, &field.name) {
            continue;
        }
        symbols.push(bare_symbol(
            field.name.clone(),
            format!("{qualified}.{}", field.name),
            SymbolKind::Field,
            None,
            Some(qualified.to_string()),
            Some(type_sym_idx),
        ));
    }
}

/// Public + static, not special-named, not compiler-generated.
pub(super) fn emittable_static_field(flags: u32, name: &str) -> bool {
    flags & FIELD_ACCESS_MASK == FIELD_PUBLIC
        && flags & FIELD_STATIC != 0
        && flags & FIELD_SPECIAL_NAME == 0
        && !name.starts_with('<')
}

/// `Inherits`/`Implements` refs for a cracked type's base class and interface
/// list. The refs make the inheritance map climbable for DLL types — a member
/// declared on a base interface resolves on the derived receiver — and let
/// the demand closure pull each supertype's own defining file, so an
/// interface chain materializes transitively. Targets carry the bare declared
/// name, matching the source-level extractor's base-list emission.
fn emit_supertype_refs(type_def: &CilType, source_symbol_index: usize, refs: &mut Vec<ExtractedRef>) {
    let mut push = |name: &str, kind: EdgeKind| {
        let simple = strip_backtick_arity(name);
        if simple.is_empty() {
            return;
        }
        refs.push(ExtractedRef {
            is_include: false,
            is_import_binding: false,
            is_reexport: false,
            source_symbol_index,
            target_name: simple.to_string(),
            kind,
            line: 0,
            col: 0,
            module: None,
            chain: None,
            byte_offset: 0,
            namespace_segments: Vec::new(),
            call_args: Vec::new(),
        });
    };
    if let Some(base) = type_def.base() {
        push(&base.name, EdgeKind::Inherits);
    }
    for (_, iface_ref) in type_def.interfaces.iter() {
        if let Some(iface) = iface_ref.upgrade() {
            push(&iface.name, EdgeKind::Implements);
        }
    }
}

/// An ExtractedSymbol row with the position/type fields a DLL row cannot
/// carry zeroed out.
fn bare_symbol(
    name: String,
    qualified_name: String,
    kind: SymbolKind,
    signature: Option<String>,
    scope_path: Option<String>,
    parent_index: Option<usize>,
) -> ExtractedSymbol {
    ExtractedSymbol {
        name,
        qualified_name,
        kind,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature,
        doc_comment: None,
        scope_path,
        parent_index,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

#[cfg(test)]
#[path = "type_symbols_tests.rs"]
mod tests;
