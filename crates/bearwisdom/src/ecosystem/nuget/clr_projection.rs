// =============================================================================
// nuget/clr_projection.rs — source-name projection from CLR metadata
// attributes.
//
// The F# compiler renames constructs between source and IL and records the
// mapping as metadata attributes on the emitted rows:
//   * CompilationSourceNameAttribute — the source name of a member whose
//     compiled name was changed by `[<CompiledName>]` (IL `Map`, source `map`).
//   * CompilationRepresentationAttribute(ModuleSuffix) — the module's compiled
//     type name carries a `Module` suffix (source `List`, IL `ListModule`).
//   * CompilationMappingAttribute(SourceConstructFlags.Module) — marks a
//     compiled static class as a source-level module.
// Decoding these attributes lets DLL-sourced symbols surface under the names
// project code writes, with no per-library name tables.
//
// The per-entity attribute lists dotscope attaches carry values without the
// attribute's type identity, so identity comes from the raw CustomAttribute
// table: each row names its constructor (a MethodDef of this assembly or a
// MemberRef into another), and the constructor's declaring type is the
// attribute. Value blobs decode per ECMA-335 §II.23.3.
// =============================================================================

use std::collections::HashMap;
use std::collections::HashSet;

use dotscope::metadata::tables::{CustomAttributeRaw, TableId};
use dotscope::metadata::typesystem::CilTypeReference;
use dotscope::prelude::CilObject;

use super::type_qname::assembly_type_defs;

/// `CompilationRepresentationFlags.ModuleSuffix` (FSharp.Core enum value).
const MODULE_SUFFIX_FLAG: i32 = 4;
/// `SourceConstructFlags.KindMask` / `SourceConstructFlags.Module`
/// (FSharp.Core enum values).
const SOURCE_CONSTRUCT_KIND_MASK: i32 = 31;
const SOURCE_CONSTRUCT_MODULE: i32 = 7;

/// The compilation-mapping attributes this projection decodes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum KnownAttr {
    SourceName,
    Representation,
    Mapping,
}

fn classify(attribute_type_name: &str) -> Option<KnownAttr> {
    match attribute_type_name {
        "CompilationSourceNameAttribute" => Some(KnownAttr::SourceName),
        "CompilationRepresentationAttribute" => Some(KnownAttr::Representation),
        "CompilationMappingAttribute" => Some(KnownAttr::Mapping),
        _ => None,
    }
}

/// Token-keyed source-name maps for one assembly, built once per parse and
/// consulted for every emitted symbol row.
#[derive(Default)]
pub(super) struct SourceNameProjection {
    /// MethodDef token value → source-level member name.
    method_names: HashMap<u32, String>,
    /// TypeDef token values whose compiled name carries the `Module` suffix.
    suffixed_types: HashSet<u32>,
    /// TypeDef token values compiled from source-level modules.
    module_types: HashSet<u32>,
}

