// =============================================================================
// engine/type_symbol — TypeSymbol: a type as it flows through a chain
//
// Roslyn's ITypeSymbol. The chain binder threads a TypeSymbol, never a bare
// type string: a constructed type carries its applied type arguments, so a
// member yielded through it can be substituted — `Repository<User>.find()` whose
// declared return is `T` yields `User`, not the unbound parameter.
//
// The only string is the type's qualified-name head; the structure (the applied
// arguments, recursively) is the entity. Type expressions enter as strings at
// the index boundary (a stored annotation, a forward-inferred local) and are
// parsed into a TypeSymbol once, at the edge — the walk itself is entity-only.
// =============================================================================

/// A type reference carried through a member-access chain: the declared type's
/// qualified-name head plus its applied type arguments. Non-generic types carry
/// no arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeSymbol {
    /// The type's qualified-name head — `Repository` for `Repository<User>`.
    pub qname: String,
    /// Applied type arguments — `[User]` for `Repository<User>`; empty otherwise.
    pub type_args: Vec<TypeSymbol>,
}

impl TypeSymbol {
    /// A non-generic type.
    pub fn plain(qname: impl Into<String>) -> Self {
        Self {
            qname: qname.into(),
            type_args: Vec::new(),
        }
    }

    /// Parse a type expression — `"Repository<User>"`, `"Map<K, List<V>>"`,
    /// `"User"` — into a structured TypeSymbol. The argument split is
    /// depth-aware and the args keep their full nested form, so a nested
    /// application recurses into its own TypeSymbol (and its parameters stay
    /// substitutable). A type with no application parses to a plain TypeSymbol.
    pub fn parse(expr: &str) -> Self {
        let expr = expr.trim();
        let Some(open) = expr.find('<') else {
            return TypeSymbol::plain(expr);
        };
        let head = expr[..open].trim();
        match matching_close(&expr[open..]) {
            Some(close_rel) => TypeSymbol {
                qname: head.to_string(),
                type_args: split_top_level(&expr[open + 1..open + close_rel])
                    .into_iter()
                    .map(TypeSymbol::parse)
                    .collect(),
            },
            // Unbalanced `<` — treat the whole expression as a plain head.
            None => TypeSymbol::plain(expr),
        }
    }

    /// Substitute a declaring type's generic parameters with a receiver's
    /// applied type arguments throughout `self`. `params` and `args` are
    /// index-aligned (`["T"]` ↔ `[User]`): a node whose head is a parameter is
    /// replaced whole by the matching argument; otherwise the substitution
    /// recurses into the type arguments (`List<T>` with `T→User` → `List<User>`).
    /// A head that matches no parameter is left untouched.
    pub fn substitute(self, params: &[String], args: &[TypeSymbol]) -> TypeSymbol {
        if let Some(i) = params.iter().position(|p| p == &self.qname) {
            if let Some(arg) = args.get(i) {
                return arg.clone();
            }
        }
        TypeSymbol {
            qname: self.qname,
            type_args: self
                .type_args
                .into_iter()
                .map(|a| a.substitute(params, args))
                .collect(),
        }
    }
}

/// Byte offset of the `>` matching the leading `<` in `s` (which must begin at
/// `<`). Depth-tracked so `Map<K, List<V>>` matches the final `>`, not the
/// inner one. `None` when the brackets are unbalanced. Brackets are ASCII, so
/// the char offset is a valid byte offset for slicing.
fn matching_close(s: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Split bracket-content on top-level commas, depth-tracked across `<>`, `[]`
/// and `()`, returning each argument's full (possibly nested) text, trimmed.
/// Empty arguments are dropped.
fn split_top_level(inner: &str) -> Vec<&str> {
    let mut args = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0usize;
    for (i, c) in inner.char_indices() {
        match c {
            '<' | '[' | '(' => depth += 1,
            '>' | ']' | ')' => depth = (depth - 1).max(0),
            ',' if depth == 0 => {
                let arg = inner[start..i].trim();
                if !arg.is_empty() {
                    args.push(arg);
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    let arg = inner[start..].trim();
    if !arg.is_empty() {
        args.push(arg);
    }
    args
}

#[cfg(test)]
#[path = "type_symbol_tests.rs"]
mod tests;
