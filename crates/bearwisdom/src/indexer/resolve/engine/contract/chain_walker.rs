// =============================================================================
// indexer/resolve/engine/chain_walker.rs — type-inference chain walker
//
// Free functions that walk a MemberChain (or a single qname) and infer the
// type the chain yields. Used by the resolver to:
//   * Detect that a chain leaves project-internal territory and classify the
//     unresolved type as external (infer_external_from_chain).
//   * Propagate forward type inference across binding sites
//     (infer_type_from_chain).
//   * Resolve a type name relative to a scope chain
//     (resolve_type_name_in_scope).
//
// Plus the small string utilities the chain walker needs: find_matching_bracket,
// tuple_element, parse_return_type_from_signature.
// =============================================================================

use std::collections::BTreeMap;

use rustc_hash::FxHashMap;


use super::{Symbol, SymbolLookup, TypeInfo};

/// Return the first generic argument of `T<A, B, …>` (depth-aware), or `None`
/// when `s` carries no top-level generic application. Used to peel a fallible
/// wrapper's payload after a `?`/unwrap: `Result<IndexReader>` → `IndexReader`,
/// `Result<Field, Error>` → `Field`.
pub(crate) fn first_generic_arg(s: &str) -> Option<String> {
    let open = s.find('<')?;
    let close_rel = find_matching_bracket(&s[open..], '<', '>')?;
    let inner = &s[open + 1..open + close_rel];
    let mut depth = 0usize;
    for (i, c) in inner.char_indices() {
        match c {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let arg = inner[..i].trim();
                return (!arg.is_empty()).then(|| arg.to_string());
            }
            _ => {}
        }
    }
    let arg = inner.trim();
    (!arg.is_empty()).then(|| arg.to_string())
}

