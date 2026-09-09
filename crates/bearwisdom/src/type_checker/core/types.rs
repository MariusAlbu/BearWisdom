// =============================================================================
// type_checker/core/types.rs — Type, TypeId, TypeArena, PrimKind, LitValue
//
// The arena owns every Type instance in the workspace. Consumers hold TypeIds
// (NonZeroU32) and dereference through TypeArena::get. Identity equality on
// types is integer equality on TypeIds.
//
// Phase 5 makes the arena interior-mutable: `intern` / `class` / `primitive`
// take `&self` so a single shared `Engine` can drive resolution across rayon
// workers. `get` returns owned `Type` (cheap clone — every prior call site
// already cloned the borrowed result).
//
// Spec: research/architecture/01-canonical-symbol-ref-contract.html
//       research/architecture/02-engine-internal-architecture.html § Layer 1
// =============================================================================

use rustc_hash::FxHashMap;
use std::num::NonZeroU32;
use std::sync::RwLock;

/// Interned-type identifier. Nonzero so `Option<TypeId>` is one word.
/// Stable within a workspace build; not durable across indexing runs.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct TypeId(pub NonZeroU32);

impl TypeId {
    pub fn index(self) -> usize {
        (self.0.get() as usize) - 1
    }
}

/// Generic-parameter identifier. Bound names live in TypeArena::generic_params.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct GenericParamId(pub NonZeroU32);

impl GenericParamId {
    pub fn index(self) -> usize {
        (self.0.get() as usize) - 1
    }
}

#[path = "primitives.rs"]
mod primitives;
pub use primitives::PrimKind;
#[path = "intrinsics.rs"]
mod intrinsics;
pub use intrinsics::Intrinsic;
#[path = "unique_symbols.rs"]
mod unique_symbols;
pub use unique_symbols::UniqueSymbol;
#[path = "type_operators.rs"]
mod type_operators;
pub use type_operators::{MappedModifier, TypeOperator, TypeProperty};
#[path = "indirection.rs"]
mod indirection;
pub use indirection::{GenericParamKind, Indirection, Lifetime, Mutability};
#[path = "nominal_types.rs"]
mod nominal_types;
#[path = "type_rebind.rs"]
mod type_rebind;
pub use nominal_types::NominalContextId;
#[path = "callable_types.rs"]
mod callable_types;
pub use callable_types::{
    Callable, CallableGeneric, CallableOrigin, CallableParameter, CallablePredicate,
};

/// Singleton-type value carrier. `Type::Literal(LitValue::Str("foo"))` is the
/// type whose only inhabitant is the string `"foo"`.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum LitValue {
    Str(String),
    Int(i64),
    Bool(bool),
    /// IEEE-754 bits, canonicalized at ingestion (not a source spelling).
    Number(u64),
    /// Little-endian base-2^32 words; zero has no words and no negative sign.
    BigInt {
        negative: bool,
        words: Vec<u32>,
    },
    /// Used only when the string contains unpaired UTF-16 surrogates.
    Utf16(Vec<u16>),
}

/// Bound info for a generic parameter — captured at extraction time so the
/// engine can resolve `T` back to the declaration that introduced it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct GenericParamData {
    #[serde(default)]
    pub kind: GenericParamKind,
    /// Source name of the parameter (e.g. `T`, `K`, `V`).
    pub name: String,
    /// Index of the symbol that introduces this parameter (function, type,
    /// method). Used to delimit the parameter's scope.
    pub owner_symbol_index: usize,
    /// Optional upper-bound type (e.g. `T extends Animal` in TS).
    pub bound: Option<TypeId>,
}

#[path = "type_forms.rs"]
mod type_forms;
pub use type_forms::Type;
#[path = "object_types.rs"]
mod object_types;
pub use object_types::{ObjectOrigin, SourceObject};
#[derive(Default)]
struct TypeArenaInner {
    types: Vec<Type>,
    intern: FxHashMap<Type, TypeId>,
    qname_to_class: FxHashMap<String, TypeId>,
    decl_by_symbol: FxHashMap<(Option<NominalContextId>, i64), TypeId>,
    nominal_scopes: Vec<nominal_types::Scope>,
    generic_params: Vec<GenericParamData>,
}

