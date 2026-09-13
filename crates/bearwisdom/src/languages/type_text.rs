//! Source-language type-text parsing.
//!
//! This module deliberately sits beside language plugins rather than in
//! `TypeArena`: a `TypeArena` stores normalized semantic values, while a
//! plugin chooses which source spellings it accepts through `TypeTextPolicy`.

use crate::type_checker::core::types::{Type, TypeArena, TypeId};

#[path = "type_text_policy.rs"]
mod policy;
pub use policy::TypeTextPolicy;

/// Intern one source-language type spelling according to an explicit policy.
/// The resulting value is always a normalized `TypeId`; source syntax is never
/// stored in `TypeArena` parsing machinery.
pub fn intern_type_text(arena: &TypeArena, text: &str, policy: TypeTextPolicy) -> TypeId {
    TypeTextParser { arena, policy }.intern(text)
}

/// Read the direct arguments of a source-language type application using the
/// delimiters selected by its adapter. Nested arguments remain opaque heads for
/// this lookup-oriented view; full semantic parsing happens in
/// [`intern_type_text`].
pub(crate) fn parse_type_head_and_args(text: &str, open: char, close: char) -> (&str, Vec<&str>) {
    let Some(open_index) = text.find(open) else {
        return (text.trim(), Vec::new());
    };
    if open_index == 0 {
        return (text.trim(), Vec::new());
    }
    let head = text[..open_index].trim();
    let tail = &text[open_index..];
    let Some(close_index) = find_matching_close(tail, open, close) else {
        return (head, Vec::new());
    };
    if !tail[close_index + close.len_utf8()..].trim().is_empty() {
        return (text.trim(), Vec::new());
    }
    (
        head,
        direct_argument_heads(&tail[open.len_utf8()..close_index]),
    )
}