/// Find the index of the closing bracket that matches the first opening bracket
/// in `s`.  Uses depth counting so nested brackets are handled correctly.
///
/// Example: `find_matching_bracket("Map<K, List<V>>", '<', '>')` → `Some(14)`.
/// `s.find('>')` would incorrectly return `Some(11)` for this input.
pub(crate) fn find_matching_bracket(s: &str, open: char, close: char) -> Option<usize> {
    let mut depth = 0usize;
    for (i, c) in s.char_indices() {
        if c == open {
            depth += 1;
        } else if c == close {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// Parse the body of a generic-parameter clause — the text between the
/// `<>` / `[]` brackets — into `(name, optional-upper-bound)` pairs.
///
/// Recognizes three bound spellings: `T extends Animal` (TS, Java, ...),
/// `T: Animal` (Rust, Scala `<:`, ...), and Go's space-separated
/// `[T Constraint]`. The first nominal token is the bound; multi-bounds
/// (`T: A + B`) and defaults (`T extends X = Y`) collapse to the first segment,
/// and higher-kinded markers (`F[_]`) carry no bound. Declaration-site variance
/// (`out T` / `in T`, C#/Kotlin) drops the keyword so the name is the parameter
/// and the param stays unbounded. Params are split on plain commas — a bound
/// that itself contains a comma (`T extends Map<K, V>`) truncates, which fails
/// closed at lookup rather than producing a wrong member. The name output is
/// identical to a name-only parse so generic-param positions stay aligned with
/// `type_args`. A `where`-clause bound (C#, Rust) lives outside this clause; see
/// `merge_where_bounds`.
pub(crate) fn parse_generic_param_clause(clause: &str) -> Vec<(String, Option<String>, Option<String>)> {
    clause
        .split(',')
        .filter_map(|part| {
            let part = part.trim();
            // Strip a leading variance keyword: `out T` / `in T` name the param
            // after the keyword and carry no bound.
            let (body, variance) = match part
                .strip_prefix("out ")
                .or_else(|| part.strip_prefix("in "))
            {
                Some(rest) => (rest.trim_start(), true),
                None => (part, false),
            };
            let name = body
                .split(|c: char| c == '[' || c == '<' || c == ':' || c == '=')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            if name.is_empty() {
                return None;
            }
            // Default type: the text after `=` (a param's `extends` bound precedes
            // it: `K extends Q = Q` → "Q"). Drives binding a param the call site
            // leaves unbound to an earlier param (`TData = TQueryFnData`) or a
            // concrete type. The bound parser below strips this `= …` tail.
            let default = body
                .split_once('=')
                .map(|(_, d)| d.trim().to_string())
                .filter(|d| !d.is_empty());
            if variance {
                return Some((name, None, default));
            }
            let bound = body
                .find(" extends ")
                .map(|i| &body[i + " extends ".len()..])
                .or_else(|| body.find(':').map(|i| &body[i + 1..]))
                .map(|b| b.split(|c: char| c == '=' || c == '+').next().unwrap_or(b).trim())
                .filter(|b| !b.is_empty())
                .map(|b| b.to_string())
                // Go `[T Constraint]`: the constraint is a space-separated
                // second token, with no `extends`/`:` separator. Require an
                // identifier start so a TS default (`T = string`) doesn't read
                // its `=` as the bound.
                .or_else(|| {
                    body.split_whitespace()
                        .nth(1)
                        .filter(|t| t.starts_with(|c: char| c.is_alphabetic() || c == '_'))
                        .map(str::to_string)
                });
            Some((name, bound, default))
        })
        .collect()
}

/// Fold `where T : B` / `where T: B` constraint-clause bounds into params
/// already parsed from the bracket clause. C# and Rust place a generic
/// parameter's bound in a trailing `where` clause that sits outside the
/// `<>`/`[]` the bracket parser sees. A `where` bound only fills a param whose
/// inline bound was absent — an inline bound is the more local declaration and
/// wins. C# special constraints (`class`, `struct`, `new()`, `unmanaged`,
/// `notnull`) and lifetimes are not member-bearing types and are skipped, so
/// such a param stays unbounded (member lookup then fails closed).
pub(crate) fn merge_where_bounds(
    params: &mut [(String, Option<String>, Option<String>)],
    sig: &str,
) {
    let Some(region) = where_clause_region(sig) else {
        return;
    };
    for (name, bound, _default) in params.iter_mut() {
        if bound.is_none() {
            *bound = where_bound_for(region, name);
        }
    }
}

/// The text following the first standalone `where` keyword in a signature, or
/// `None` when there is no `where` clause. The keyword must sit at identifier
/// boundaries so a substring (`somewhere`) or a type named `Where` is not
/// mistaken for the clause.
fn where_clause_region(sig: &str) -> Option<&str> {
    let bytes = sig.as_bytes();
    let mut from = 0;
    while let Some(rel) = sig[from..].find("where") {
        let idx = from + rel;
        let after = idx + "where".len();
        let before_boundary = idx == 0 || !is_ident_byte(bytes[idx - 1]);
        let after_boundary = bytes.get(after).map_or(true, |&b| !is_ident_byte(b));
        if before_boundary && after_boundary {
            return Some(sig[after..].trim_start());
        }
        from = after;
    }
    None
}

/// First nominal bound declared for `name` within a `where` region.
fn where_bound_for(region: &str, name: &str) -> Option<String> {
    let constraints = constraints_for(region, name)?;
    for seg in constraints.split(|c| c == ',' || c == '+') {
        let t = seg.trim();
        if t.is_empty() || t.starts_with('\'') || is_special_constraint(t) {
            continue;
        }
        return Some(t.to_string());
    }
    None
}

/// The constraint list declared for `name` in a `where` region: the text after
/// `name :`, up to the next `where` clause (C# uses one `where` per param) or
/// the region end. `None` when `name` heads no predicate. `name` must appear at
/// identifier boundaries followed by `:` so a bound mention of the same text
/// (`U : T`, `IList<T>`) is not mistaken for `T`'s own predicate.
fn constraints_for<'a>(region: &'a str, name: &str) -> Option<&'a str> {
    let bytes = region.as_bytes();
    let mut from = 0;
    while let Some(rel) = region[from..].find(name) {
        let idx = from + rel;
        let after = idx + name.len();
        let before_boundary = idx == 0 || !is_ident_byte(bytes[idx - 1]);
        let rest = region[after..].trim_start();
        if before_boundary && rest.starts_with(':') {
            let list = rest[1..].trim_start();
            let end = list.find(" where ").unwrap_or(list.len());
            return Some(list[..end].trim_end());
        }
        from = after;
    }
    None
}

fn is_special_constraint(t: &str) -> bool {
    matches!(t, "class" | "struct" | "unmanaged" | "notnull" | "default") || t.starts_with("new(")
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Lightweight chain resolution for variable type inference during index building.
/// Uses the already-built type_info map (not the full SymbolLookup trait).
/// Given a type string that looks like a TypeScript tuple literal
/// (`[A, B, C]` where commas are at bracket/angle depth zero), return the
/// Nth element. Returns `None` if `raw` isn't a tuple, if `idx` is out of
/// range, or if the brackets are unbalanced.
///
/// Used by `infer_type_from_chain` to resolve array-pattern destructuring:
/// `const [a, b] = useState<T>()` walks to `useState`'s return type
/// `[T, Dispatch<SetStateAction<T>>]`, then slices index 0 or 1.
///
/// Nested tuples/generics are handled by tracking `<>`/`[]`/`()` depth so
/// commas inside `Dispatch<SetStateAction<T>>` or `[inner, tuple]` aren't
/// split. This is a pure string operation — the walker's TypeEnvironment
/// handles the subsequent generic substitution separately.
pub(crate) fn tuple_element(raw: &str, idx: usize) -> Option<String> {
    let trimmed = raw.trim();
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?;
    let mut depth_angle = 0i32;
    let mut depth_square = 0i32;
    let mut depth_paren = 0i32;
    let mut current = String::new();
    let mut parts: Vec<String> = Vec::new();
    for ch in inner.chars() {
        match ch {
            '<' => depth_angle += 1,
            '>' => depth_angle -= 1,
            '[' => depth_square += 1,
            ']' => depth_square -= 1,
            '(' => depth_paren += 1,
            ')' => depth_paren -= 1,
            ',' if depth_angle == 0 && depth_square == 0 && depth_paren == 0 => {
                parts.push(current.trim().to_string());
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(ch);
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    parts.into_iter().nth(idx)
}

/// Extract a return type from a method signature string of the shape
/// `{name}{gp}(params): ReturnType`. Returns the substring after the
/// last top-level `):` separator, trimmed. Returns `None` if the shape
/// doesn't match (no parens, no colon after the closing paren, etc).
///
/// Top-level tracking: the colon must be at paren depth 0 so `(): ret`
/// inside a nested function type like `(): () => void` doesn't get
/// captured by mistake.
///
/// Used by the TypeInfo builder to populate return_type for synthetic
/// .NET DLL metadata symbols that have no TypeRef edges but do carry
/// a dotscope-formatted signature string.
/// Resolve a raw type-name reference (as written in source, e.g. `Dayjs`)
/// against the set of known fully-qualified names in the index, using the
/// referring symbol's `scope_path` as the walk starting point.
///
/// The algorithm mirrors how TypeScript / C# / Java name resolution works:
/// when we see `Dayjs` inside `namespace dayjs { class Dayjs { clone(): Dayjs } }`,
/// the symbol's scope_path is `dayjs.Dayjs`. We try, in order:
///
///   1. `dayjs.Dayjs.Dayjs` — "Dayjs" in the innermost scope (class)
///   2. `dayjs.Dayjs`       — "Dayjs" in the parent scope (namespace)
///   3. `dayjs`             — "Dayjs" at root (won't match unless global)
///   4. `Dayjs`             — the raw text itself (top-level / file-scope)
///
/// The first candidate present in `by_qname` wins. Falls back to the raw
/// name when no scope-qualified form matches — that preserves historical
/// behaviour for languages / cases where scope-path wasn't populated or
/// where the type really is a top-level name.
///
/// Pre-qualified targets (containing a dot already, like `dayjs.Dayjs`)
/// are returned as-is since they've already been resolved upstream.
pub(crate) fn resolve_type_name_in_scope(
    raw: &str,
    scope_path: Option<&str>,
    by_qname: &BTreeMap<String, Symbol>,
) -> String {
    // Extractor-emitted FQNs flow through unchanged.
    if raw.contains('.') {
        return raw.to_string();
    }
    let Some(scope) = scope_path else {
        return raw.to_string();
    };
    // Walk scope_path outward: "dayjs.Dayjs" → ["dayjs.Dayjs", "dayjs", ""].
    let mut cur: &str = scope;
    loop {
        let candidate = if cur.is_empty() {
            raw.to_string()
        } else {
            format!("{cur}.{raw}")
        };
        if by_qname.contains_key(&candidate) {
            return candidate;
        }
        if cur.is_empty() {
            break;
        }
        match cur.rfind('.') {
            Some(idx) => cur = &cur[..idx],
            None => cur = "",
        }
    }
    raw.to_string()
}

/// Convenience wrapper: TS/.NET-shaped signature parsing. Kept for sites
/// that don't have a language id handy.
pub(crate) fn parse_param_types_from_signature(sig: &str) -> Option<Vec<String>> {
    parse_param_types_from_signature_for_lang(sig, "")
}

/// Parse the members of an inline object type — `{ a: A; b: B }` — into
/// `(name, type)` pairs. Members are separated by `;`, `,`, or newline at brace
/// depth 0; each member is `name: Type` with an optional `?`/`readonly`/`get`
/// prefix, or a method `name(args): R` (yielding `(name, R)`). Index signatures
/// (`[k: string]: V`), spreads, and call/construct signatures are skipped.
///
/// Nested object types, generic args, tuples, and parameter lists are kept whole
/// (depth-tracked), so `p: Promise<X>` and `o: { a: A }` parse to the full type
/// text. Returns empty when `s` is not a `{ … }` object type.
pub(crate) fn parse_object_type_members(s: &str) -> Vec<(String, String)> {
    let t = s.trim();
    let Some(inner) = t.strip_prefix('{').and_then(|x| x.strip_suffix('}')) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0usize;
    let bytes = inner.as_bytes();
    // Top-level entries: split on `;` / `,` / newline at bracket depth 0.
    let mut push_entry = |slice: &str, out: &mut Vec<(String, String)>| {
        let e = slice.trim();
        if e.is_empty() {
            return;
        }
        // Skip index signatures, spreads, and call/construct signatures.
        if e.starts_with('[') || e.starts_with("...") || e.starts_with('(') || e.starts_with("new ")
        {
            return;
        }
        // Top-level `:` separating the (possibly method-headed) name from the type.
        let eb = e.as_bytes();
        let mut d: i32 = 0;
        let mut colon: Option<usize> = None;
        for (i, &b) in eb.iter().enumerate() {
            match b {
                b'{' | b'(' | b'<' | b'[' => d += 1,
                b'}' | b')' | b'>' | b']' => d -= 1,
                b':' if d == 0 => {
                    colon = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let Some(ci) = colon else { return };
        let type_part = e[ci + 1..].trim();
        if type_part.is_empty() {
            return;
        }
        // Name: drop a method param list / generic clause, a trailing `?`, and
        // any `readonly`/`get`/`set` modifier — the member name is the last
        // identifier of what remains.
        let head = e[..ci].split('(').next().unwrap_or("");
        let head = head.split('<').next().unwrap_or(head);
        let name = head.trim().trim_end_matches('?').split_whitespace().last().unwrap_or("");
        if name.is_empty()
            || !name
                .chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
        {
            return;
        }
        out.push((name.to_string(), type_part.to_string()));
    };
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'{' | b'(' | b'<' | b'[' => depth += 1,
            b'}' | b')' | b'>' | b']' => depth -= 1,
            b';' | b',' | b'\n' if depth == 0 => {
                push_entry(&inner[start..i], &mut out);
                start = i + 1;
            }
            _ => {}
        }
    }
    push_entry(&inner[start..], &mut out);
    out
}

/// Parse the declared type out of a field / property / variable /
/// parameter signature. Recognised shapes:
///   - TS / Kotlin / Swift / Scala / Python / Rust style:
///       `name: Type` or `name: Type = default` — take what's between
///       the first top-level `:` and the next `=` (or end of string).
///   - Go style: `name Type` — take the last whitespace-separated token
///     when the signature has no `:`.
///   - C / C++ / Java / C# / VB.NET style: `Type name` — take everything
///     before the last whitespace at top level when no `:` exists.
///
/// Returns `None` when no usable type can be extracted (signature is
/// empty / has no whitespace / has only the name).
pub(crate) fn parse_declared_type_from_signature_for_lang(
    sig: &str,
    lang_id: &str,
) -> Option<String> {
    let trimmed = sig.trim();
    if trimmed.is_empty() {
        return None;
    }
    // First: top-level `:` always means a TS-shaped annotation, regardless
    // of language id. Some extractors emit `name: Type` even in Java
    // signatures, so we honour the colon when present.
    let mut depth: i32 = 0;
    for (i, ch) in trimmed.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' | ']' | ')' | '}' => depth -= 1,
            ':' if depth == 0 => {
                let after = trimmed[i + 1..].trim();
                let ty = after[..initializer_split(after)].trim();
                if ty.is_empty() {
                    return None;
                }
                return Some(ty.to_string());
            }
            _ => {}
        }
    }
    // No `:`. Dispatch per language for the postfix vs prefix shape.
    match lang_id {
        "go" => {
            // `name Type` — last whitespace-separated token.
            let mut depth: i32 = 0;
            let mut last_ws: Option<usize> = None;
            for (i, ch) in trimmed.char_indices() {
                match ch {
                    '<' | '[' | '(' | '{' => depth += 1,
                    '>' | ']' | ')' | '}' => depth -= 1,
                    c if c.is_whitespace() && depth == 0 => last_ws = Some(i),
                    _ => {}
                }
            }
            last_ws.map(|ws| trimmed[ws + 1..].trim().to_string())
        }
        "c" | "c_lang" | "cpp" | "java" | "csharp" | "vbnet" => {
            // `Type name` — everything before the last whitespace.
            let mut depth: i32 = 0;
            let mut last_ws: Option<usize> = None;
            for (i, ch) in trimmed.char_indices() {
                match ch {
                    '<' | '[' | '(' | '{' => depth += 1,
                    '>' | ']' | ')' | '}' => depth -= 1,
                    c if c.is_whitespace() && depth == 0 => last_ws = Some(i),
                    _ => {}
                }
            }
            last_ws.map(|ws| trimmed[..ws].trim().to_string())
        }
        _ => None,
    }
}

/// The byte offset where a `Type = default` annotation's initializer begins —
/// the first depth-0 `=` that is not the `=` of a `=>` arrow. An `=` inside a
/// bracket group (`<T = X>` generic defaults, `(a = 1)` parameter defaults) is
/// part of the annotation, not an initializer. Returns `s.len()` when no
/// initializer is present.
fn initializer_split(s: &str) -> usize {
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'<' | b'[' | b'(' | b'{' => depth += 1,
            b'>' | b']' | b')' | b'}' => depth -= 1,
            b'=' if i + 1 < bytes.len() && bytes[i + 1] == b'>' => {
                // A function-type arrow — skip both bytes so the `>` doesn't
                // decrement the bracket depth.
                i += 2;
                continue;
            }
            b'=' if depth == 0 => return i,
            _ => {}
        }
        i += 1;
    }
    s.len()
}

