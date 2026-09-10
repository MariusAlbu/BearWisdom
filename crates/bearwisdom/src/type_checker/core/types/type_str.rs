// =============================================================================
// core/types/type_str.rs — string → structured type parsing
//
// `intern_type_str` and its helpers: decompose a type written as source text
// (`Map<K, V>`, `Foo<Bar<Baz>>`, Scala `Map[K, V]`) into structural
// `Apply`/`Class` types. Child module of `types`, so the parser keeps access
// to the arena's interning internals.
// =============================================================================

use super::*;

impl TypeArena {
    /// Intern a type written as a string, decomposing generic applications
    /// into structural `Apply { base, args }`. Recognized shapes:
    ///   - `Foo`            → `Class("Foo")`
    ///   - `Foo<Bar>`       → `Apply(Class("Foo"), [Class("Bar")])`
    ///   - `Map<K, V>`      → `Apply(Class("Map"), [Class("K"), Class("V")])`
    ///   - `Foo<Bar<Baz>>`  → nested `Apply` recursively
    ///   - `Map[K, V]`      → `Apply(Class("Map"), [...])` (Scala bracket style)
    /// Anything the parser can't classify (function types, unions,
    /// intersections, tuples) falls back to `Class(input)` so consumers
    /// always get a TypeId.
    pub fn intern_type_str(&self, s: &str) -> TypeId {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return self.class(s);
        }
        // Reference sigil: a LEADING `&`, optionally followed by a lifetime
        // (`'a`) and `mut`, denotes a reference whose members are the
        // referent's. A `&`-led string is never a valid class name in any
        // language, so re-interning the referent is universally sound:
        // `&C`→C, `&mut C`→C, `&'a C`→C, `&Box<C>`→Box<C>. Only byte 0 is
        // treated as a sigil — an interior `&` (a TS `A & B` intersection)
        // stays on the union/intersection fallback below.
        if let Some(referent) = strip_reference_sigil(trimmed) {
            return self.intern_type_str(referent);
        }
        // Pointer sigil: a LEADING `*` (Go `*fiber.Ctx`, C/C++/C#/Zig `*T`)
        // denotes a pointer whose members are the pointee's; `**T` peels the
        // same way. A `*`-led string is never a class name, so re-interning
        // the pointee is sound.
        if let Some(pointee) = strip_pointer_sigil(trimmed) {
            return self.intern_type_str(pointee);
        }
        // Opaque/existential prefix: a LEADING `some`/`any` keyword (Swift's
        // `some P` opaque type / `any P` existential) names a value whose
        // member-lookup base is the constraint `P`, not a type literally named
        // `some P`. The keyword is contextual and only appears in type position,
        // so peeling it and re-interning the inner type is sound: `some Greet`→
        // Greet, `any Collection<Int>`→Collection<Int>. A word boundary is
        // required so a class named `Something`/`anyOf` is left untouched.
        if let Some(inner) = strip_opaque_existential_prefix(trimmed) {
            return self.intern_type_str(inner);
        }
        // Python's fixed-arity `Callable[[A, B], R]` annotation is a
        // structural function type. Only the explicit parameter-list form is
        // representable: `Callable[..., R]`, ParamSpec, and Concatenate carry
        // calling-convention identity that this compact type does not model.
        if let Some((params, return_)) = parse_python_callable_type(trimmed) {
            let params = params
                .iter()
                .map(|param| self.intern_type_str(param))
                .collect();
            let return_ = self.intern_type_str(&return_);
            return self.intern(Type::Function { params, return_ });
        }
        // Dart writes function types return-first: `R Function(A, B)`. This
        // spelling is structurally distinct from a nominal generic and needs
        // to become the same callable shape as arrow function types before the
        // generic-bracket parser sees its parameter list.
        if let Some((params, return_)) = parse_dart_function_type(trimmed) {
            let params = params
                .iter()
                .map(|param| self.intern_type_str(strip_dart_function_param_name(param)))
                .collect();
            let return_ = self.intern_type_str(&return_);
            return self.intern(Type::Function { params, return_ });
        }
        // Go function types likewise have no arrow: `func(T) R`. Recognition
        // deliberately rejects variadic and multi-result forms because this
        // compact Type::Function has no sound representation for either.
        if let Some((params, return_)) = parse_go_function_type(trimmed) {
            let params = params
                .iter()
                .map(|param| self.intern_type_str(param))
                .collect();
            let return_ = self.intern_type_str(&return_);
            return self.intern(Type::Function { params, return_ });
        }
        // Function type: a top-level `=>` (`() => User`, TS/JS, Scala `T => R`)
        // or `->` (Rust `Fn() -> T`, Kotlin/Swift `(T) -> R`) marks a callable.
        // Both arrows are 2 bytes, so the return is `trimmed[arrow + 2..]`. The
        // params are the LAST top-level parenthesized group in `trimmed[..arrow]`
        // (`(value: T)`, Rust `Fn(T)` / `impl Fn(T)`), comma-split with each
        // piece's `name:` annotation peeled (the bare type is what binds T).
        // Checked before the generic-bracket search so a function type with
        // generic params/return isn't mis-read as an application.
        if let Some(arrow) = find_top_level_arrow(trimmed) {
            let return_ = self.intern_type_str(trimmed[arrow + 2..].trim());
            let pre = trimmed[..arrow].trim();
            let params = match pre.char_indices().find(|&(_, c)| c == '(') {
                Some((open, _)) => match find_matching_close(&pre[open..], '(', ')') {
                    Some(close_rel) => {
                        let inner = &pre[open + 1..open + close_rel];
                        split_depth_zero_commas(inner)
                            .iter()
                            .map(|piece| self.intern_type_str(strip_param_name(piece)))
                            .collect()
                    }
                    None => Vec::new(),
                },
                None => Vec::new(),
            };
            return self.intern(Type::Function { params, return_ });
        }
        // `readonly T[]` — the modifier doesn't change the array shape; strip it
        // so the suffix and the bare form converge.
        let trimmed = trimmed.strip_prefix("readonly ").map(str::trim).unwrap_or(trimmed);
        // Trailing nullable marker (`Action<T>?`, `string?` — C#/Kotlin/Swift).
        // A conditional type's `?` is interior, never trailing, so the suffix
        // strip is unambiguous.
        if let Some(inner) = trimmed.strip_suffix('?') {
            let inner = inner.trim_end();
            if !inner.is_empty() {
                let i = self.intern_type_str(inner);
                return self.intern(Type::Optional(i));
            }
        }
        // `T[]` array suffix → the lib `Array<T>` so member calls (map / push /
        // …) root on the Array type. Checked before the generic-bracket search
        // so `User[]` becomes `Apply(Array, [User])` rather than collapsing to
        // the bare head (an empty `[]` group). A repeated suffix nests (`T[][]` →
        // `Array<Array<T>>`); a bare `[]` or a tuple (`[A, B]`) has no single
        // element and falls through to the bracket parse below.
        if let Some(elem) = trimmed.strip_suffix("[]") {
            let elem = elem.trim();
            if !elem.is_empty() {
                let inner = self.intern_type_str(elem);
                let base = self.class("Array");
                return self.intern(Type::Apply { base, args: vec![inner] });
            }
        }
        // Union / intersection: a top-level `|` (union) or `&` (intersection) at
        // bracket depth 0 makes this a composite type, not a nominal class. `|`
        // binds looser than `&` (TS: `A & B | C` == `(A & B) | C`), so the union
        // split runs first and each arm is re-interned — an arm may itself be an
        // intersection. A leading operator (TS pretty-prints unions as `| A | B`)
        // leaves an empty first piece that the split drops; a single arm (no
        // top-level operator) returns `None` and falls through to the nominal
        // parse. The byte-0 `&` reference sigil is already peeled above, so only
        // an interior `&` reaches here.
        if let Some(arms) = split_top_level(trimmed, '|') {
            let args = arms.iter().map(|a| self.intern_type_str(a)).collect();
            return self.intern(Type::Union(args));
        }
        if let Some(arms) = split_top_level(trimmed, '&') {
            let args = arms.iter().map(|a| self.intern_type_str(a)).collect();
            return self.intern(Type::Intersection(args));
        }
        // Tuple `[A, B, …]` — fully bracket-enclosed with two or more depth-0
        // elements. A `T[]` array suffix is handled above; a single-element `[T]`
        // and a leading-bracket generic (Scala `List[T]`) fall through to the
        // bracket parse below. Each element drops a `label:` prefix (labeled
        // tuple `[get: A, set: B]`) so the element type is what interns.
        if let Some(inner) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let elems = split_depth_zero_commas(inner);
            if elems.len() >= 2 {
                let ids = elems
                    .iter()
                    .map(|e| self.intern_type_str(strip_tuple_label(e)))
                    .collect();
                return self.intern(Type::Tuple(ids));
            }
            // Rust fixed-size array `[T; N]` / slice `[T]` — bracket-enclosed
            // with no top-level comma. A depth-0 `;` separates the element
            // type from the length (`[T; N]`); its absence means a slice
            // (`[T]`). Both are homogeneous single-element sequences, so both
            // collapse to the same canonical `Array<T>` application the
            // `T[]` suffix above already mints — `array_element_type`
            // (engine/chain.rs) projects the element the same way either
            // shape arrives.
            let elem_text = match find_depth_zero_semicolon(inner) {
                Some(semi) => inner[..semi].trim(),
                None => inner.trim(),
            };
            if !elem_text.is_empty() {
                let elem = self.intern_type_str(elem_text);
                let base = self.class("Array");
                return self.intern(Type::Apply { base, args: vec![elem] });
            }
        }
        // Locate the first generic-open at depth 0. Accept both `<` and
        // `[` so Scala / OCaml-style param brackets resolve too.
        let (open_idx, open_char, close_char) = {
            let mut found = None;
            for (i, ch) in trimmed.char_indices() {
                if ch == '<' {
                    found = Some((i, '<', '>'));
                    break;
                }
                if ch == '[' {
                    found = Some((i, '[', ']'));
                    break;
                }
            }
            match found {
                Some(x) => x,
                None => return self.class(trimmed),
            }
        };
        // Find matching close at depth 0 starting from `open_idx`.
        let rest = &trimmed[open_idx..];
        let Some(close_rel) = find_matching_close(rest, open_char, close_char) else {
            return self.class(trimmed);
        };
        let head = trimmed[..open_idx].trim();
        if head.is_empty() {
            return self.class(trimmed);
        }
        // Reject anything after the closing bracket (chained generics like
        // `Foo<Bar>.Baz` aren't first-class here — fallback to Class).
        let tail = trimmed[open_idx + close_rel + 1..].trim();
        if !tail.is_empty() {
            return self.class(trimmed);
        }
        let inner = &trimmed[open_idx + 1..open_idx + close_rel];
        // A `'`-led argument is a lifetime parameter (`'a`, `'static`), never a
        // type — drop it so a wrapper whose leading param is a lifetime
        // (`Cow<'a, str>`) decomposes to the type args alone. No language has a
        // type whose name starts with `'`, so this is universally sound; an
        // all-lifetime arg list collapses to the bare base class below.
        let arg_strs: Vec<String> = split_depth_zero_commas(inner)
            .into_iter()
            .filter(|a| !a.trim_start().starts_with('\''))
            .collect();
        if arg_strs.is_empty() {
            // Empty `Foo<>`, or only lifetime args — treat as plain class.
            return self.class(head);
        }
        let args: Vec<TypeId> = arg_strs.iter().map(|a| self.intern_type_str(a)).collect();
        let base = self.class(head);
        self.intern(Type::Apply { base, args })
    }
}

