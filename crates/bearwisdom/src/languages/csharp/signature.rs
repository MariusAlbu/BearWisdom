//! C# extension-method signature markers.

pub(crate) fn extension_receiver(signature: &str) -> Option<String> {
    let index = signature.find("(this ")?;
    let rest = &signature[index + "(this ".len()..];
    let mut depth = 0i32;
    let mut end = rest.len();
    for (index, ch) in rest.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' | ']' | ')' | '}' if depth > 0 => depth -= 1,
            ',' | ')' if depth == 0 => {
                end = index;
                break;
            }
            _ => {}
        }
    }
    let parameter = rest[..end].trim();
    let mut depth = 0i32;
    let mut last_space = None;
    for (index, ch) in parameter.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' | ']' | ')' | '}' => depth -= 1,
            ch if ch.is_whitespace() && depth == 0 => last_space = Some(index),
            _ => {}
        }
    }
    let ty = last_space.map_or(parameter, |index| parameter[..index].trim());
    (!ty.is_empty()).then(|| ty.to_string())
}

/// C# delegate type arguments seed callback-local bindings.
pub(crate) fn delegate_argument_types(signature: &str) -> Vec<String> {
    ["Action", "Func", "Predicate"]
        .into_iter()
        .flat_map(|wrapper| {
            let mut found = Vec::new();
            let mut from = 0;
            while let Some(rel) = signature[from..].find(wrapper) {
                let at = from + rel;
                from = at + wrapper.len();
                let boundary = at == 0
                    || !signature.as_bytes()[at - 1].is_ascii_alphanumeric()
                        && signature.as_bytes()[at - 1] != b'_';
                if !boundary || !signature[from..].starts_with('<') {
                    continue;
                }
                if let Some(args) = angle_args(&signature[from..]) {
                    found.extend(split_args(args));
                }
            }
            found
        })
        .collect()
}

/// C# declaration generic parameters follow the callable name.  This stays in
/// the C# adapter so generic call binding consumes only normalized names.
pub(crate) fn generic_params(
    signature: &str,
    name: &str,
) -> Vec<(String, Option<String>, Option<String>)> {
    let Some(name_at) = signature.rfind(name) else {
        return Vec::new();
    };
    let after_name = name_at + name.len();
    let tail = &signature[after_name..];
    if !tail.starts_with('<') {
        return Vec::new();
    }
    let Some(close) =
        crate::type_checker::profile::signature_parser::find_matching_bracket(tail, '<', '>')
    else {
        return Vec::new();
    };
    tail[1..close]
        .split(',')
        .filter_map(|param| {
            let param = param.trim();
            (!param.is_empty()
                && param
                    .chars()
                    .all(|ch| ch == '_' || ch.is_ascii_alphanumeric()))
            .then(|| (param.to_string(), None, None))
        })
        .collect()
}

fn angle_args(text: &str) -> Option<&str> {
    let mut depth = 0usize;
    for (i, ch) in text.char_indices() {
        match ch {
            '<' => depth += 1,
            '>' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(&text[1..i]);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_args(text: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, ch) in text.char_indices() {
        match ch {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                values.push(text[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = text[start..].trim();
    if !last.is_empty() {
        values.push(last.to_string());
    }
    values
}

#[cfg(test)]
mod tests {
    use super::delegate_argument_types;
    #[test]
    fn reads_only_csharp_delegate_wrapper_arguments() {
        assert_eq!(
            delegate_argument_types("void Configure(Action<Builder, Options> setup)"),
            vec!["Builder", "Options"]
        );
        assert!(delegate_argument_types("void Configure(Callback<Builder> setup)").is_empty());
    }
}