/// Per-language parameter-type extraction. Recognized shapes:
///   - TS / TSX / JSX / JS / Kotlin / Swift / Scala / Python / Rust /
///     Dart / Haskell / OCaml / F#:
///       `name(arg: Type, …): Ret` — type lives after the `:` in each arg.
///   - Go: `name(a Type, b Type) Ret` — type lives after a whitespace
///     in each arg (postfix, no `:`).
///   - C / C++ / Java / C#: `name(Type a, Type b)` — type lives before
///     the variable name (prefix). For these we take all-but-last token.
///   - Empty `lang_id` or unknown languages: fall back to the bare-type
///     reading (.NET DLL metadata style).
///
/// Returns `None` when the signature has no balanced parens, `Some(vec![])`
/// for zero-arg calls. Bracket-aware top-level comma split so generic
/// args don't fragment.
pub(crate) fn parse_param_types_from_signature_for_lang(
    sig: &str,
    lang_id: &str,
) -> Option<Vec<String>> {
    let bytes = sig.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    // A Go struct method (`func (recv) Name(params) result`) leads with a
    // RECEIVER paren group; its params live in the SECOND top-level group. Skip
    // the receiver so params aren't read off `(r *Repo)`. An interface
    // method_elem (`Name(params) result`) has no `func`/receiver, so the first
    // group is already the param list.
    let groups_to_skip = (lang_id == "go" && sig.trim_start().starts_with("func")) as usize;
    // Locate the param-list group's opening `(`: forward-scan for the
    // (groups_to_skip+1)-th `(` at depth 0 across `<` and `[` so generic /
    // index args don't fool us.
    let mut depth_angle: i32 = 0;
    let mut depth_square: i32 = 0;
    let mut groups_skipped = 0usize;
    let mut open_idx: Option<usize> = None;
    let mut paren_depth: i32 = 0;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'<' => depth_angle += 1,
            b'>' => depth_angle -= 1,
            b'[' => depth_square += 1,
            b']' => depth_square -= 1,
            b'(' if depth_angle == 0 && depth_square == 0 && paren_depth == 0 => {
                if groups_skipped == groups_to_skip {
                    open_idx = Some(i);
                    break;
                }
                paren_depth += 1;
            }
            b'(' => paren_depth += 1,
            b')' => {
                paren_depth -= 1;
                if paren_depth == 0 {
                    groups_skipped += 1;
                }
            }
            _ => {}
        }
    }
    let open = open_idx?;
    // Forward-scan from open to find the matching `)` at paren-depth 1.
    let mut depth_paren: i32 = 0;
    let mut depth_angle: i32 = 0;
    let mut depth_square: i32 = 0;
    let mut close_idx: Option<usize> = None;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'(' => depth_paren += 1,
            b')' => {
                depth_paren -= 1;
                if depth_paren == 0 {
                    close_idx = Some(i);
                    break;
                }
            }
            b'<' => depth_angle += 1,
            b'>' => depth_angle -= 1,
            b'[' => depth_square += 1,
            b']' => depth_square -= 1,
            _ => {}
        }
    }
    let close = close_idx?;
    let inner = &sig[open + 1..close];
    if inner.trim().is_empty() {
        return Some(Vec::new());
    }
    // Bracket-aware split on top-level commas.
    let mut parts: Vec<String> = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0usize;
    let ibytes = inner.as_bytes();
    for (i, &b) in ibytes.iter().enumerate() {
        match b {
            b'<' | b'[' | b'(' | b'{' => depth += 1,
            b'>' | b']' | b')' | b'}' => depth -= 1,
            b',' if depth == 0 => {
                parts.push(inner[start..i].to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < inner.len() {
        parts.push(inner[start..].to_string());
    }
    // A `this`-marked first parameter is a C# extension-method receiver —
    // unambiguous regardless of `lang_id` (no other signature shape writes
    // `(this Type name`). The receiver is NOT a call argument: drop it so the
    // pattern positions align with the call's actual arguments, and read the
    // remaining slots prefix-shaped.
    let is_extension = parts
        .first()
        .is_some_and(|p| p.trim_start().starts_with("this "));
    if is_extension {
        let types: Vec<String> = parts
            .into_iter()
            .skip(1)
            .map(|p| extract_param_type_prefix(&p))
            .filter(|s| !s.is_empty())
            .collect();
        return Some(types);
    }
    // Dispatch per language. Each strategy extracts the TYPE portion of
    // one parameter slot, after the comma split above.
    let extract = match lang_id {
        "go" => extract_param_type_postfix_no_colon,
        "c" | "c_lang" | "cpp" | "java" | "csharp" | "vbnet" => extract_param_type_prefix,
        _ => extract_param_type_colon_separated,
    };
    let types: Vec<String> = parts
        .into_iter()
        .map(|p| extract(&p))
        .filter(|s| !s.is_empty())
        .collect();
    Some(types)
}

/// The receiver TYPE text of a C#-style extension-method signature — the
/// `this`-marked first parameter of `void UseSnapshot(this ModelBuilder
/// builder, …)`, with the `this` marker and the parameter NAME stripped.
/// `None` when the signature carries no `(this ` marker (not an extension).
pub(crate) fn extension_receiver_type(sig: &str) -> Option<String> {
    let idx = sig.find("(this ")?;
    let rest = &sig[idx + "(this ".len()..];
    // End of the first parameter: the first `,` or `)` at bracket depth 0.
    let mut depth: i32 = 0;
    let mut end = rest.len();
    for (i, ch) in rest.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' | ']' | ')' | '}' if depth > 0 => depth -= 1,
            ',' | ')' if depth == 0 => {
                end = i;
                break;
            }
            _ => {}
        }
    }
    let ty = extract_param_type_prefix(&rest[..end]);
    (!ty.is_empty()).then_some(ty)
}