/// Parse the fixed-arity subset of Python's `Callable` annotation. This
/// intentionally recognizes only the standard unqualified, `typing.`, and
/// `collections.abc.` spellings, all with exactly two outer arguments and an
/// explicit bracketed parameter list. Other aliases are nominal types: their
/// expansion needs declaration identity rather than text parsing.
fn parse_python_callable_type(s: &str) -> Option<(Vec<String>, String)> {
    let open = s.find('[')?;
    let head = s[..open].trim();
    if !matches!(
        head,
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
    let parameter_list = args[0].trim();
    let return_ = args[1].trim();
    let inner = parameter_list.strip_prefix('[')?.strip_suffix(']')?;
    if return_.is_empty()
        || has_unmodeled_callable_identity(parameter_list)
        || has_unmodeled_callable_identity(return_)
    {
        return None;
    }
    let params = split_depth_zero_commas(inner);
    if params
        .iter()
        .any(|param| has_unmodeled_callable_identity(param))
    {
        return None;
    }
    Some((params, return_.to_string()))
}

/// These spellings require a binder or calling-convention model. A source
/// generic parameter such as `T` is left to the later source-aware generic
/// rebinding path; these forms expose no corresponding declaration identity.
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

/// Dart function-type slots may retain their source declaration name
/// (`User user`, `void Function(User) callback`). Remove only one trailing
/// depth-zero identifier; an unnamed composite type such as
/// `void Function(User)` ends in `)` and remains untouched.
fn strip_dart_function_param_name(param: &str) -> &str {
    let mut depth = 0i32;
    let mut last_whitespace = None;
    for (index, ch) in param.char_indices() {
        match ch {
            '<' | '[' | '{' | '(' => depth += 1,
            '>' | ']' | '}' | ')' => depth -= 1,
            ch if ch.is_whitespace() && depth == 0 => last_whitespace = Some(index),
            _ => {}
        }
    }
    let Some(split) = last_whitespace else {
        return param.trim();
    };
    let type_ = param[..split].trim_end();
    let name = param[split..].trim();
    if !type_.is_empty() && is_dart_identifier(name) {
        type_
    } else {
        param.trim()
    }
}

fn is_dart_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some('_' | 'a'..='z' | 'A'..='Z'))
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