/// Per-workspace interning storage for `Type` values. Interior-mutable so a
/// single `&TypeArena` can be shared across rayon workers — every mutating
/// method (`intern`, `class`, `primitive`, `intern_generic`) takes `&self`
/// and acquires a write lock briefly; reads acquire a read lock and clone.
///
/// Cloning the result of `get` is cheap for the dominant case (Class /
/// Primitive / Literal / Unknown are single-word) and matches what every
/// pre-Phase-5 caller did already (`arena.get(ty).clone()`).
pub struct TypeArena {
    inner: RwLock<TypeArenaInner>,
}

impl Default for TypeArena {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for TypeArena {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.read().unwrap();
        f.debug_struct("TypeArena")
            .field("types_len", &inner.types.len())
            .field("generic_params_len", &inner.generic_params.len())
            .finish()
    }
}

impl TypeArena {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(TypeArenaInner::default()),
        }
    }

    /// Intern `ty`, returning an existing TypeId on duplicate insert.
    /// Reads first for the common dedup case; upgrades to write only when
    /// a new entry is needed. Safe to call concurrently.
    pub fn intern(&self, ty: Type) -> TypeId {
        if let Type::Decl {
            symbol_id,
            qname,
            context,
        } = ty
        {
            return self.intern_decl(&qname, symbol_id, context);
        }
        if let Some(&id) = self.inner.read().unwrap().intern.get(&ty) {
            return id;
        }
        let mut inner = self.inner.write().unwrap();
        // Re-check after acquiring write lock — another thread may have
        // interned the same Type while we were upgrading.
        if let Some(&id) = inner.intern.get(&ty) {
            return id;
        }
        let idx = inner.types.len();
        let id = TypeId(NonZeroU32::new((idx + 1) as u32).expect("arena index overflow"));
        let scope = nominal_types::Scope::capture(&ty, &inner.nominal_scopes);
        inner.nominal_scopes.push(scope);
        inner.types.push(ty.clone());
        inner.intern.insert(ty, id);
        id
    }

    /// Look up an existing TypeId for a `Type` without inserting.
    pub fn lookup(&self, ty: &Type) -> Option<TypeId> {
        if let Type::Decl {
            symbol_id, context, ..
        } = ty
        {
            return self
                .inner
                .read()
                .unwrap()
                .decl_by_symbol
                .get(&(*context, *symbol_id))
                .copied();
        }
        self.inner.read().unwrap().intern.get(ty).copied()
    }

    /// Intern a Class type by qualified name. Subsequent calls return the
    /// same TypeId.
    pub fn class(&self, qname: &str) -> TypeId {
        if let Some(&id) = self.inner.read().unwrap().qname_to_class.get(qname) {
            return id;
        }
        let id = self.intern(Type::Class(qname.to_string()));
        // Reacquire write to track in qname_to_class. Idempotent insert is
        // safe under concurrent callers.
        self.inner
            .write()
            .unwrap()
            .qname_to_class
            .insert(qname.to_string(), id);
        id
    }

    /// Look up the Class TypeId for `qname` without interning.
    pub fn class_lookup(&self, qname: &str) -> Option<TypeId> {
        self.inner
            .read()
            .unwrap()
            .qname_to_class
            .get(qname)
            .copied()
    }

    /// Intern a primitive type.
    pub fn primitive(&self, p: PrimKind) -> TypeId {
        self.intern(Type::Primitive(p))
    }

    /// Resolve a TypeId to the underlying `Type`. Returns an owned clone so
    /// callers don't hold the read lock across operations. Cloning is
    /// cheap for the common variants and matches what every prior call
    /// site already did (`arena.get(ty).clone()`).
    ///
    /// Panics on out-of-range ids — TypeIds are only constructed through
    /// `intern`, so this should never fire in well-formed code.
    pub fn get(&self, id: TypeId) -> Type {
        self.inner.read().unwrap().types[id.index()].clone()
    }

    /// Inverse of `intern_type_str`: render a TypeId as its source-form
    /// string. `Class("Foo")` → `"Foo"`; `Apply(Class("Repository"),
    /// [Class("User")])` → `"Repository<User>"`. Used by consumers that
    /// still operate on string-shaped type info while the storage layer
    /// has migrated to TypeIds.
    pub fn format_type(&self, id: TypeId) -> String {
        let mut out = String::new();
        self.format_type_into(id, &mut out);
        out
    }

    fn format_type_into(&self, id: TypeId, out: &mut String) {
        use std::fmt::Write as _;
        let ty = self.get(id);
        match ty {
            Type::Class(q) => out.push_str(&q),
            Type::Decl { qname, .. } => out.push_str(&qname),
            Type::Primitive(p) => {
                let _ = write!(out, "{p:?}");
            }
            Type::Intrinsic(kind) => out.push_str(kind.display()),
            Type::UniqueSymbol(_) => out.push_str("unique symbol"),
            Type::Operator(op) => op.format(self, out),
            Type::Apply { base, args } => {
                self.format_type_into(base, out);
                if !args.is_empty() {
                    out.push('<');
                    for (i, arg) in args.iter().enumerate() {
                        if i > 0 {
                            out.push_str(", ");
                        }
                        self.format_type_into(*arg, out);
                    }
                    out.push('>');
                }
            }
            Type::Tuple(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    self.format_type_into(*item, out);
                }
                out.push(']');
            }
            Type::Union(branches) => {
                for (i, b) in branches.iter().enumerate() {
                    if i > 0 {
                        out.push_str(" | ");
                    }
                    self.format_type_into(*b, out);
                }
            }
            Type::Intersection(branches) => {
                for (i, b) in branches.iter().enumerate() {
                    if i > 0 {
                        out.push_str(" & ");
                    }
                    self.format_type_into(*b, out);
                }
            }
            Type::Callable(callable) => callable.format(self, out),
            Type::Object(object) => object.format(self, out),
            Type::Function { params, return_ } => {
                out.push('(');
                for (i, p) in params.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    self.format_type_into(*p, out);
                }
                out.push_str(") => ");
                self.format_type_into(return_, out);
            }
            Type::Generic { param } => {
                let data = self.generic_param(param);
                out.push_str(&data.name);
            }
            Type::Region(region) => out.push_str(&self.format_region(region)),
            Type::Indirect {
                kind,
                mutability,
                inner,
            } => {
                match kind {
                    Indirection::Reference(region) => {
                        out.push('&');
                        out.push_str(&self.format_region(region));
                        out.push(' ');
                        if mutability == Mutability::Mutable {
                            out.push_str("mut ");
                        }
                    }
                    Indirection::Pointer => out.push_str(if mutability == Mutability::Mutable {
                        "*mut "
                    } else {
                        "*const "
                    }),
                }
                self.format_type_into(inner, out);
            }
            Type::Optional(inner) => {
                self.format_type_into(inner, out);
                out.push('?');
            }
            Type::AsyncWrapper(inner) => {
                out.push_str("Promise<");
                self.format_type_into(inner, out);
                out.push('>');
            }
            Type::Iterator(inner) => {
                out.push_str("Iterator<");
                self.format_type_into(inner, out);
                out.push('>');
            }
            Type::Constructor(inner) => {
                out.push_str("typeof ");
                self.format_type_into(inner, out);
            }
            Type::Literal(v) => {
                let _ = write!(out, "{v:?}");
            }
            Type::Unknown => out.push_str("unknown"),
        }
    }

    /// Number of distinct types currently interned.
    pub fn len(&self) -> usize {
        self.inner.read().unwrap().types.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.read().unwrap().types.is_empty()
    }

    /// Allocate a generic parameter slot.
    pub fn intern_generic(&self, data: GenericParamData) -> GenericParamId {
        let mut inner = self.inner.write().unwrap();
        let idx = inner.generic_params.len();
        let id = GenericParamId(NonZeroU32::new((idx + 1) as u32).expect("arena index overflow"));
        inner.generic_params.push(data);
        id
    }

    pub fn generic_param(&self, id: GenericParamId) -> GenericParamData {
        self.inner.read().unwrap().generic_params[id.index()].clone()
    }

    pub fn generic_param_count(&self) -> usize {
        self.inner.read().unwrap().generic_params.len()
    }

    /// Faithful, order-preserving snapshot of the whole arena (every `Type` plus
    /// every `GenericParamData`) as one JSON blob. Unlike `format_type` this is
    /// lossless — every variant and child `TypeId` is encoded — so a persisted
    /// raw `TypeId` index stays valid after `restore_snapshot` rebuilds an
    /// identical arena. One blob per index; the durable form that lets
    /// `symbol_type_info` carry `TypeId`s instead of re-parsed type strings.
    pub fn serialize_snapshot(&self) -> String {
        let inner = self.inner.read().unwrap();
        serde_json::to_string(&(&inner.types, &inner.generic_params))
            .unwrap_or_else(|_| "[[],[]]".to_string())
    }

    /// Rebuild the arena from a `serialize_snapshot` blob, preserving every
    /// `TypeId` (types are restored in id order, so a child's id is always
    /// smaller than its parent's). Rebuilds the intern + qname indices so later
    /// `intern`/`class` calls dedup against the restored set and appended types
    /// get fresh ids after the restored range. Returns the count restored.
    /// Intended for a fresh arena at the start of an incremental load.
    /// Configured nominal contexts are reminted consistently within this load;
    /// program views must rebind them before interpreting restored type slots.
    pub fn restore_snapshot(&self, blob: &str) -> usize {
        let Ok((mut types, generic_params)) =
            serde_json::from_str::<(Vec<Type>, Vec<GenericParamData>)>(blob)
        else {
            return 0;
        };
        let mut inner = self.inner.write().unwrap();
        inner.intern.clear();
        inner.qname_to_class.clear();
        inner.decl_by_symbol.clear();
        inner.nominal_scopes.clear();
        nominal_types::refresh_contexts(&mut types);
        for (i, ty) in types.iter().enumerate() {
            let id = TypeId(NonZeroU32::new((i + 1) as u32).expect("arena index overflow"));
            if let Type::Decl {
                context, symbol_id, ..
            } = ty
            {
                inner
                    .decl_by_symbol
                    .entry((*context, *symbol_id))
                    .or_insert(id);
            } else {
                inner.intern.insert(ty.clone(), id);
            }
            let scope = nominal_types::Scope::capture(ty, &inner.nominal_scopes);
            inner.nominal_scopes.push(scope);
            if let Type::Class(name) = ty {
                inner.qname_to_class.insert(name.clone(), id);
            }
        }
        let n = types.len();
        inner.types = types;
        inner.generic_params = generic_params;
        n
    }
}