/// TS / Rust / Python / Kotlin / Swift / Scala / Dart / Haskell / OCaml
/// / F# style: arg looks like `name: Type` (or sometimes `Type` when
/// destructured / unnamed). The type sits after the first top-level `:`.
fn extract_param_type_colon_separated(part: &str) -> String {
    let mut depth: i32 = 0;
    for (i, ch) in part.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' | ']' | ')' | '}' => depth -= 1,
            ':' if depth == 0 => return part[i + 1..].trim().to_string(),
            _ => {}
        }
    }
    part.trim().to_string()
}

/// Go style: arg looks like `name Type` (postfix type, no colon). Take
/// the substring after the last whitespace at top-level — the trailing
/// token is the type, possibly with generic brackets. A leading pointer `*`
/// is stripped so `*User` and `User` intern alike (matching `pointer_type_name`),
/// keeping the structural member-compare's SOURCE/TARGET TypeIds aligned.
fn extract_param_type_postfix_no_colon(part: &str) -> String {
    let trimmed = part.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut depth: i32 = 0;
    let mut last_ws: Option<usize> = None;
    for (i, ch) in trimmed.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' | ']' | ')' | '}' => depth -= 1,
            c if c.is_whitespace() && depth == 0 => last_ws = Some(i),
            _ => {}
        }
    }
    let ty = match last_ws {
        Some(ws) => trimmed[ws + 1..].trim(),
        None => trimmed,
    };
    strip_go_pointer(ty).to_string()
}

