// =============================================================================
// nuget/signature_format.rs — ECMA-335 method-signature rendering.
//
// Turns a dotscope `SignatureMethod` into the one-line display signature the
// resolve engine parses: generic placeholders (`!0` / `!!0`) substituted with
// declared parameter names, metadata token references (`class[hex]` /
// `valuetype[hex]`) resolved to namespace-qualified type names, and the
// synthesized `this ` receiver marker for extension-method candidates.
// =============================================================================

pub(crate) fn strip_backtick_arity(name: &str) -> &str {
    match name.find('`') {
        Some(idx) => &name[..idx],
        None => name,
    }
}

pub(crate) fn format_generic_suffix(names: &[String]) -> String {
    if names.is_empty() {
        String::new()
    } else {
        format!("<{}>", names.join(", "))
    }
}

pub(super) fn format_method_signature(
    method_name: &str,
    sig: &dotscope::metadata::signatures::SignatureMethod,
    type_generic_names: &[String],
    method_generic_names: &[String],
    assembly: &dotscope::prelude::CilObject,
    mark_extension_receiver: bool,
) -> String {
    let gp_suffix = format_generic_suffix(method_generic_names);
    let mut params_str = String::from("(");
    for (i, p) in sig.params.iter().enumerate() {
        if i > 0 {
            params_str.push_str(", ");
        }
        if i == 0 && mark_extension_receiver {
            // A static method of a STATIC class reads as an extension
            // candidate: IL carries no textual `this`, so the marker the
            // extension-dispatch rung keys on is synthesized here. The rung's
            // own gates (instance-member miss + receiver-head match + single
            // qname) bound the over-admission of ordinary static helpers.
            params_str.push_str("this ");
        }
        let rendered = format!("{}", p);
        let substituted =
            substitute_generic_placeholders(&rendered, type_generic_names, method_generic_names);
        params_str.push_str(&resolve_signature_tokens(&substituted, assembly));
    }
    params_str.push(')');
    let return_rendered = format!("{}", sig.return_type);
    let return_substituted =
        substitute_generic_placeholders(&return_rendered, type_generic_names, method_generic_names);
    let return_str = resolve_signature_tokens(&return_substituted, assembly);
    format!("{method_name}{gp_suffix}{params_str}: {return_str}")
}

fn resolve_signature_tokens(rendered: &str, assembly: &dotscope::prelude::CilObject) -> String {
    use dotscope::metadata::token::Token;
    let type_registry = assembly.types();
    let imports = assembly.imports().cil();

    let mut out = String::with_capacity(rendered.len());
    let bytes = rendered.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let remaining = &rendered[i..];
        let (prefix_len, skip_prefix) = if remaining.starts_with("class[") {
            (6, true)
        } else if remaining.starts_with("valuetype[") {
            (10, true)
        } else {
            (0, false)
        };
        if skip_prefix {
            let after_prefix = &remaining[prefix_len..];
            if let Some(close_rel) = after_prefix.find(']') {
                let hex = &after_prefix[..close_rel];
                if let Ok(value) = u32::from_str_radix(hex, 16) {
                    let token = Token::new(value);
                    let table_byte = value >> 24;
                    let resolved: Option<String> = match table_byte {
                        0x02 => type_registry.get(&token).map(|ty| {
                            let name = strip_backtick_arity(&ty.name).to_string();
                            if ty.namespace.is_empty() {
                                name
                            } else {
                                format!("{}.{}", ty.namespace, name)
                            }
                        }),
                        0x01 => imports.get(token).map(|imp| {
                            let name = strip_backtick_arity(&imp.name).to_string();
                            if imp.namespace.is_empty() {
                                name
                            } else {
                                format!("{}.{}", imp.namespace, name)
                            }
                        }),
                        _ => None,
                    };
                    if let Some(full) = resolved {
                        out.push_str(&full);
                        i += prefix_len + close_rel + 1;
                        continue;
                    }
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

pub(crate) fn substitute_generic_placeholders(
    rendered: &str,
    type_gen: &[String],
    method_gen: &[String],
) -> String {
    let bytes = rendered.as_bytes();
    let mut out = String::with_capacity(rendered.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'!' {
            let is_method = i + 1 < bytes.len() && bytes[i + 1] == b'!';
            let num_start = if is_method { i + 2 } else { i + 1 };
            let mut num_end = num_start;
            while num_end < bytes.len() && bytes[num_end].is_ascii_digit() {
                num_end += 1
            }
            if num_end > num_start {
                let idx: usize = rendered[num_start..num_end].parse().unwrap_or(usize::MAX);
                let target = if is_method { method_gen } else { type_gen };
                if let Some(name) = target.get(idx) {
                    out.push_str(name);
                    i = num_end;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}