/// Strip a leading reference sigil from a type string, returning the trimmed
/// referent when `s` begins with `&`. Consumes the `&`, then an optional
/// lifetime (`'a`), then an optional `mut`, returning whatever type string
/// follows. Returns `None` when `s` does not start with `&` so the caller
/// leaves non-reference strings (including a TS `A & B` intersection, whose
/// `&` is interior) untouched.
fn strip_reference_sigil(s: &str) -> Option<&str> {
    let rest = s.strip_prefix('&')?.trim_start();
    // Optional lifetime: `'a`, `'static`, etc. Consumed up to the next
    // whitespace so the type that follows is what binds.
    let rest = if let Some(after_tick) = rest.strip_prefix('\'') {
        match after_tick.find(char::is_whitespace) {
            Some(ws) => after_tick[ws..].trim_start(),
            // A bare `&'a` with no following type — nothing to intern.
            None => "",
        }
    } else {
        rest
    };
    // Optional `mut`, requiring a word boundary so a type named `mutate`
    // isn't truncated.
    let rest = match rest.strip_prefix("mut") {
        Some(after) if after.starts_with(char::is_whitespace) => after.trim_start(),
        _ => rest,
    };
    Some(rest)
}

/// Strip a leading pointer sigil from a type string, returning the trimmed
/// pointee when `s` begins with `*`. Returns `None` when `s` does not start
/// with `*` so the caller leaves non-pointer strings untouched.
fn strip_pointer_sigil(s: &str) -> Option<&str> {
    Some(s.strip_prefix('*')?.trim_start())
}