/// C / C++ / Java / C# style: arg looks like `Type name` (prefix type).
/// Take everything up to the last whitespace at top-level — that's the
/// type expression, possibly with pointer/reference markers.
fn extract_param_type_prefix(part: &str) -> String {
    // A default value (`Action<T>? configure = null`) is not part of the
    // parameter's type-name pair — strip it at the first depth-0 `=` before
    // taking the type prefix.
    let part = part[..initializer_split(part)].trim_end();
    let trimmed = part.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let mut depth: i32 = 0;
    let mut last_ws: Option<usize> = None;
    for (i, ch) in trimmed.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' | ']' | ')' | '}' => depth -= 1,
            c if c.is_whitespace() && depth == 0 => last_ws = Some(i),
            _ => {}
        }
    }
    match last_ws {
        Some(ws) => trimmed[..ws].trim().to_string(),
        None => trimmed.to_string(),
    }
}

/// Convenience wrapper: TS/.NET/arrow-shaped return parsing. Kept for sites
/// that don't have a language id handy — byte-identical to the no-lang path
/// of `parse_return_type_from_signature_for_lang`.
pub(crate) fn parse_return_type_from_signature(sig: &str) -> Option<String> {
    parse_return_type_from_signature_for_lang(sig, "")
}

/// Per-language return-type extraction from a callable signature string.
///
/// The default (empty / non-Go `lang_id`) recognizes the params-first return
/// spellings: a top-level `):` (TS / .NET) and a top-level `->`/`=>` arrow
/// (Python / Rust / TS). Go has no separator — the result sits bare AFTER the
/// param list — so it gets a dedicated arm: `parse_go_result`. The Go result is
/// returned verbatim, including a parenthesized multi-return group `(int, error)`
/// (so two identical multi-returns intern to the same TypeId and the structural
/// member-compare sees `Yes`), with a single-token result's leading pointer `*`
/// stripped to agree with the rest of the Go engine (`pointer_type_name`).
pub(crate) fn parse_return_type_from_signature_for_lang(
    sig: &str,
    lang_id: &str,
) -> Option<String> {
    if lang_id == "go" {
        return parse_go_result(sig);
    }
    // Find the top-level `):` separator. Scan right-to-left tracking
    // paren depth from zero upward — the FIRST `)` at depth 0 (from
    // the end) followed by `:` marks the return type boundary.
    let bytes = sig.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut depth_paren: i32 = 0;
    let mut depth_angle: i32 = 0;
    let mut depth_square: i32 = 0;
    // Scan right-to-left so we get the outermost close paren.
    for (i, &b) in bytes.iter().enumerate().rev() {
        match b {
            b')' => depth_paren += 1,
            b'(' => depth_paren -= 1,
            b'>' => depth_angle += 1,
            b'<' => depth_angle -= 1,
            b']' => depth_square += 1,
            b'[' => depth_square -= 1,
            b':' if depth_paren == 0 && depth_angle == 0 && depth_square == 0 => {
                // Must be preceded by `)` at position i-1 (or earlier
                // with whitespace). Trim whitespace and verify.
                let before = sig[..i].trim_end();
                if before.ends_with(')') {
                    let after = sig[i + 1..].trim();
                    if !after.is_empty() {
                        return Some(after.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    // Arrow return forms: Python/Rust `-> T`, TS `=> T`. A `>` immediately
    // preceded by `-` or `=` is a return arrow, never a generic close (no type
    // bracket ends in `->`/`=>`), so it doesn't perturb angle depth. Return the
    // type after the rightmost top-level arrow.
    let mut depth_paren: i32 = 0;
    let mut depth_angle: i32 = 0;
    let mut depth_square: i32 = 0;
    for (i, &b) in bytes.iter().enumerate().rev() {
        match b {
            b')' => depth_paren += 1,
            b'(' => depth_paren -= 1,
            b']' => depth_square += 1,
            b'[' => depth_square -= 1,
            b'<' => depth_angle -= 1,
            b'>' => {
                let is_arrow = i > 0 && matches!(bytes[i - 1], b'-' | b'=');
                if is_arrow && depth_paren == 0 && depth_angle == 0 && depth_square == 0 {
                    let mut after = sig[i + 1..].trim();
                    // Drop a Rust block / where-clause and a trailing Python `:`
                    // that follow the return type in the signature text.
                    if let Some(pos) = after.find('{').or_else(|| after.find(';')) {
                        after = after[..pos].trim_end();
                    }
                    if let Some(pos) = after.find(" where ") {
                        after = after[..pos].trim_end();
                    }
                    after = after.strip_suffix(':').unwrap_or(after).trim_end();
                    if !after.is_empty() {
                        return Some(after.to_string());
                    }
                }
                if !is_arrow {
                    depth_angle += 1;
                }
            }
            _ => {}
        }
    }
    None
}

/// Split a top-level conditional type (`C extends E ? T : F`) into its
/// true/false branch texts. `None` when the text carries no depth-0
/// ` extends ` followed by a depth-0 `?` — the marker pair that
/// distinguishes a conditional from an optional member or a ternary-free
/// type. Only the OUTERMOST `? :` pair splits; a conditional nested in a
/// branch stays whole in that branch's text. Depth counts `()`/`<>`/`[]`/
/// `{}`, with an arrow's `=>` exempt from angle depth (no type bracket
/// ends in `=>`), so a function-typed branch does not derail the scan.
pub(crate) fn parse_top_level_conditional(rt: &str) -> Option<(String, String)> {
    let bytes = rt.as_bytes();
    let mut depth: i32 = 0;
    let mut extends_at: Option<usize> = None;
    let mut question_at: Option<usize> = None;
    let mut cond_nesting = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'<' => depth += 1,
            b'>' => {
                let is_arrow = i > 0 && matches!(bytes[i - 1], b'-' | b'=');
                if !is_arrow {
                    depth -= 1;
                }
            }
            b'e' if depth == 0 && extends_at.is_none() => {
                // ` extends ` as a standalone keyword — both neighbours must
                // be non-identifier bytes so `Textends`-like names don't match.
                if rt[i..].starts_with("extends")
                    && i > 0
                    && !bytes[i - 1].is_ascii_alphanumeric()
                    && bytes[i - 1] != b'_'
                    && bytes
                        .get(i + 7)
                        .is_some_and(|c| !c.is_ascii_alphanumeric() && *c != b'_')
                {
                    extends_at = Some(i);
                }
            }
            b'?' if depth == 0 && extends_at.is_some() => {
                if question_at.is_none() {
                    question_at = Some(i);
                } else {
                    cond_nesting += 1;
                }
            }
            b':' if depth == 0 => {
                let Some(q) = question_at else { continue };
                if cond_nesting > 0 {
                    cond_nesting -= 1;
                    continue;
                }
                let true_branch = rt[q + 1..i].trim();
                let false_branch = rt[i + 1..].trim();
                if true_branch.is_empty() || false_branch.is_empty() {
                    return None;
                }
                return Some((true_branch.to_string(), false_branch.to_string()));
            }
            _ => {}
        }
    }
    None
}

/// Parse a Go method's result type from its signature. Go places the result
/// AFTER the param list with no separator, in two shapes the structural
/// member-compare consumes:
///   - interface method_elem `Name(params) result` (no receiver)
///   - struct method `func (recv) Name(params) result` (a leading receiver
///     group the result must skip past)
///
/// The result is the text following the PARAM list's closing paren — the param
/// list is the SECOND top-level paren group for a `func`-prefixed signature
/// (the first is the receiver), the FIRST otherwise. A parenthesized
/// multi-return `(int, error)` is returned verbatim (identical multi-returns on
/// two sides intern to the same TypeId → the structural compare sees `Yes`); a
/// single trailing token is returned with a trailing `{` body dropped and a
/// leading pointer `*` stripped (so `*User` and `User` agree, matching
/// `pointer_type_name`). A void method (nothing after the param list) → `None`.
fn parse_go_result(sig: &str) -> Option<String> {
    let trimmed = sig.trim();
    let skip_receiver = trimmed.starts_with("func");
    // Index of the first param-list group to skip (the receiver) before the
    // real param list. `func`-prefixed struct methods carry one; interface
    // method_elems carry none.
    let groups_to_skip = if skip_receiver { 1 } else { 0 };

    let bytes = trimmed.as_bytes();
    let mut depth: i32 = 0;
    let mut groups_seen = 0usize;
    let mut param_close: Option<usize> = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b']' | b'}' => depth -= 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    if groups_seen == groups_to_skip {
                        param_close = Some(i);
                        break;
                    }
                    groups_seen += 1;
                }
            }
            _ => {}
        }
    }
    let close = param_close?;
    let mut after = trimmed[close + 1..].trim();
    if let Some(pos) = after.find('{') {
        after = after[..pos].trim_end();
    }
    if after.is_empty() {
        return None;
    }
    // A parenthesized multi-return is a single comparable group — keep it whole.
    if after.starts_with('(') {
        return Some(after.to_string());
    }
    Some(strip_go_pointer(after).to_string())
}

