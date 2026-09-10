//! Go's trailing callable-result syntax.

/// Read a Go callable result.  Go places it after the parameter list rather
/// than after a separator, so this belongs to the Go plugin instead of the
/// generic resolver signature reader.
pub(crate) fn return_type(signature: &str) -> Option<String> {
    let signature = signature.trim();
    let groups_to_skip = method_receiver(signature) as usize;
    let mut depth = 0i32;
    let mut groups_seen = 0usize;
    let mut close = None;
    for (index, byte) in signature.bytes().enumerate() {
        match byte {
            b'(' | b'[' | b'{' => depth += 1,
            b']' | b'}' => depth -= 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    if groups_seen == groups_to_skip {
                        close = Some(index);
                        break;
                    }
                    groups_seen += 1;
                }
            }
            _ => {}
        }
    }
    let mut result = signature[close? + 1..].trim();
    if let Some(body) = result.find('{') {
        result = result[..body].trim_end();
    }
    if result.is_empty() {
        return None;
    }
    if result.starts_with('(') {
        return Some(result.to_string());
    }
    Some(result.trim_start_matches('*').trim_start().to_string())
}

pub(crate) fn parameter_types(signature: &str) -> Option<Vec<String>> {
    let parameters = callable_parameters(signature)?;
    let mut types = Vec::new();
    let mut grouped_names = Vec::new();

    for parameter in split_parameters(parameters) {
        let parameter = parameter.trim();
        if parameter.is_empty() || parameter.contains("...") {
            return None;
        }
        if let Some(type_) = named_parameter_type(parameter) {
            let type_ = type_.trim_start_matches('*').trim_start().to_string();
            for _ in 0..=grouped_names.len() {
                types.push(type_.clone());
            }
            grouped_names.clear();
        } else {
            // A bare item is either an unnamed parameter type or an early name
            // in Go's `func(a, b T)` grouped declaration. Retain it until a
            // following typed item establishes the latter; otherwise it is the
            // unnamed type itself.
            grouped_names.push(parameter.trim_start_matches('*').trim_start().to_string());
        }
    }

    types.extend(grouped_names);
    Some(types)
}

pub(crate) fn declared_type(signature: &str) -> Option<String> {
    crate::type_checker::profile::signature_parser::parse_declared_type_from_signature(
        signature,
        crate::type_checker::profile::signature_parser::DeclaredTypeLayout::Postfix,
    )
    .map(|ty| ty.trim_start_matches('*').trim_start().to_string())
}

fn method_receiver(signature: &str) -> bool {
    let Some(after_func) = signature.strip_prefix("func") else {
        return false;
    };
    let after_func = after_func.trim_start();
    let Some(receiver_end) = matching_paren(after_func) else {
        return false;
    };
    let after_receiver = after_func[receiver_end + 1..].trim_start();
    let name_len = after_receiver
        .bytes()
        .take_while(|byte| *byte == b'_' || byte.is_ascii_alphanumeric())
        .count();
    if name_len == 0 {
        return false;
    }
    let mut after_name = after_receiver[name_len..].trim_start();
    if after_name.starts_with('[') {
        let Some(end) = matching(after_name, '[', ']') else {
            return false;
        };
        after_name = after_name[end + 1..].trim_start();
    }
    after_name.starts_with('(')
}

fn matching_paren(text: &str) -> Option<usize> {
    matching(text, '(', ')')
}

fn matching(text: &str, open: char, close: char) -> Option<usize> {
    text.starts_with(open).then_some(())?;
    let mut depth = 0usize;
    for (index, ch) in text.char_indices() {
        match ch {
            ch if ch == open => depth += 1,
            ch if ch == close => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

/// Return the source text inside a callable declaration's parameter group.
/// Go's receiver, method name, and type parameter group precede the actual
/// parameters, so they must not be confused with the function's slots.
fn callable_parameters(signature: &str) -> Option<&str> {
    let mut rest = signature.trim().strip_prefix("func")?.trim_start();
    if rest.starts_with('(') {
        let receiver_end = matching_paren(rest)?;
        rest = rest[receiver_end + 1..].trim_start();
    }

    let name_len = rest
        .bytes()
        .take_while(|byte| *byte == b'_' || byte.is_ascii_alphanumeric())
        .count();
    if name_len == 0 {
        return None;
    }
    rest = rest[name_len..].trim_start();
    if rest.starts_with('[') {
        let type_params_end = matching(rest, '[', ']')?;
        rest = rest[type_params_end + 1..].trim_start();
    }

    let close = matching_paren(rest)?;
    Some(&rest[1..close])
}

/// Split parameters only at commas outside nested callable, generic, and
/// composite type syntax.
fn split_parameters(parameters: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    for (index, ch) in parameters.char_indices() {
        match ch {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&parameters[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&parameters[start..]);
    parts
}

/// Return the type side of `name Type`, retaining all type-internal spaces.
fn named_parameter_type(parameter: &str) -> Option<&str> {
    let mut depth = 0i32;
    for (index, ch) in parameter.char_indices() {
        match ch {
            '(' | '[' | '{' | '<' => depth += 1,
            ')' | ']' | '}' | '>' => depth -= 1,
            ch if ch.is_whitespace() && depth == 0 => {
                let name = parameter[..index].trim();
                let type_ = parameter[index..].trim();
                if is_identifier(name) && !type_.is_empty() {
                    return Some(type_);
                }
            }
            _ => {}
        }
    }
    None
}

fn is_identifier(text: &str) -> bool {
    let mut bytes = text.bytes();
    matches!(bytes.next(), Some(b'_' | b'a'..=b'z' | b'A'..=b'Z'))
        && bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::{parameter_types, return_type};

    #[test]
    fn reads_free_and_receiver_method_results() {
        assert_eq!(
            return_type("func New(c *Client) *Thing").as_deref(),
            Some("Thing")
        );
        assert_eq!(
            return_type("func (r Receiver) Read() Result").as_deref(),
            Some("Result")
        );
    }

    #[test]
    fn preserves_function_typed_parameter_slots() {
        assert_eq!(
            parameter_types("func use(f func(Item) bool, a, b Result)"),
            Some(vec![
                "func(Item) bool".to_string(),
                "Result".to_string(),
                "Result".to_string(),
            ])
        );
    }
}
