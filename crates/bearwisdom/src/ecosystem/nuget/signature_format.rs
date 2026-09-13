// =============================================================================
// nuget/signature_format.rs — ECMA-335 method-signature rendering.
//
// Turns a dotscope `SignatureMethod` into the one-line display signature the
// resolve engine parses: generic placeholders (`!0` / `!!0`) substituted with
// declared parameter names, metadata token references (`class[hex]` /
// `valuetype[hex]`) resolved to namespace-qualified type names, and the
// synthesized `this ` receiver marker for extension-method candidates.
//
// Methods and constructors share the parameter-list renderer and the line
// shape; they differ only in what stands in the return slot.
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
    let params_str = format_parameter_list(
        sig,
        type_generic_names,
        method_generic_names,
        assembly,
        mark_extension_receiver,
    );
    let return_str = render_signature_element(
        &sig.return_type,
        type_generic_names,
        method_generic_names,
        assembly,
    );
    compose_signature_line(
        method_name,
        &format_generic_suffix(method_generic_names),
        &params_str,
        &return_str,
    )
}

/// An instance constructor's display signature. The return slot carries the
/// declaring type because construction yields that type, and the return slot is
/// what a callable row's yield is decoded from. A `.ctor` declares no method
/// generics of its own and is never an extension candidate, so the generic
/// suffix comes from the declaring type and the receiver marker is off.
pub(super) fn format_constructor_signature(
    display: &str,
    sig: &dotscope::metadata::signatures::SignatureMethod,
    type_generic_names: &[String],
    declaring_qname: &str,
    assembly: &dotscope::prelude::CilObject,
) -> String {
    let params_str = format_parameter_list(sig, type_generic_names, &[], assembly, false);
    compose_signature_line(
        display,
        &format_generic_suffix(type_generic_names),
        &params_str,
        declaring_qname,
    )
}

/// The `Name<GP>(params): Ret` one-line display shape of a callable.
pub(super) fn compose_signature_line(
    name: &str,
    gp_suffix: &str,
    params: &str,
    return_text: &str,
) -> String {
    format!("{name}{gp_suffix}{params}: {return_text}")
}

/// The parenthesised parameter list of one signature, every parameter rendered
/// to display text.
pub(super) fn format_parameter_list(
    sig: &dotscope::metadata::signatures::SignatureMethod,
    type_generic_names: &[String],
    method_generic_names: &[String],
    assembly: &dotscope::prelude::CilObject,
    mark_extension_receiver: bool,
) -> String {
    let rendered: Vec<String> = sig
        .params
        .iter()
        .map(|p| render_signature_element(p, type_generic_names, method_generic_names, assembly))
        .collect();
    join_parameter_list(&rendered, mark_extension_receiver)
}

/// Joins already-rendered parameter texts into the `(a, b)` list.
///
/// A static method of a STATIC class reads as an extension candidate: IL
/// carries no textual `this`, so the marker the extension-dispatch rung keys
/// on is synthesized on the first parameter here. The rung's own gates
/// (instance-member miss + receiver-head match + single qname) bound the
/// over-admission of ordinary static helpers.
pub(super) fn join_parameter_list(rendered: &[String], mark_extension_receiver: bool) -> String {
    let mut params_str = String::from("(");
    for (i, text) in rendered.iter().enumerate() {
        if i > 0 {
            params_str.push_str(", ");
        }
        if i == 0 && mark_extension_receiver {
            params_str.push_str("this ");
        }
        params_str.push_str(text);
    }
    params_str.push(')');
    params_str
}

/// One parameter or return element rendered to display text: ECMA-335 generic
/// placeholders substituted, metadata-token references resolved.
fn render_signature_element(
    element: &impl std::fmt::Display,
    type_generic_names: &[String],
    method_generic_names: &[String],
    assembly: &dotscope::prelude::CilObject,
) -> String {
    let rendered = format!("{element}");
    let substituted =
        substitute_generic_placeholders(&rendered, type_generic_names, method_generic_names);
    resolve_signature_tokens(&substituted, assembly)
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

#[cfg(test)]
#[path = "signature_format_tests.rs"]
mod tests;