/// Drop leading Go pointer markers (`*`, `**`) so a pointer type interns the
/// same as its pointee — mirrors `pointer_type_name`, which the Go extractor
/// already applies to TypeRefs. Keeps the structural compare's SOURCE and
/// TARGET TypeIds aligned regardless of pointer spelling.
fn strip_go_pointer(t: &str) -> &str {
    t.trim_start_matches('*').trim_start()
}

/// Split a type string into (head, args) where args are the top-level generic
/// parameters.  Handles nested generics and multiple args.
///
/// `"List<User>"` → `("List", ["User"])`
/// `"Map<String, User>"` → `("Map", ["String", "User"])`
/// `"List"` → `("List", [])`
/// `"List<Map<String, User>>"` → `("List", ["Map"])` (flat, nested not walked)
///
/// Only the DIRECT type args of the outermost application are returned; the
/// caller is responsible for further descending into nested args if needed.
pub(crate) fn parse_type_head_and_args(type_str: &str) -> (&str, Vec<&str>) {
    split_application(type_str, '<', '>')
}

/// `parse_type_head_and_args` for square-bracket generics (`List[User]`,
/// Go/Scala). Requires a non-empty head before `[`, so a leading-bracket form
/// (a Go slice `[]User`, a tuple `[A, B]`) is NOT read as an application.
///
/// `"List[User]"` → `("List", ["User"])` · `"[]User"` → `("[]User", [])`
pub(crate) fn parse_type_head_and_args_bracket(type_str: &str) -> (&str, Vec<&str>) {
    let open = match type_str.find('[') {
        // No bracket, or a leading bracket (slice/array/tuple) — not an
        // application.
        None | Some(0) => return (type_str.trim(), Vec::new()),
        Some(i) => i,
    };
    let tail = &type_str[open..];
    let Some(close_rel) = find_matching_bracket(tail, '[', ']') else {
        return (type_str.trim(), Vec::new());
    };
    // A clean application ends at the closing `]`. Trailing text means a
    // compound type (`map[K]V`) where the head/args reading would be wrong —
    // leave those to the caller's fallback.
    if !tail[close_rel + 1..].trim().is_empty() {
        return (type_str.trim(), Vec::new());
    }
    (
        type_str[..open].trim(),
        split_top_level_args(&tail[1..close_rel]),
    )
}

