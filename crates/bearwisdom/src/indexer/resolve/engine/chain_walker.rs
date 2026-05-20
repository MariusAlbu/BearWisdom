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
// Plus the small string utilities the chain walker needs: strip_generic_args,
// find_matching_bracket, tuple_element, parse_return_type_from_signature.
// strip_generic_args is also used by SymbolLookup::record_chain_miss so it
// lives here (the chain miss it strips is a chain-walker emission).
// =============================================================================

use std::collections::BTreeMap;

use rustc_hash::FxHashMap;

use crate::type_checker::type_env::TypeEnvironment;

use super::{SymbolInfo, SymbolLookup, TypeInfo};

pub(crate) fn strip_generic_args(s: &str) -> String {
    let base = match s.find('<') {
        Some(i) => &s[..i],
        None => s,
    };
    base.trim_end_matches('.').to_string()
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
    by_qname: &BTreeMap<String, SymbolInfo>,
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
    // Locate the outermost parenthesized group: forward-scan for first `(`
    // at depth 0 across `<` and `[` so generic / index args don't fool us.
    let mut depth_angle: i32 = 0;
    let mut depth_square: i32 = 0;
    let mut open_idx: Option<usize> = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'<' => depth_angle += 1,
            b'>' => depth_angle -= 1,
            b'[' => depth_square += 1,
            b']' => depth_square -= 1,
            b'(' if depth_angle == 0 && depth_square == 0 => {
                open_idx = Some(i);
                break;
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
/// token is the type, possibly with generic brackets.
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
    match last_ws {
        Some(ws) => trimmed[ws + 1..].trim().to_string(),
        None => trimmed.to_string(),
    }
}

/// C / C++ / Java / C# style: arg looks like `Type name` (prefix type).
/// Take everything up to the last whitespace at top-level — that's the
/// type expression, possibly with pointer/reference markers.
fn extract_param_type_prefix(part: &str) -> String {
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

pub(crate) fn parse_return_type_from_signature(sig: &str) -> Option<String> {
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
    None
}


pub(crate) fn infer_type_from_chain(
    chain: &crate::types::MemberChain,
    scope_path: &Option<String>,
    type_info: &FxHashMap<String, TypeInfo>,
    by_name: &FxHashMap<String, Vec<SymbolInfo>>,
    by_qname: &BTreeMap<String, SymbolInfo>,
) -> Option<String> {
    use crate::types::SegmentKind;

    let segments = &chain.segments;
    if segments.is_empty() {
        return None;
    }

    // Build a minimal scope chain from scope_path.
    let scopes: Vec<String> = if let Some(sp) = scope_path {
        let mut scope_chain = Vec::new();
        let mut current = sp.clone();
        scope_chain.push(current.clone());
        while let Some(dot) = current.rfind('.') {
            current.truncate(dot);
            scope_chain.push(current.clone());
        }
        scope_chain
    } else {
        Vec::new()
    };

    // Phase 1: Root type.
    let root_type = match segments[0].kind {
        SegmentKind::SelfRef => {
            // Find enclosing class from scope.
            scopes
                .iter()
                .find_map(|s| {
                    by_qname.get(s).and_then(|sym| {
                        if matches!(sym.kind.as_str(), "class" | "struct" | "interface") {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                })
                .or_else(|| scopes.last().cloned())
        }
        SegmentKind::Identifier => {
            let name = &segments[0].name;
            scopes
                .iter()
                .find_map(|scope| {
                    let qname = format!("{scope}.{name}");
                    type_info.get(&qname).and_then(|ti| ti.field_type.clone())
                })
                .or_else(|| segments[0].declared_type.clone())
        }
        _ => None,
    }?;

    let mut current_type = root_type;

    // Build a TypeEnvironment for generic substitution.
    let mut env = TypeEnvironment::new();

    // Look up initial generic args for the root field.
    for scope in &scopes {
        let key = format!("{scope}.{}", segments[0].name);
        if let Some(ti) = type_info.get(&key) {
            if !ti.type_args.is_empty() {
                // Enter the generic context for the root type.
                let args = ti.type_args.clone();
                env.enter_generic_context(&current_type, &args, |name| {
                    // Look up by qualified name first, then by simple name.
                    type_info
                        .get(name)
                        .map(|ti| ti.generic_params.clone())
                        .filter(|p| !p.is_empty())
                        .or_else(|| {
                            let simple = name.rsplit('.').next().unwrap_or(name);
                            type_info
                                .get(simple)
                                .map(|ti| ti.generic_params.clone())
                                .filter(|p| !p.is_empty())
                        })
                });
                break;
            }
        }
    }

    // Phase 2: Walk remaining segments.
    for seg in &segments[1..] {
        // Tuple-index selection: produced by array-pattern destructuring.
        // `const [isOpen, setIsOpen] = useState<boolean>(false)` emits a
        // ComputedAccess segment with integer name "0" / "1" so this pass
        // can slice the tuple return type of the preceding call.
        //
        // The incoming `current_type` at this point is the return type of
        // the call (already generic-resolved by a prior iteration), e.g.
        // `[boolean, Dispatch<SetStateAction<boolean>>]`. Splitting by
        // top-level commas and picking the Nth element yields the type
        // bound to the destructured name.
        if seg.kind == SegmentKind::ComputedAccess {
            if let Ok(idx) = seg.name.parse::<usize>() {
                if let Some(element) = tuple_element(&current_type, idx) {
                    current_type = env.resolve(&element);
                    continue;
                }
            }
        }

        // Strip the structural generic args off the current type before
        // building the member qname — type_info is keyed by the BASE
        // class qname (e.g. "Repository.findOne"), not the applied form
        // ("Repository<User>.findOne"). The args are already in `env`
        // from the previous iteration's enter_generic_context.
        let base_type = strip_generic_args(&current_type);
        let member_qname = format!("{base_type}.{}", seg.name);

        if let Some(ti) = type_info.get(&member_qname) {
            if let Some(ft) = &ti.field_type {
                let new_args = ti.type_args.clone();
                let resolved_type = env.resolve(ft);
                env.push_scope();
                if !new_args.is_empty() {
                    env.enter_generic_context(&resolved_type, &new_args, |name| {
                        type_info
                            .get(name)
                            .map(|ti| ti.generic_params.clone())
                            .filter(|p| !p.is_empty())
                            .or_else(|| {
                                let simple = name.rsplit('.').next().unwrap_or(name);
                                type_info
                                    .get(simple)
                                    .map(|ti| ti.generic_params.clone())
                                    .filter(|p| !p.is_empty())
                            })
                    });
                }
                current_type = resolved_type;
                continue;
            }
            if let Some(raw_return) = &ti.return_type {
                // Use TypeEnvironment for generic substitution (T → User, E → Error, etc).
                let resolved = env.resolve(raw_return);
                env.push_scope();
                current_type = resolved;
                continue;
            }
        }

        // Fallback: if the segment corresponds to a call with no matching
        // TypeInfo entry, treat the current_type as the "call return" so
        // chains like `useState(...)[0]` still work when useState is
        // external and its signature is in the index keyed by simple name
        // rather than qualified name. Look up by simple name.
        if let Some(sym_list) = by_name.get(&seg.name) {
            for sym in sym_list {
                if let Some(ti) = type_info.get(&sym.qualified_name) {
                    if let Some(raw_return) = &ti.return_type {
                        let resolved = env.resolve(raw_return);
                        env.push_scope();
                        current_type = resolved;
                        break;
                    }
                }
            }
            // If the loop above didn't `continue`, fall through to the
            // "can't follow" return below only when nothing matched.
            if current_type.contains('[') || current_type.contains('<') {
                // current_type was updated; move to the next segment.
                continue;
            }
        }

        // Can't follow further.
        let _ = by_qname; // retained for future use
        return None;
    }

    // The final current_type is the inferred type of the chain.
    Some(current_type)
}

// ---------------------------------------------------------------------------
// Chain-aware external inference (shared by all resolvers)
// ---------------------------------------------------------------------------

/// If a ref has a MemberChain, walk it to see if we can determine a type
/// that isn't in our index — meaning the chain leads to an external type.
///
/// For `this.repo.findMany()` where repo has type `PrismaClient`:
/// 1. `this` → `UserService` (from scope chain)
/// 2. `repo` → field_type = `PrismaClient`
/// 3. `PrismaClient` not in index → return Some("PrismaClient")
///
/// This classifies the entire chain call as external with the unresolved
/// type name as the namespace.
pub fn infer_external_from_chain(
    chain: &crate::types::MemberChain,
    scope_chain: &[String],
    lookup: &dyn SymbolLookup,
) -> Option<String> {
    use crate::types::SegmentKind;

    let segments = &chain.segments;
    if segments.len() < 2 {
        return None;
    }

    // Fast path: if the root segment is a known external name (primitive, test
    // framework global), classify the whole chain as external immediately.
    // We use an empty string as the language hint here since `infer_external_from_chain`
    // is language-agnostic; the language-specific primitives are checked inside
    // `is_external_name` only when the language is known.  The test-global check
    // is language-independent, so it still fires correctly.
    if segments[0].kind == SegmentKind::Identifier {
        let root = &segments[0].name;
        // Use an empty language string — this still catches test globals.
        // Primitive checks per language happen in the language-specific resolvers.
        if lookup.is_external_name(root, "") {
            return Some(format!("{}.*", root));
        }
    }

    // Phase 1: Determine root type.
    let root_type = match segments[0].kind {
        SegmentKind::SelfRef => {
            // Find enclosing class.
            let mut found = None;
            for scope in scope_chain {
                if let Some(sym) = lookup.by_qualified_name(scope) {
                    if matches!(sym.kind.as_str(), "class" | "struct" | "interface") {
                        found = Some(scope.clone());
                        break;
                    }
                }
            }
            found.or_else(|| scope_chain.last().cloned())
        }
        SegmentKind::Identifier => {
            let name = &segments[0].name;
            // Field on enclosing class? Consult the TypeId surface first
            // and format back to the legacy string view — the walker still
            // operates on `current_type: String`, but the data comes from
            // the canonical TypeArena.
            let mut found = None;
            for scope in scope_chain {
                let field_qname = format!("{scope}.{name}");
                if let Some(id) = lookup.field_type_id(&field_qname) {
                    if let Some(arena) = lookup.type_arena() {
                        found = Some(arena.format_type(id));
                        break;
                    }
                }
                if let Some(type_name) = lookup.field_type_name(&field_qname) {
                    found = Some(type_name.to_string());
                    break;
                }
            }
            found.or_else(|| segments[0].declared_type.clone())
        }
        _ => None,
    };

    // Phase 2: Walk the chain checking if the current type is external.
    // If no root type was determined, check if the root identifier itself
    // is external (not in the index, or a variable with no resolvable type).
    //
    // IMPORTANT: all `by_name` lookups in this function must filter out
    // external-origin symbols. Two signals combine: the `ext:` path-prefix
    // convention AND the explicit external-paths set (DB origin='external'
    // files that kept a project-relative path — script-tag-discovered
    // vendor JS like `wwwroot/lib/jquery.min.js`). Otherwise a Python
    // external type like `sqlalchemy.Table` or a vendored `$` declaration
    // would match a user-code root and convince the walker the chain is
    // "internal", suppressing the external classification for code that
    // genuinely calls into third-party library surface.
    let is_internal = |s: &SymbolInfo| -> bool { !lookup.is_external_file(&s.file_path) };

    let mut current_type = match root_type {
        Some(t) => t,
        None => {
            if segments[0].kind == SegmentKind::Identifier {
                let name = &segments[0].name;
                // Type-like kinds (class/struct/interface/enum/type_alias) are
                // pre-indexed in types_by_name — avoids scanning the full
                // by_name candidate pool (externals can collide by the tens of
                // thousands on common names like "Context"/"Error"/"Request").
                let has_type = lookup.types_by_name(name).iter().filter(|s| is_internal(s)).any(|s| {
                    matches!(
                        s.kind.as_str(),
                        "class" | "struct" | "interface" | "enum" | "type_alias"
                    )
                });
                if has_type {
                    return None;
                }
                // Function/method presence still needs the full by_name pool
                // since those aren't type-kinds — but here we only care whether
                // any exists, and `.any()` short-circuits. `has_in_namespace`
                // would be even cheaper but a bare-name function doesn't live
                // under a namespace prefix. Fall through to by_name but gate
                // behind the cheaper type check above so the fast path wins
                // on hot type names.
                let has_func = lookup.by_name(name).iter().filter(|s| is_internal(s)).any(|s| {
                    matches!(s.kind.as_str(), "function" | "method")
                });
                if has_func {
                    return None;
                }
                return Some(name.clone());
            }
            return None;
        }
    };

    for seg in &segments[1..] {
        // Strip the structural generic args from current_type for the
        // is-it-in-the-index probe AND the member-qname lookup. The
        // index keys store the base class name; the args are tracked
        // implicitly via TypeArena Apply decomposition for callers that
        // need them.
        let base_type = strip_generic_args(&current_type);

        // If the current type isn't in the index → it's external.
        // Use types_by_name (pre-filtered type-kind pool) so common external
        // type names like "Context"/"Request"/"Error" don't drag in tens of
        // thousands of non-type candidates on every ref.
        let type_in_index = lookup
            .by_qualified_name(&base_type)
            .filter(|s| is_internal(s))
            .is_some()
            || lookup.types_by_name(&base_type).iter().filter(|s| is_internal(s)).any(|s| {
                matches!(
                    s.kind.as_str(),
                    "class" | "struct" | "interface" | "enum" | "type_alias"
                        | "trait" | "module" | "namespace"
                )
            });

        if !type_in_index {
            return Some(current_type);
        }

        // Try to follow to the next type. Prefer the canonical TypeId
        // path so any structural information (Apply decomposition) is
        // surfaced through arena.format_type; fall back to the legacy
        // string accessor when no arena is bound.
        let member_qname = format!("{base_type}.{}", seg.name);
        let arena = lookup.type_arena();
        if let Some(id) = lookup.field_type_id(&member_qname) {
            if let Some(a) = arena {
                current_type = a.format_type(id);
                continue;
            }
        }
        if let Some(id) = lookup.return_type_id(&member_qname) {
            if let Some(a) = arena {
                current_type = a.format_type(id);
                continue;
            }
        }
        if let Some(next) = lookup.field_type_name(&member_qname) {
            current_type = next.to_string();
            continue;
        }
        if let Some(next) = lookup.return_type_name(&member_qname) {
            current_type = next.to_string();
            continue;
        }

        // Can't follow further — the member exists on a known type but
        // we don't know its return type. Not enough info to classify.
        break;
    }

    None
}