fn direct_argument_heads(arguments: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (index, byte) in arguments.bytes().enumerate() {
        match byte {
            b'<' | b'[' => depth += 1,
            b'>' | b']' => depth = (depth - 1).max(0),
            b',' if depth == 0 => {
                push_argument_head(&mut result, &arguments[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    push_argument_head(&mut result, &arguments[start..]);
    result
}

fn push_argument_head<'a>(result: &mut Vec<&'a str>, argument: &'a str) {
    let head = argument
        .trim()
        .split(['<', '['])
        .next()
        .unwrap_or(argument)
        .trim();
    if !head.is_empty() {
        result.push(head);
    }
}

/// Compatibility fixture for tests that exercise normalized semantic shapes
/// without selecting a concrete source language. Production ingestion must
/// dispatch through a language plugin instead.
#[cfg(test)]
pub(crate) fn intern_test_type_text(arena: &TypeArena, text: &str) -> TypeId {
    intern_type_text(arena, text, TypeTextPolicy::ALL_LEGACY_FORMS)
}

struct TypeTextParser<'a> {
    arena: &'a TypeArena,
    policy: TypeTextPolicy,
}

impl TypeTextParser<'_> {
    fn intern(&self, text: &str) -> TypeId {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return self.arena.class(text);
        }
        // An atomic spelling denotes a semantic atom, never a nominal. Asked
        // before every syntax rule: an atom is a whole spelling, so no rule can
        // decompose it into something else.
        if let Some(atom) = self.policy.atom(trimmed) {
            return self.arena.intern(Type::Intrinsic(atom));
        }
        if self.policy.reference_sigil {
            if let Some(referent) = strip_reference_sigil(trimmed) {
                return self.intern(referent);
            }
        }
        if self.policy.pointer_sigil {
            if let Some(pointee) = trimmed.strip_prefix('*') {
                return self.intern(pointee.trim_start());
            }
        }
        if self.policy.opaque_existential_prefix {
            if let Some(inner) = strip_opaque_existential_prefix(trimmed) {
                return self.intern(inner);
            }
        }
        if self.policy.python_callable {
            if let Some((params, return_)) = parse_python_callable_type(trimmed) {
                return self.arena.intern(Type::Function {
                    params: params.iter().map(|param| self.intern(param)).collect(),
                    return_: self.intern(&return_),
                });
            }
        }
        if self.policy.dart_function {
            if let Some((params, return_)) = parse_dart_function_type(trimmed) {
                return self.arena.intern(Type::Function {
                    params: params
                        .iter()
                        .map(|param| self.intern(strip_dart_function_param_name(param)))
                        .collect(),
                    return_: self.intern(&return_),
                });
            }
        }
        if self.policy.go_function {
            if let Some((params, return_)) = parse_go_function_type(trimmed) {
                return self.arena.intern(Type::Function {
                    params: params.iter().map(|param| self.intern(param)).collect(),
                    return_: self.intern(&return_),
                });
            }
        }
        if self.policy.fat_arrow_function || self.policy.thin_arrow_function {
            if let Some(arrow) = find_top_level_arrow(
                trimmed,
                self.policy.fat_arrow_function,
                self.policy.thin_arrow_function,
            ) {
                let return_ = self.intern(trimmed[arrow + 2..].trim());
                let pre = trimmed[..arrow].trim();
                let params = match pre.char_indices().find(|&(_, c)| c == '(') {
                    Some((open, _)) => match find_matching_close(&pre[open..], '(', ')') {
                        Some(close_rel) => {
                            split_depth_zero_commas(&pre[open + 1..open + close_rel])
                                .iter()
                                .map(|piece| self.intern(strip_param_name(piece)))
                                .collect()
                        }
                        None => Vec::new(),
                    },
                    None if self.policy.bare_arrow_parameter && !pre.is_empty() => {
                        vec![self.intern(strip_param_name(pre))]
                    }
                    None => Vec::new(),
                };
                return self.arena.intern(Type::Function { params, return_ });
            }
        }

        let trimmed = if self.policy.readonly_modifier {
            trimmed
                .strip_prefix("readonly ")
                .map(str::trim)
                .unwrap_or(trimmed)
        } else {
            trimmed
        };
        if self.policy.nullable_prefix {
            if let Some(inner) = trimmed.strip_prefix('?') {
                let inner = inner.trim_start();
                if !inner.is_empty() {
                    return self.arena.intern(Type::Optional(self.intern(inner)));
                }
            }
        }
        if self.policy.nullable_suffix {
            if let Some(inner) = trimmed.strip_suffix('?') {
                let inner = inner.trim_end();
                if !inner.is_empty() {
                    return self.arena.intern(Type::Optional(self.intern(inner)));
                }
            }
        }
        if self.policy.array_suffix {
            if let Some(elem) = trimmed.strip_suffix("[]") {
                let elem = elem.trim();
                if !elem.is_empty() {
                    return self.array_of(self.intern(elem));
                }
            }
        }
        if self.policy.union_intersection {
            if let Some(arms) = split_top_level(trimmed, '|') {
                return self.arena.intern(Type::Union(
                    arms.iter().map(|arm| self.intern(arm)).collect(),
                ));
            }
            if let Some(arms) = split_top_level(trimmed, '&') {
                return self.arena.intern(Type::Intersection(
                    arms.iter().map(|arm| self.intern(arm)).collect(),
                ));
            }
        }
        if self.policy.parenthesized_tuple {
            if let Some(elements) = parenthesized_tuple_elements(trimmed) {
                return self.arena.intern(Type::Tuple(
                    elements
                        .iter()
                        .map(|element| self.intern(strip_tuple_label(element)))
                        .collect(),
                ));
            }
        }
        if self.policy.bracket_tuple || self.policy.bracket_array || self.policy.rust_array_or_slice
        {
            if let Some(inner) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                let elements = split_depth_zero_commas(inner);
                if self.policy.bracket_tuple && elements.len() >= 2 {
                    return self.arena.intern(Type::Tuple(
                        elements
                            .iter()
                            .map(|element| self.intern(strip_tuple_label(element)))
                            .collect(),
                    ));
                }
                if self.policy.rust_array_or_slice {
                    let element = match find_depth_zero_semicolon(inner) {
                        Some(semi) => inner[..semi].trim(),
                        None => inner.trim(),
                    };
                    if !element.is_empty() {
                        return self.array_of(self.intern(element));
                    }
                }
                if self.policy.bracket_array
                    && elements.len() == 1
                    && find_depth_zero_semicolon(inner).is_none()
                {
                    let element = inner.trim();
                    if !element.is_empty() {
                        return self.array_of(self.intern(element));
                    }
                }
            }
        }

        // Preserve the former parser's first-delimiter rule: if an earlier
        // enabled opener produces an invalid application, do not reinterpret a
        // later delimiter as a second, unrelated grammar production.
        let application_opener = [
            self.policy
                .angle_application
                .then(|| trimmed.find('<').map(|index| (index, '<', '>')))
                .flatten(),
            self.policy
                .bracket_application
                .then(|| trimmed.find('[').map(|index| (index, '[', ']')))
                .flatten(),
        ]
        .into_iter()
        .flatten()
        .min_by_key(|(index, _, _)| *index);
        let application =
            application_opener.and_then(|(_, open, close)| find_application(trimmed, open, close));
        let Some((head, inner)) = application else {
            return self.arena.class(trimmed);
        };
        let split_args = split_depth_zero_commas(inner);
        let args: Vec<&str> = split_args
            .iter()
            .map(String::as_str)
            .filter(|arg| !self.policy.lifetime_arguments || !arg.trim_start().starts_with('\''))
            .collect();
        if args.is_empty() {
            return self.arena.class(head);
        }
        let base = self.arena.class(head);
        self.arena.intern(Type::Apply {
            base,
            args: args.iter().map(|arg| self.intern(arg)).collect(),
        })
    }

    fn array_of(&self, element: TypeId) -> TypeId {
        self.arena.intern(Type::Apply {
            base: self.arena.class("Array"),
            args: vec![element],
        })
    }
}

