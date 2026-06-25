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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TypeId(pub NonZeroU32);

impl TypeId {
    pub fn index(self) -> usize {
        (self.0.get() as usize) - 1
    }
}

/// Generic-parameter identifier. Bound names live in TypeArena::generic_params.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GenericParamId(pub NonZeroU32);

impl GenericParamId {
    pub fn index(self) -> usize {
        (self.0.get() as usize) - 1
    }
}

/// Canonical primitive categories. Width-specific integer / float variants
/// collapse to Int / Float; the engine does not type-check numeric precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PrimKind {
    Int,
    Float,
    Str,
    Char,
    Bytes,
    Bool,
    Unit,
    Never,
    Symbol,
    Unknown,
}

/// Singleton-type value carrier. `Type::Literal(LitValue::Str("foo"))` is the
/// type whose only inhabitant is the string `"foo"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LitValue {
    Str(String),
    Int(i64),
    Bool(bool),
}

/// Bound info for a generic parameter — captured at extraction time so the
/// engine can resolve `T` back to the declaration that introduced it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GenericParamData {
    /// Source name of the parameter (e.g. `T`, `K`, `V`).
    pub name: String,
    /// Index of the symbol that introduces this parameter (function, type,
    /// method). Used to delimit the parameter's scope.
    pub owner_symbol_index: usize,
    /// Optional upper-bound type (e.g. `T extends Animal` in TS).
    pub bound: Option<TypeId>,
}

/// The structured form every value, parameter, return, and field receives once
/// the engine has interned it. Strings appear only inside `Class(QName)` and
/// `Literal(LitValue::Str)`; every other type relationship is by TypeId.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Type {
    /// Nominal reference identified by fully qualified name.
    Class(String),
    /// Built-in scalar.
    Primitive(PrimKind),
    /// Callable signature.
    Function {
        params: Vec<TypeId>,
        return_: TypeId,
    },
    /// Positional fixed-length aggregate.
    Tuple(Vec<TypeId>),
    /// Sum of disjoint types. Member lookup returns the intersection of
    /// members across branches.
    Union(Vec<TypeId>),
    /// Combined member set. Member lookup returns the union of members
    /// across branches.
    Intersection(Vec<TypeId>),
    /// Generic application: `List<User>`, `Map<K, V>`, `Promise<Result>`.
    /// `base` resolves to a Class or TypeAlias.
    Apply { base: TypeId, args: Vec<TypeId> },
    /// In-scope generic parameter.
    Generic { param: GenericParamId },
    /// Nullable wrapper. Engine looks through it for member resolution when
    /// `LanguageProfile::look_through_optional` is true.
    Optional(TypeId),
    /// Async wrapper. `await` unwraps to the inner type.
    AsyncWrapper(TypeId),
    /// Iterable wrapper. `for x in collection` binds `x` to the inner type.
    Iterator(TypeId),
    /// Singleton type (literal types).
    Literal(LitValue),
    /// Engine bailout — member lookup fails closed rather than guessing.
    Unknown,
}

#[derive(Default)]
struct TypeArenaInner {
    types: Vec<Type>,
    intern: FxHashMap<Type, TypeId>,
    qname_to_class: FxHashMap<String, TypeId>,
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
        inner.types.push(ty.clone());
        inner.intern.insert(ty, id);
        id
    }

    /// Look up an existing TypeId for a `Type` without inserting.
    pub fn lookup(&self, ty: &Type) -> Option<TypeId> {
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

    /// Rewrite every `Class(name)` whose `name` is a key of `params` to that
    /// param's `Type::Generic` id, recursing through structural types.
    /// `intern_type_str` is param-blind — it interns a generic return like
    /// `Iter<T>` as `Apply{Iter,[Class("T")]}`. Applying this with the owning
    /// type's `{name → Type::Generic id}` map turns the nominal `Class("T")`
    /// into the bindable `Generic(T)` so the chain walker's `substitute` can
    /// resolve it against the receiver's bound args.
    pub fn rebind_class_params(&self, id: TypeId, params: &FxHashMap<String, TypeId>) -> TypeId {
        match self.get(id) {
            Type::Class(name) => params.get(&name).copied().unwrap_or(id),
            Type::Apply { base, args } => {
                let base = self.rebind_class_params(base, params);
                let args = args
                    .iter()
                    .map(|&a| self.rebind_class_params(a, params))
                    .collect();
                self.intern(Type::Apply { base, args })
            }
            Type::Optional(inner) => {
                let inner = self.rebind_class_params(inner, params);
                self.intern(Type::Optional(inner))
            }
            Type::AsyncWrapper(inner) => {
                let inner = self.rebind_class_params(inner, params);
                self.intern(Type::AsyncWrapper(inner))
            }
            Type::Iterator(inner) => {
                let inner = self.rebind_class_params(inner, params);
                self.intern(Type::Iterator(inner))
            }
            Type::Tuple(elems) => {
                let elems = elems
                    .iter()
                    .map(|&e| self.rebind_class_params(e, params))
                    .collect();
                self.intern(Type::Tuple(elems))
            }
            Type::Union(branches) => {
                let branches = branches
                    .iter()
                    .map(|&b| self.rebind_class_params(b, params))
                    .collect();
                self.intern(Type::Union(branches))
            }
            Type::Intersection(branches) => {
                let branches = branches
                    .iter()
                    .map(|&b| self.rebind_class_params(b, params))
                    .collect();
                self.intern(Type::Intersection(branches))
            }
            Type::Function {
                params: ps,
                return_,
            } => {
                let ps = ps
                    .iter()
                    .map(|&p| self.rebind_class_params(p, params))
                    .collect();
                let return_ = self.rebind_class_params(return_, params);
                self.intern(Type::Function {
                    params: ps,
                    return_,
                })
            }
            Type::Primitive(_) | Type::Generic { .. } | Type::Literal(_) | Type::Unknown => id,
        }
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
            Type::Primitive(p) => {
                let _ = write!(out, "{p:?}");
            }
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

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