/// Split `Head<A, B>` / `Head[A, B]` into the head and its DIRECT top-level
/// args (nested args are not walked — only each arg's own head is returned).
/// Returns `(trimmed input, [])` when there is no `open` bracket.
fn split_application(type_str: &str, open: char, close: char) -> (&str, Vec<&str>) {
    let Some(open_idx) = type_str.find(open) else {
        return (type_str.trim(), Vec::new());
    };
    let head = type_str[..open_idx].trim();
    let tail = &type_str[open_idx..];
    let Some(close_rel) = find_matching_bracket(tail, open, close) else {
        return (head, Vec::new());
    };
    (head, split_top_level_args(&tail[1..close_rel]))
}

/// Split bracket-content on top-level commas (depth-tracked across `<>` and
/// `[]`), returning each arg's bare head (the identifier before any nested
/// bracket).
fn split_top_level_args(args_str: &str) -> Vec<&str> {
    let mut args: Vec<&str> = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0usize;
    for (i, b) in args_str.bytes().enumerate() {
        match b {
            b'<' | b'[' => depth += 1,
            b'>' | b']' => depth = (depth - 1).max(0),
            b',' if depth == 0 => {
                push_arg_head(&mut args, &args_str[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    push_arg_head(&mut args, &args_str[start..]);
    args
}

fn push_arg_head<'a>(args: &mut Vec<&'a str>, seg: &'a str) {
    let arg_head = seg.trim().split(['<', '[']).next().unwrap_or(seg).trim();
    if !arg_head.is_empty() {
        args.push(arg_head);
    }
}

/// True when `t` is a plain (bare or dotted) type name — no generic args,
/// brackets, unions, spaces, pointers, or parameter lists. Used to decide when
/// a signature-derived return type is authoritative for the head over a
/// (possibly parameter) trailing TypeRef.
pub(crate) fn is_plain_type_name(t: &str) -> bool {
    !t.is_empty()
        && t.chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
}

/// Extract a leading-form return type — `RetType name(params)` (Java, C#, and
/// other return-first declaration syntaxes). The return type is the first
/// depth-0 whitespace-delimited token of the signature; the signature builders
/// that produce these strings omit modifiers, so the first token is the return
/// type (`List<User> getItems()` → `List<User>`; a generic method
/// `T <T>get(int i)` → `T`).
///
/// Returns `None` when the first token is empty, opens a parameter list
/// (`getItems()` — the signature carries no return token), or is a declaration
/// keyword / modifier (Go's `func`, or a modifier-prefixed form) — failing safe
/// so the caller keeps its prior head detection. Intended ONLY as a fallback
/// after `parse_return_type_from_signature`, so colon/arrow (params-first)
/// languages never reach it.
pub(crate) fn parse_return_type_positional(sig: &str) -> Option<String> {
    let trimmed = sig.trim_start();
    let head = first_token(trimmed);
    if head.is_empty() || head.contains('(') || head.contains(')') {
        return None;
    }
    // A C elaborated-type-specifier return (`struct Foo name(...)`) leads with an
    // aggregate keyword; the real type is the next depth-0 token. Peel the keyword
    // and read that token. A bare keyword with no following type token (a
    // forward-decl signature with no declarator) yields None.
    if is_c_aggregate_keyword(head) {
        let rest = trimmed[head.len()..].trim_start();
        let next = first_token(rest);
        return (!next.is_empty()
            && !next.contains('(')
            && !next.contains(')')
            && !is_c_aggregate_keyword(next))
        .then(|| next.to_string());
    }
    // `void` is the absence of a return value, not a chainable type — leave the
    // return type unset (a void method's `setName()` carries no return).
    if head == "void" {
        return None;
    }
    if is_decl_keyword_or_modifier(head) {
        return None;
    }
    Some(head.to_string())
}

/// The first depth-0 whitespace-delimited token of `s`, with surrounding
/// whitespace stripped. Brackets keep a generic application
/// (`Map<String, Integer>`) together as one token. `s` must already be
/// `trim_start`-ed for `s[token.len()..]` to address the remainder.
fn first_token(s: &str) -> &str {
    let mut depth: i32 = 0;
    let mut end = s.len();
    for (i, &b) in s.as_bytes().iter().enumerate() {
        match b {
            b'<' | b'[' | b'(' | b'{' => depth += 1,
            b'>' | b']' | b')' | b'}' => depth -= 1,
            b' ' | b'\t' if depth == 0 => {
                end = i;
                break;
            }
            _ => {}
        }
    }
    s[..end].trim()
}

/// A C/C++ aggregate keyword that prefixes an elaborated-type-specifier in a
/// leading-form return (`struct Foo`, `enum Bar`, `union Baz`). The keyword is
/// peeled and the following token is the actual return type.
fn is_c_aggregate_keyword(t: &str) -> bool {
    matches!(t, "struct" | "enum" | "union")
}

/// A first-token value that means the signature is NOT a clean leading-return
/// form: a declaration keyword (Go's `func`, etc.) or an access/storage
/// modifier some builders prefix. Over-rejection is safe — the caller falls
/// back to its prior detection — so this errs toward rejecting.
fn is_decl_keyword_or_modifier(t: &str) -> bool {
    matches!(
        t,
        "func"
            | "fn"
            | "def"
            | "function"
            | "fun"
            | "struct"
            | "enum"
            | "union"
            | "public"
            | "private"
            | "protected"
            | "internal"
            | "static"
            | "final"
            | "abstract"
            | "virtual"
            | "override"
            | "sealed"
            | "async"
            | "extern"
            | "unsafe"
            | "partial"
            | "readonly"
            | "const"
            | "new"
            | "synchronized"
            | "native"
            | "transient"
            | "volatile"
            | "default"
            | "strictfp"
            | "explicit"
            | "implicit"
            | "export"
            | "inline"
            | "signed"
            | "unsigned"
    )
}

/// Extract a trailing-form return type — `func [recv] name(params) RetType` (Go).
/// Returns the text after the last top-level `)` (a trailing `{` body stripped),
/// or `None` when nothing follows (a void func) or it is a multi-return tuple
/// `(A, B)` (not a single chainable type). Depth-tracks all bracket kinds so the
/// receiver/param parens and `[T any]` type params are skipped.
pub(crate) fn parse_return_type_trailing(sig: &str) -> Option<String> {
    let bytes = sig.as_bytes();
    let mut depth: i32 = 0;
    let mut last_close: Option<usize> = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'[' | b'{' => depth += 1,
            b']' | b'}' => depth -= 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    last_close = Some(i);
                }
            }
            _ => {}
        }
    }
    let close = last_close?;
    let mut after = sig[close + 1..].trim();
    if let Some(pos) = after.find('{') {
        after = after[..pos].trim_end();
    }
    if after.is_empty() || after.starts_with('(') {
        return None;
    }
    Some(after.to_string())
}