fn find_application(s: &str, open: char, close: char) -> Option<(&str, &str)> {
    let open_idx = s.find(open)?;
    let head = s[..open_idx].trim();
    if head.is_empty() {
        return None;
    }
    let rest = &s[open_idx..];
    let close_rel = find_matching_close(rest, open, close)?;
    if !s[open_idx + close_rel + 1..].trim().is_empty() {
        return None;
    }
    Some((head, &s[open_idx + 1..open_idx + close_rel]))
}

fn strip_reference_sigil(s: &str) -> Option<&str> {
    let rest = s.strip_prefix('&')?.trim_start();
    let rest = if let Some(after_tick) = rest.strip_prefix('\'') {
        after_tick
            .find(char::is_whitespace)
            .map(|ws| after_tick[ws..].trim_start())
            .unwrap_or("")
    } else {
        rest
    };
    Some(match rest.strip_prefix("mut") {
        Some(after) if after.starts_with(char::is_whitespace) => after.trim_start(),
        _ => rest,
    })
}

fn strip_opaque_existential_prefix(s: &str) -> Option<&str> {
    ["some", "any"].iter().find_map(|keyword| {
        let after = s.strip_prefix(keyword)?;
        after
            .starts_with(char::is_whitespace)
            .then(|| after.trim_start())
            .filter(|inner| !inner.is_empty())
    })
}

fn find_top_level_arrow(s: &str, allow_fat: bool, allow_thin: bool) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0i32;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'(' | b'[' | b'<' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'>' => depth -= 1,
            b'=' if allow_fat && depth == 0 && bytes.get(index + 1) == Some(&b'>') => {
                return Some(index)
            }
            b'-' if allow_thin && depth == 0 && bytes.get(index + 1) == Some(&b'>') => {
                return Some(index)
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn find_matching_close(s: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0i32;
    for (index, ch) in s.char_indices() {
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

fn split_depth_zero_commas(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (index, ch) in s.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' | ']' | ')' | '}' => depth -= 1,
            ',' if depth == 0 => {
                let part = s[start..index].trim();
                if !part.is_empty() {
                    result.push(part.to_string());
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    let part = s[start..].trim();
    if !part.is_empty() {
        result.push(part.to_string());
    }
    result
}

fn strip_param_name(piece: &str) -> &str {
    let mut depth = 0i32;
    let mut last_colon = None;
    for (index, ch) in piece.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' | ']' | ')' | '}' => depth -= 1,
            ':' if depth == 0 => last_colon = Some(index),
            _ => {}
        }
    }
    last_colon
        .map(|index| piece[index + 1..].trim())
        .unwrap_or_else(|| piece.trim())
}

fn split_top_level(s: &str, delimiter: char) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (index, ch) in s.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' | ']' | ')' | '}' => depth -= 1,
            ch if ch == delimiter && depth == 0 => {
                let part = s[start..index].trim();
                if !part.is_empty() {
                    parts.push(part.to_string());
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    let part = s[start..].trim();
    if !part.is_empty() {
        parts.push(part.to_string());
    }
    (parts.len() >= 2).then_some(parts)
}

fn parenthesized_tuple_elements(s: &str) -> Option<Vec<String>> {
    if !s.starts_with('(') || !s.ends_with(')') || find_matching_close(s, '(', ')')? != s.len() - 1
    {
        return None;
    }
    let inner = &s[1..s.len() - 1];
    let mut elements = Vec::new();
    let mut delimiters = Vec::new();
    let mut start = 0;
    for (index, ch) in inner.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => delimiters.push(ch),
            '>' if index > 0 && inner.as_bytes()[index - 1] == b'-' => {}
            '>' | ']' | ')' | '}' => {
                let expected_open = match ch {
                    '>' => '<',
                    ']' => '[',
                    ')' => '(',
                    '}' => '{',
                    _ => unreachable!(),
                };
                if delimiters.pop() != Some(expected_open) {
                    return None;
                }
            }
            ',' if delimiters.is_empty() => {
                let element = inner[start..index].trim();
                if element.is_empty() {
                    return None;
                }
                elements.push(element.to_string());
                start = index + 1;
            }
            _ => {}
        }
    }
    let tail = inner[start..].trim();
    if !tail.is_empty() {
        elements.push(tail.to_string());
    }
    (delimiters.is_empty() && elements.len() >= 2).then_some(elements)
}