impl SourceNameProjection {
    pub(super) fn from_assembly(assembly: &CilObject) -> Self {
        let mut out = Self::default();
        let Some(tables) = assembly.tables() else {
            return out;
        };
        let Some(table) = tables.table::<CustomAttributeRaw>() else {
            return out;
        };
        let Some(blob) = assembly.blob() else {
            return out;
        };
        let local_ctors = local_attribute_ctors(assembly);
        for row in table.iter() {
            let Some(attr) = attribute_kind(assembly, &local_ctors, &row) else {
                continue;
            };
            let Ok(bytes) = blob.get(row.value as usize) else {
                continue;
            };
            let parent_token = row.parent.token.value();
            match attr {
                KnownAttr::SourceName if row.parent.tag == TableId::MethodDef => {
                    if let Some(name) = decode_string_arg(bytes) {
                        out.method_names.insert(parent_token, name);
                    }
                }
                KnownAttr::Representation if row.parent.tag == TableId::TypeDef => {
                    if decode_i32_arg(bytes).is_some_and(|f| f & MODULE_SUFFIX_FLAG != 0) {
                        out.suffixed_types.insert(parent_token);
                    }
                }
                KnownAttr::Mapping if row.parent.tag == TableId::TypeDef => {
                    let is_module = decode_i32_arg(bytes)
                        .is_some_and(|f| f & SOURCE_CONSTRUCT_KIND_MASK == SOURCE_CONSTRUCT_MODULE);
                    if is_module {
                        out.module_types.insert(parent_token);
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// Source-level simple name for a type row (arity-stripped IL name in).
    /// A module-suffixed type sheds its `Module` tail; everything else keeps
    /// its IL name.
    pub(super) fn type_name<'a>(&self, token: u32, il_name: &'a str) -> &'a str {
        if self.suffixed_types.contains(&token) {
            if let Some(stripped) = il_name.strip_suffix("Module") {
                if !stripped.is_empty() {
                    return stripped;
                }
            }
        }
        il_name
    }

    /// Source-level name for a method row, when a source-name record exists.
    pub(super) fn method_name<'a>(&'a self, token: u32, il_name: &'a str) -> &'a str {
        self.method_names
            .get(&token)
            .map(String::as_str)
            .unwrap_or(il_name)
    }

    /// True when the type row was compiled from a source-level module.
    pub(super) fn is_module(&self, token: u32) -> bool {
        self.module_types.contains(&token)
    }
}

/// The known attribute a CustomAttribute row instantiates, resolved through
/// its constructor: a MemberRef constructor names its declaring type
/// directly; a MethodDef constructor is one of this assembly's own attribute
/// `.ctor`s, pre-collected in `local_ctors`.
fn attribute_kind(
    assembly: &CilObject,
    local_ctors: &HashMap<u32, KnownAttr>,
    row: &CustomAttributeRaw,
) -> Option<KnownAttr> {
    match row.constructor.tag {
        TableId::MemberRef => {
            let member = assembly.member_ref(&row.constructor.token)?;
            if member.name != ".ctor" {
                return None;
            }
            let declaring = match &member.declaredby {
                CilTypeReference::TypeRef(r)
                | CilTypeReference::TypeDef(r)
                | CilTypeReference::TypeSpec(r) => r.upgrade()?.name.clone(),
                _ => return None,
            };
            classify(&declaring)
        }
        TableId::MethodDef => local_ctors.get(&row.constructor.token.value()).copied(),
        _ => None,
    }
}

/// `.ctor` method tokens of this assembly's own declarations of the known
/// compilation-mapping attributes (present when the assembly being read is
/// the one that defines them).
fn local_attribute_ctors(assembly: &CilObject) -> HashMap<u32, KnownAttr> {
    let mut out = HashMap::new();
    for type_def in assembly_type_defs(assembly) {
        let Some(kind) = classify(&type_def.name) else {
            continue;
        };
        for (_, method_ref) in type_def.methods.iter() {
            if let Some(method) = method_ref.upgrade() {
                if method.name == ".ctor" {
                    out.insert(method.token.value(), kind);
                }
            }
        }
    }
    out
}

/// The fixed-argument bytes of a custom-attribute value blob, behind the
/// 2-byte 0x0001 prolog (ECMA-335 §II.23.3).
fn fixed_args(bytes: &[u8]) -> Option<&[u8]> {
    if bytes.len() < 2 || bytes[0] != 0x01 || bytes[1] != 0x00 {
        return None;
    }
    Some(&bytes[2..])
}

/// First fixed argument decoded as a SerString: packed length + UTF-8.
fn decode_string_arg(bytes: &[u8]) -> Option<String> {
    let rest = fixed_args(bytes)?;
    let (len, consumed) = read_packed_len(rest)?;
    let end = consumed.checked_add(len)?;
    if end > rest.len() {
        return None;
    }
    std::str::from_utf8(&rest[consumed..end])
        .ok()
        .map(str::to_string)
}

/// First fixed argument decoded as a little-endian i32 — enum-typed
/// constructor arguments serialize as their underlying integer.
fn decode_i32_arg(bytes: &[u8]) -> Option<i32> {
    let rest = fixed_args(bytes)?;
    let b: [u8; 4] = rest.get(..4)?.try_into().ok()?;
    Some(i32::from_le_bytes(b))
}

/// ECMA-335 §II.23.2 compressed unsigned integer: `(value, bytes_read)`.
/// The 0xFF null-string marker declines.
fn read_packed_len(bytes: &[u8]) -> Option<(usize, usize)> {
    let b0 = *bytes.first()?;
    if b0 == 0xFF {
        return None;
    }
    if b0 & 0x80 == 0 {
        return Some((b0 as usize, 1));
    }
    if b0 & 0xC0 == 0x80 {
        let b1 = *bytes.get(1)?;
        return Some(((((b0 & 0x3F) as usize) << 8) | b1 as usize, 2));
    }
    if b0 & 0xE0 == 0xC0 {
        let b = bytes.get(1..4)?;
        let v = (((b0 & 0x1F) as usize) << 24)
            | ((b[0] as usize) << 16)
            | ((b[1] as usize) << 8)
            | b[2] as usize;
        return Some((v, 4));
    }
    None
}

#[cfg(test)]
#[path = "clr_projection_tests.rs"]
mod tests;