/// True for languages whose externals carry JVM bytecode descriptors as
/// method/field signatures (Maven sources, `.class` metadata). Gates the
/// JVM-descriptor decoder rung so it never reaches non-JVM signatures.
pub(crate) fn is_jvm_language(lang: &str) -> bool {
    matches!(lang, "java" | "kotlin" | "scala" | "groovy" | "clojure")
}

/// Decode a JVM bytecode descriptor to a chainable element type. JVM externals
/// (Maven sources, `.class` metadata) carry method/field types as raw
/// descriptors — `(Ljava/lang/String;)Lcom/foo/Bar;` for a method, `Lcom/foo/Bar;`
/// for a field — which the colon/arrow/leading/trailing scans cannot read.
///
/// - A method `(params)Ret` decodes its return descriptor (text after the
///   top-level `)`).
/// - An array `[...` decodes to the ELEMENT type (the leading `[`s are peeled),
///   so the chain types through the element like `List<T>` does.
/// - An object `L<slashed/name>;` becomes the dotted `pkg.Cls`.
/// - Primitives (`B I S J F D C Z`) and `V` (void) carry no chainable type →
///   `None`.
pub(crate) fn parse_return_type_from_jvm_descriptor(sig: &str) -> Option<String> {
    let sig = sig.trim();
    // Method descriptor: decode the return after the matching close paren.
    let desc = if let Some(rest) = sig.strip_prefix('(') {
        let close = rest.find(')')?;
        &rest[close + 1..]
    } else {
        sig
    };
    // Peel array dimensions — the element type is what chains.
    let desc = desc.trim_start_matches('[');
    let object = desc.strip_prefix('L')?;
    let slashed = object.strip_suffix(';').unwrap_or(object);
    if slashed.is_empty() {
        return None;
    }
    Some(slashed.replace('/', "."))
}

#[cfg(test)]
#[path = "chain_walker_tests.rs"]
mod tests;