fn find_depth_zero_semicolon(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    for (index, ch) in s.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' | ']' | ')' | '}' => depth -= 1,
            ';' if depth == 0 => return Some(index),
            _ => {}
        }
    }
    None
}

fn strip_tuple_label(element: &str) -> &str {
    let element = element.trim();
    if let Some((label, rest)) = element.split_once(':') {
        let label = label.trim_end_matches('?').trim();
        if !label.is_empty()
            && label.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
            && !rest.trim_start().starts_with(':')
        {
            return rest.trim();
        }
    }
    element
}

fn parse_python_callable_type(s: &str) -> Option<(Vec<String>, String)> {
    let open = s.find('[')?;
    if !matches!(
        s[..open].trim(),
        "Callable" | "typing.Callable" | "collections.abc.Callable"
    ) {
        return None;
    }
    let close = open + find_matching_close(&s[open..], '[', ']')?;
    if !s[close + 1..].trim().is_empty() {
        return None;
    }
    let args = split_depth_zero_commas(&s[open + 1..close]);
    if args.len() != 2 {
        return None;
    }
    let params = args[0].trim();
    let return_ = args[1].trim();
    let inner = params.strip_prefix('[')?.strip_suffix(']')?;
    if return_.is_empty()
        || has_unmodeled_callable_identity(params)
        || has_unmodeled_callable_identity(return_)
    {
        return None;
    }
    let parameters = split_depth_zero_commas(inner);
    (!parameters
        .iter()
        .any(|param| has_unmodeled_callable_identity(param)))
    .then_some((parameters, return_.to_string()))
}

fn has_unmodeled_callable_identity(s: &str) -> bool {
    let s = s.trim();
    s == "..."
        || s.starts_with('~')
        || s.contains("TypeVar(")
        || s.contains("ParamSpec")
        || s.contains("Concatenate")
        || s.ends_with(".args")
        || s.ends_with(".kwargs")
}

fn parse_dart_function_type(s: &str) -> Option<(Vec<String>, String)> {
    let mut depth = 0i32;
    for (index, ch) in s.char_indices() {
        match ch {
            '<' | '[' | '{' | '(' => depth += 1,
            '>' | ']' | '}' | ')' => depth -= 1,
            'F' if depth == 0 && s[index..].starts_with("Function(") => {
                let return_ = s[..index].trim();
                if return_.is_empty()
                    || !s[..index]
                        .chars()
                        .next_back()
                        .is_some_and(char::is_whitespace)
                {
                    continue;
                }
                let open = index + "Function".len();
                let close = open + find_matching_close(&s[open..], '(', ')')?;
                if !s[close + 1..].trim().is_empty() {
                    return None;
                }
                return Some((
                    split_depth_zero_commas(&s[open + 1..close]),
                    return_.to_string(),
                ));
            }
            _ => {}
        }
    }
    None
}

fn strip_dart_function_param_name(param: &str) -> &str {
    let mut depth = 0i32;
    let mut split = None;
    for (index, ch) in param.char_indices() {
        match ch {
            '<' | '[' | '{' | '(' => depth += 1,
            '>' | ']' | '}' | ')' => depth -= 1,
            ch if ch.is_whitespace() && depth == 0 => split = Some(index),
            _ => {}
        }
    }
    let Some(split) = split else {
        return param.trim();
    };
    let type_ = param[..split].trim_end();
    let name = param[split..].trim();
    (!type_.is_empty() && is_identifier(name))
        .then_some(type_)
        .unwrap_or_else(|| param.trim())
}

fn parse_go_function_type(s: &str) -> Option<(Vec<String>, String)> {
    let rest = s.strip_prefix("func")?.trim_start();
    if !rest.starts_with('(') {
        return None;
    }
    let close = find_matching_close(rest, '(', ')')?;
    let return_ = rest[close + 1..].trim();
    if return_.is_empty() || return_.starts_with('(') || go_declaration_tail(return_) {
        return None;
    }
    Some((
        parse_go_function_params(&rest[1..close])?,
        return_.to_string(),
    ))
}