/// Parse Dart's return-first function type (`R Function(A, B)`) without
/// treating a nested `Function(...)` inside another type as a top-level type.
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
                let close_rel = find_matching_close(&s[open..], '(', ')')?;
                let close = open + close_rel;
                if !s[close + 1..].trim().is_empty() {
                    return None;
                }
                let params = split_depth_zero_commas(&s[open + 1..close]);
                return Some((params, return_.to_string()));
            }
            _ => {}
        }
    }
    None
}

/// Parse the representable subset of a Go function type. The source grammar
/// permits named and grouped names (`func(a, b T) R`); names are erased here so
/// each callback position keeps its declared type. Variadics and parenthesized
/// result groups intentionally fall through as opaque nominal text.
fn parse_go_function_type(s: &str) -> Option<(Vec<String>, String)> {
    let rest = s.strip_prefix("func")?.trim_start();
    if !rest.starts_with('(') {
        return None;
    }
    let close = find_matching_close(rest, '(', ')')?;
    let return_ = rest[close + 1..].trim();
    if return_.is_empty() || return_.starts_with('(') {
        return None;
    }
    // `func (recv T) Method(U) R` is a declaration signature, not a function
    // type. A legitimate function-valued result begins with the `func` keyword;
    // another ordinary identifier immediately followed by `(` is a method
    // name, so fail closed instead of structurally misclassifying it.
    if go_declaration_tail(return_) {
        return None;
    }
    let params = parse_go_function_params(&rest[1..close])?;
    Some((params, return_.to_string()))
}

