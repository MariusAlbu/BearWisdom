// =============================================================================
// type_checker/profile/syntax_specs.rs — spec vocabulary for the extractor-side syntax axes:
// constructor patterns, decorators, and class-builder call shapes.
// =============================================================================

use crate::types::Visibility;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstructorPattern {
    /// `new Foo()` — JS, TS, Java, C#, PHP, Dart.
    New,
    /// `Foo()` — Python, R6 (when invoked via constructor), most dynamic
    /// languages.
    CallableClass,
    /// `Foo.new(...)` — Ruby, Crystal.
    ClassDotNew,
    /// `Foo::new(...)` — Rust convention.
    TypeColonColonNew,
    /// `Foo::build(...)` and other named ::-prefixed factory conventions.
    TypeColonColonBuild,
    /// `Foo->new(...)` — Perl, valid in PHP.
    ArrowNew,
    /// `Foo$new(...)` — R R6 dispatch.
    R6DollarNew,
    /// `new("Foo", ...)` — R S4.
    S4New,
    /// `Foo:new(...)` — Lua method-call convention on metatable factories.
    LuaColonNew,
    /// Free-function call returning a known type. Pattern matched against
    /// the callee name.
    NamedFactory { fn_name_pattern: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoratorSyntax {
    /// Python `@dataclass`, TS `@Component`, Java `@Override`, Kotlin
    /// `@Suppress`.
    AtPrefix,
    /// Rust `#[derive(...)]`, `#[cfg(...)]`.
    HashBracket,
    /// C# `[Test]`, `[HttpGet]`.
    AttrBracket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgKey {
    Named(&'static str),
    Positional(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassNameSource {
    /// First positional arg, e.g. `R6Class("Foo", ...)`.
    ArgIndex(usize),
    /// Name from the assignment LHS: `MyClass <- ...`.
    LhsIdentifier,
    /// Prefer LHS, fall back to positional arg at the given index.
    LhsThenArg(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BucketContainer {
    /// `list(a = function(...) ...)` — R / R6.
    ListCall(&'static str),
    /// `{ a = function(...) end }` — Lua table constructor.
    TableConstructor,
    /// `{ "a": fn ... }` — Python dict literal.
    DictLiteral,
    /// `do ... end` block — Ruby, Elixir.
    DoBlock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberShape {
    /// `name = function(args) body` — R / R6 / Lua-style.
    NameEqFunction,
    /// `function name(args) body end` — Lua.
    FunctionDefStmt,
    /// `def name(args): body` — Python.
    DefStmt,
    /// `(key, function)` pair in a dict / table literal.
    KeyValueFunc,
}

pub struct MethodBucket {
    pub arg: ArgKey,
    pub container: BucketContainer,
    pub member_shape: MemberShape,
    pub visibility: Visibility,
}

pub struct ClassBuilderSpec {
    /// Function or macro name that builds a class — `R6Class`, `ggproto`,
    /// `setClass`, `Struct.new`, `setmetatable`.
    pub callee: &'static str,
    /// Namespaces accepted as prefixes (`R6::R6Class` etc.).
    pub accepted_namespaces: &'static [&'static str],
    /// Where to find the class name in the call expression.
    pub class_name_source: ClassNameSource,
    /// Named or positional args holding method lists, with their shape.
    pub method_buckets: &'static [MethodBucket],
    /// Named or positional arg holding the superclass — emits Inherits ref
    /// when present.
    pub inherits_arg: Option<ArgKey>,
}