fn go_declaration_tail(tail: &str) -> bool {
    let length = tail
        .bytes()
        .take_while(|byte| *byte == b'_' || byte.is_ascii_alphanumeric())
        .count();
    let name = &tail[..length];
    !name.is_empty() && name != "func" && tail[length..].trim_start().starts_with('(')
}

fn parse_go_function_params(inner: &str) -> Option<Vec<String>> {
    if inner.trim().is_empty() {
        return Some(Vec::new());
    }
    let mut params = Vec::new();
    let mut grouped_names = Vec::new();
    for part in split_depth_zero_commas(inner) {
        let part = part.trim();
        if part.is_empty() || part.contains("...") {
            return None;
        }
        if let Some(type_) = go_named_parameter_type(part) {
            params.extend(grouped_names.drain(..).map(|_| type_.to_string()));
            params.push(type_.to_string());
        } else {
            grouped_names.push(part.to_string());
        }
    }
    params.extend(grouped_names);
    Some(params)
}

fn go_named_parameter_type(part: &str) -> Option<&str> {
    let mut depth = 0i32;
    for (index, ch) in part.char_indices() {
        match ch {
            '<' | '[' | '{' | '(' => depth += 1,
            '>' | ']' | '}' | ')' => depth -= 1,
            ch if ch.is_whitespace() && depth == 0 => {
                let name = part[..index].trim();
                let type_ = part[index..].trim();
                return (is_identifier(name) && !type_.is_empty()).then_some(type_);
            }
            _ => {}
        }
    }
    None
}

fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some('_' | 'a'..='z' | 'A'..='Z'))
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_policy_does_not_interpret_foreign_surface_syntax() {
        let arena = TypeArena::new();
        let id = intern_type_text(&arena, "Callable[[A], B] | C?", TypeTextPolicy::OPAQUE);
        assert!(matches!(arena.get(id), Type::Class(name) if name == "Callable[[A], B] | C?"));
    }

    #[test]
    fn individual_features_require_explicit_opt_in() {
        let arena = TypeArena::new();
        let angle = TypeTextPolicy {
            angle_application: true,
            ..TypeTextPolicy::OPAQUE
        };
        assert!(matches!(
            arena.get(intern_type_text(&arena, "Box<Item>", angle)),
            Type::Apply { .. }
        ));
        assert!(
            matches!(arena.get(intern_type_text(&arena, "Item?", angle)), Type::Class(name) if name == "Item?")
        );
    }

    #[test]
    fn arrow_delimiters_are_independent_capabilities() {
        let arena = TypeArena::new();
        let fat = TypeTextPolicy {
            fat_arrow_function: true,
            ..TypeTextPolicy::OPAQUE
        };
        assert!(matches!(
            arena.get(intern_type_text(&arena, "(Item) => Result", fat)),
            Type::Function { .. }
        ));
        assert!(matches!(
            arena.get(intern_type_text(&arena, "(Item) -> Result", fat)),
            Type::Class(name) if name == "(Item) -> Result"
        ));

        let thin = TypeTextPolicy {
            thin_arrow_function: true,
            ..TypeTextPolicy::OPAQUE
        };
        assert!(matches!(
            arena.get(intern_type_text(&arena, "(Item) -> Result", thin)),
            Type::Function { .. }
        ));
        assert!(matches!(
            arena.get(intern_type_text(&arena, "(Item) => Result", thin)),
            Type::Class(name) if name == "(Item) => Result"
        ));
    }

    #[test]
    fn bare_arrow_parameter_requires_explicit_opt_in() {
        let arena = TypeArena::new();
        let policy = TypeTextPolicy {
            fat_arrow_function: true,
            bare_arrow_parameter: true,
            ..TypeTextPolicy::OPAQUE
        };
        let id = intern_type_text(&arena, "Item => Result", policy);
        match arena.get(id) {
            Type::Function { params, return_ } => {
                assert_eq!(params, vec![arena.class("Item")]);
                assert_eq!(return_, arena.class("Result"));
            }
            other => panic!("expected Function, got {other:?}"),
        }
    }

    #[test]
    fn legacy_compatibility_policy_retains_structural_function_forms() {
        let arena = TypeArena::new();
        let id = intern_type_text(
            &arena,
            "func(value Box<Item>) Result",
            TypeTextPolicy::ALL_LEGACY_FORMS,
        );
        assert!(matches!(arena.get(id), Type::Function { .. }));
    }
}