fn go_declaration_tail(tail: &str) -> bool {
    let name_len = tail
        .bytes()
        .take_while(|byte| *byte == b'_' || byte.is_ascii_alphanumeric())
        .count();
    let name = &tail[..name_len];
    !name.is_empty() && name != "func" && tail[name_len..].trim_start().starts_with('(')
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
            // A no-space item is an unnamed type unless a following named
            // item closes its name group (`a, b T`). Keeping it pending lets
            // both source forms remain unambiguous without type lookup.
            grouped_names.push(part.to_string());
        }
    }
    params.extend(grouped_names);
    Some(params)
}

/// The type suffix of a single named Go function parameter. A nested function
/// type has its own spaces (`func(T) R`), so only consider a name/type split
/// when the left side is a plain identifier and the right side is non-empty.
fn go_named_parameter_type(part: &str) -> Option<&str> {
    let mut depth = 0i32;
    for (index, ch) in part.char_indices() {
        match ch {
            '<' | '[' | '{' | '(' => depth += 1,
            '>' | ']' | '}' | ')' => depth -= 1,
            ch if ch.is_whitespace() && depth == 0 => {
                let name = part[..index].trim();
                let type_ = part[index..].trim();
                if is_go_identifier(name) && !type_.is_empty() {
                    return Some(type_);
                }
                return None;
            }
            _ => {}
        }
    }
    None
}

fn is_go_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some('_' | 'a'..='z' | 'A'..='Z'))
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}
