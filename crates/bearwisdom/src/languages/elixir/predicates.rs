// =============================================================================
// elixir/predicates.rs — Elixir builtin and helper predicates
// =============================================================================

use crate::types::EdgeKind;

/// Check that the edge kind is compatible with the symbol kind.
pub(crate) fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::Calls => matches!(
            sym_kind,
            "method" | "function" | "constructor" | "test" | "property" | "module"
        ),
        EdgeKind::Inherits => matches!(sym_kind, "class" | "module"),
        EdgeKind::Implements => matches!(sym_kind, "class" | "module" | "interface"),
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "class" | "module" | "interface" | "enum" | "type_alias" | "namespace"
        ),
        EdgeKind::Instantiates => matches!(sym_kind, "class" | "module"),
        _ => true,
    }
}

/// A reserved Elixir language form: a `Kernel.SpecialForms` compiler form or a
/// `Kernel` def-family / control-flow macro. These look like bare calls in the
/// AST but are syntax, not project functions — so they decline before the
/// resolution ladder (via `builtin_skip`) and never bind to a same-named
/// project symbol. This is the closed set of language constructs only; library
/// Kernel functions (`is_nil`, `elem`, `hd`, `tap`, …) are NOT here.
pub(crate) fn is_elixir_special_form(name: &str) -> bool {
    matches!(
        name,
        // Kernel.SpecialForms — compiler special forms.
        "__CALLER__"
            | "__DIR__"
            | "__ENV__"
            | "__MODULE__"
            | "__STACKTRACE__"
            | "__aliases__"
            | "__block__"
            | "alias"
            | "case"
            | "cond"
            | "fn"
            | "for"
            | "import"
            | "quote"
            | "receive"
            | "require"
            | "super"
            | "try"
            | "unquote"
            | "unquote_splicing"
            | "with"
            // Kernel — def-family declaration macros.
            | "def"
            | "defp"
            | "defmodule"
            | "defmacro"
            | "defmacrop"
            | "defstruct"
            | "defexception"
            | "defprotocol"
            | "defimpl"
            | "defguard"
            | "defguardp"
            | "defdelegate"
            | "defoverridable"
            // Kernel — control-flow macros.
            | "if"
            | "unless"
    )
}

/// The closed Elixir / Erlang-OTP standard-library module set — the runtime
/// every Elixir project links unconditionally (lowercase OTP atoms, the Elixir
/// stdlib modules, the built-in exception types, and the toolchain-bundled
/// `ExUnit` / `Mix`). This is the language/runtime substrate, not a dependency
/// list. Hex-package modules are classified via the `mix.exs` manifest at the
/// resolver hooks, never here.
const STDLIB_MODULES: &[&str] = &[
    // Erlang/OTP (bare atoms, lowercase)
    "erlang",
    "lists",
    "maps",
    "string",
    "io",
    "file",
    "timer",
    "ets",
    "dets",
    "mnesia",
    "gen_server",
    "gen_event",
    "gen_statem",
    "supervisor",
    "application",
    "code",
    "crypto",
    "os",
    "net_kernel",
    "node",
    "rpc",
    "proc_lib",
    "sys",
    // Elixir stdlib
    "Elixir",
    "Kernel",
    "IO",
    "Enum",
    "Map",
    "MapSet",
    "List",
    "String",
    "Integer",
    "Float",
    "Atom",
    "Tuple",
    "Process",
    "Port",
    "Node",
    "File",
    "Path",
    "System",
    "Code",
    "Macro",
    "Module",
    "Agent",
    "Task",
    "GenServer",
    "GenEvent",
    "GenStateMachine",
    "Supervisor",
    "Application",
    "Registry",
    "DynamicSupervisor",
    "PartitionSupervisor",
    "Stream",
    "Range",
    "Regex",
    "URI",
    "DateTime",
    "Date",
    "Time",
    "NaiveDateTime",
    "Calendar",
    "Duration",
    "Keyword",
    "Access",
    "Bitwise",
    "Base",
    "Protocol",
    "Behaviour",
    "Inspect",
    "Collectable",
    "Enumerable",
    "OptionParser",
    "StringIO",
    "Version",
    "Config",
    "Function",
    "Record",
    "Set",
    "Dict",
    "HashDict",
    "HashSet",
    // Elixir exception types
    "ArgumentError",
    "ArithmeticError",
    "BadArityError",
    "BadBooleanError",
    "BadFunctionError",
    "BadMapError",
    "BadStructError",
    "CaseClauseError",
    "CompileError",
    "CondClauseError",
    "ErlangError",
    "FunctionClauseError",
    "KeyError",
    "MatchError",
    "RuntimeError",
    "SyntaxError",
    "SystemLimitError",
    "TokenMissingError",
    "TryClauseError",
    "UndefinedFunctionError",
    "WithClauseError",
    "UnicodeConversionError",
    // Toolchain-bundled (ship with every Elixir install, not Hex packages)
    "ExUnit",
    "Mix",
];

/// Check whether an Elixir module alias is a standard-library / OTP runtime
/// module. Hex-package modules are NOT recognized here — they are classified
/// from the project's `mix.exs` dependency list at the resolver hooks. A
/// dependency module without a `mix.exs` declaration is honestly unresolved.
pub(crate) fn is_external_elixir_module(module: &str) -> bool {
    // The root segment of the module (before the first `.`).
    let root = module.split('.').next().unwrap_or(module);
    for &ext in STDLIB_MODULES {
        if root == ext {
            return true;
        }
    }
    false
}