/// Strip a leading opaque/existential keyword (`some`/`any`) from a type
/// string, returning the trimmed inner type when `s` begins with the keyword
/// followed by whitespace and a non-empty type. A trailing whitespace boundary
/// is required so a class name that merely starts with those letters
/// (`Something`, `anyOf`) is not truncated. Returns `None` otherwise so the
/// caller leaves the string untouched.
fn strip_opaque_existential_prefix(s: &str) -> Option<&str> {
    for kw in ["some", "any"] {
        if let Some(after) = s.strip_prefix(kw) {
            if after.starts_with(char::is_whitespace) {
                let inner = after.trim_start();
                if !inner.is_empty() {
                    return Some(inner);
                }
            }
        }
    }
    None
}

/// Byte index of a top-level `=>` (function-type arrow) in `s`, or `None`.
/// Depth-tracks `()`/`[]`/`<>`/`{}` so an arrow nested inside a generic
/// argument (`Foo<() => T>`) or a parameter list isn't mistaken for the
/// type's own arrow. Returns the index of the `=`.
fn find_top_level_arrow(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth: i32 = 0;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'(' | b'[' | b'<' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'>' => depth -= 1,
            b'=' if depth == 0 && i + 1 < b.len() && b[i + 1] == b'>' => return Some(i),
            // `->` return arrow: Rust `Fn() -> T` / `impl Fn() -> T`, Kotlin and
            // Swift `(T) -> R`. Matched before the `>` decrement below so the
            // arrow's own `>` isn't mistaken for a generic close.
            b'-' if depth == 0 && i + 1 < b.len() && b[i + 1] == b'>' => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

/// Locate the position of the bracket that closes `open` in `s`. `s` must
/// start with `open`. Returns the byte index of the matching `close` (still
/// relative to `s`) or `None` if the brackets don't balance.
fn find_matching_close(s: &str, open: char, close: char) -> Option<usize> {
    let mut depth: i32 = 0;
    for (i, ch) in s.char_indices() {
        if ch == open {
            depth += 1;
        } else if ch == close {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// Peel a `name:` annotation off one function-type parameter piece, returning
/// the bare type. Splits on the LAST `:` at bracket depth 0 so TS `value: T`
/// yields `T` while a type argument that itself contains `:` stays intact. A
/// piece with no top-level `:` (Rust `Fn(T)`, where the param is bare) is
/// returned whole.
fn strip_param_name(piece: &str) -> &str {
    let mut depth: i32 = 0;
    let mut last_colon: Option<usize> = None;
    for (i, ch) in piece.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' | ']' | ')' | '}' => depth -= 1,
            ':' if depth == 0 => last_colon = Some(i),
            _ => {}
        }
    }
    match last_colon {
        Some(i) => piece[i + 1..].trim(),
        None => piece.trim(),
    }
}

/// Split `s` on commas that sit at bracket depth 0, returning the trimmed
/// pieces with empty entries dropped. Recognizes `<>`, `[]`, `()`, and `{}`
/// for nesting so generic type arguments split cleanly.
fn split_depth_zero_commas(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth: i32 = 0;
    let mut start: usize = 0;
    for (i, ch) in s.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' | ']' | ')' | '}' => depth -= 1,
            ',' if depth == 0 => {
                let piece = s[start..i].trim();
                if !piece.is_empty() {
                    out.push(piece.to_string());
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    if start < s.len() {
        let piece = s[start..].trim();
        if !piece.is_empty() {
            out.push(piece.to_string());
        }
    }
    out
}

/// Byte index of a `;` at bracket depth 0 in `s`, or `None`. Splits a Rust
/// fixed-size array's element type from its length: `[T; N]` → element ends
/// where this returns.
fn find_depth_zero_semicolon(s: &str) -> Option<usize> {
    let mut depth: i32 = 0;
    for (i, ch) in s.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' | ']' | ')' | '}' => depth -= 1,
            ';' if depth == 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// Strip a `label:` / `label?:` prefix from one tuple element (`get: Accessor<T>`
/// → `Accessor<T>`). The label must be a plain identifier; an element with no
/// such prefix — or a type that merely contains `:` (an object literal, a mapped
/// type) — is returned unchanged.
fn strip_tuple_label(elem: &str) -> &str {
    let e = elem.trim();
    if let Some((label, rest)) = e.split_once(':') {
        let label = label.trim_end_matches('?').trim();
        if !label.is_empty()
            && label.chars().all(|c| c.is_alphanumeric() || c == '_')
            && !rest.trim_start().starts_with(':')
        {
            return rest.trim();
        }
    }
    e
}

/// Split `s` on `delim` (`|` or `&`) at bracket depth 0, returning the trimmed
/// non-empty pieces — but ONLY when the split yields two or more arms, marking a
/// genuine composite type. A string with no top-level `delim`, or only a leading
/// one (`| A`), collapses to a single arm and returns `None`, so the caller falls
/// through to the nominal / generic parse. Recognizes `<>[]{}()` nesting so an
/// operator inside generic arguments (`Record<string, A | B>`) is not split.
fn split_top_level(s: &str, delim: char) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut depth: i32 = 0;
    let mut start: usize = 0;
    for (i, ch) in s.char_indices() {
        match ch {
            '<' | '[' | '(' | '{' => depth += 1,
            '>' | ']' | ')' | '}' => depth -= 1,
            c if c == delim && depth == 0 => {
                let piece = s[start..i].trim();
                if !piece.is_empty() {
                    out.push(piece.to_string());
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    let piece = s[start..].trim();
    if !piece.is_empty() {
        out.push(piece.to_string());
    }
    if out.len() >= 2 {
        Some(out)
    } else {
        None
    }
}

mod type_str;

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
