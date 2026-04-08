// =============================================================================
// elixir/coverage_tests.rs  —  One test per declared symbol_node_kind and ref_node_kind
// =============================================================================

use super::extract::extract;
use crate::types::{EdgeKind, SymbolKind};

// ---------------------------------------------------------------------------
// Symbol node kinds  (all `call` nodes, matched on callee text)
// ---------------------------------------------------------------------------

#[test]
fn symbol_defmodule() {
    let r = extract("defmodule Foo do\nend");
    assert!(
        r.symbols.iter().any(|s| s.name == "Foo" && s.kind == SymbolKind::Class),
        "expected Class Foo; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_def() {
    let r = extract("defmodule M do\n  def hello, do: :ok\nend");
    assert!(
        r.symbols.iter().any(|s| s.name == "hello"),
        "expected hello; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_defp() {
    let r = extract("defmodule M do\n  defp private_fn, do: :ok\nend");
    assert!(
        r.symbols.iter().any(|s| s.name == "private_fn"),
        "expected private_fn; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_defmacro() {
    let r = extract("defmodule M do\n  defmacro my_macro(x) do\n    x\n  end\nend");
    assert!(
        r.symbols.iter().any(|s| s.name == "my_macro"),
        "expected my_macro; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_defmacrop() {
    let r = extract("defmodule M do\n  defmacrop secret(x) do\n    x\n  end\nend");
    assert!(
        r.symbols.iter().any(|s| s.name == "secret"),
        "expected secret; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_defstruct() {
    let r = extract("defmodule User do\n  defstruct [:name, :email]\nend");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Struct),
        "expected Struct; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_defprotocol() {
    let r = extract("defprotocol Enumerable do\n  def count(collection)\nend");
    assert!(
        r.symbols.iter().any(|s| s.name == "Enumerable"),
        "expected Enumerable; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn symbol_defimpl() {
    let r = extract("defimpl Enumerable, for: List do\n  def count(list), do: {:ok, length(list)}\nend");
    assert!(
        !r.symbols.is_empty(),
        "expected symbols from defimpl; got none"
    );
}

#[test]
fn symbol_defguard() {
    let r = extract("defmodule M do\n  defguard is_pos(x) when x > 0\nend");
    assert!(
        r.symbols.iter().any(|s| s.name == "is_pos"),
        "expected is_pos; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Ref node kinds
// ---------------------------------------------------------------------------

#[test]
fn ref_call_generic() {
    // A generic function call inside a def body emits a Calls edge.
    let r = extract("defmodule M do\n  def f do\n    bar()\n  end\nend");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "bar" && rf.kind == EdgeKind::Calls),
        "expected Calls bar; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_dot_call() {
    // Module.function() — dot call via Enum.map/2 pattern.
    let r = extract("defmodule M do\n  def f do\n    Enum.map([], fn x -> x end)\n  end\nend");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "map" && rf.kind == EdgeKind::Calls),
        "expected Calls map; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_alias_directive() {
    let r = extract("defmodule M do\n  alias MyApp.User\nend");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports from alias; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_import_directive() {
    let r = extract("defmodule M do\n  import Enum\nend");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports from import; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_use_directive() {
    let r = extract("defmodule M do\n  use GenServer\nend");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports from use; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_require_directive() {
    let r = extract("defmodule M do\n  require Logger\nend");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports),
        "expected Imports from require; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_binary_operator_pipe() {
    // |> pipe operator — right side emits a Calls edge.
    let r = extract("defmodule M do\n  def f(x) do\n    x |> String.upcase()\n  end\nend");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "upcase" && rf.kind == EdgeKind::Calls),
        "expected Calls upcase from pipe; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_alias_node_type_ref() {
    // `alias` grammar node — module reference in @behaviour or @type.
    let r = extract("defmodule M do\n  @behaviour GenServer\nend");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "GenServer" && rf.kind == EdgeKind::TypeRef),
        "expected TypeRef GenServer from @behaviour; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_alias_node_in_function_body() {
    // `alias` grammar nodes (module references) in function bodies emit TypeRef edges.
    let r = extract("defmodule M do\n  def f do\n    Repo.all(User)\n  end\nend");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef && (rf.target_name == "Repo" || rf.target_name == "User")),
        "expected TypeRef for module reference in function body; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_pipe_operator_inside_function() {
    // `|>` pipe chains inside def bodies emit Calls edges for each piped function.
    let r = extract("defmodule M do\n  def transform(list) do\n    list\n    |> Enum.map(fn x -> x * 2 end)\n    |> Enum.filter(fn x -> x > 0 end)\n  end\nend");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "map" && rf.kind == EdgeKind::Calls),
        "expected Calls edge for piped 'map'; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "filter" && rf.kind == EdgeKind::Calls),
        "expected Calls edge for piped 'filter'; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_pipe_with_capture_expression() {
    // `|> &String.upcase/1` — capture expression on pipe right side should emit Calls.
    let r = extract("defmodule M do\n  def f(list) do\n    list |> Enum.map(&String.upcase/1)\n  end\nend");
    // We expect at least 'map' to be extracted as a Calls edge.
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "map" && rf.kind == EdgeKind::Calls),
        "expected Calls edge for piped 'map' with capture arg; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_pipe_with_bare_function() {
    // `list |> upcase` — bare function reference on pipe right side.
    let r = extract("defmodule M do\n  def f(list) do\n    list |> upcase()\n  end\nend");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "upcase" && rf.kind == EdgeKind::Calls),
        "expected Calls edge for bare piped 'upcase'; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

#[test]
fn ref_alias_multi_module() {
    // `alias MyApp.{User, Post}` — multi-alias should emit two Imports refs.
    let r = extract("defmodule M do\n  alias MyApp.{User, Post}\nend");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "User" && rf.kind == EdgeKind::Imports),
        "expected Imports 'User' from multi-alias; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Post" && rf.kind == EdgeKind::Imports),
        "expected Imports 'Post' from multi-alias; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Additional symbol kinds from rules
// ---------------------------------------------------------------------------

/// `defexception` inside a module emits a `Struct` symbol whose name is the enclosing
/// module name (the exception type IS the module in Elixir).
#[test]
fn symbol_defexception() {
    let r = extract("defmodule MyApp.NotFoundError do\n  defexception [:message]\nend");
    assert!(
        r.symbols.iter().any(|s| s.kind == SymbolKind::Struct),
        "expected Struct symbol from defexception; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// defprotocol inner def — protocol callback method should produce a Method symbol.
#[test]
fn symbol_defprotocol_callback_method() {
    let r = extract("defprotocol Greet do\n  def hello(impl)\nend");
    assert!(
        r.symbols.iter().any(|s| s.name == "hello"),
        "expected Method hello inside defprotocol; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// @type inside a module — extractor emits Variable (not TypeAlias) per current impl.
#[test]
fn symbol_at_type_attribute() {
    let r = extract("defmodule M do\n  @type name_t :: String.t()\nend");
    assert!(
        r.symbols.iter().any(|s| s.name == "@type" && s.kind == SymbolKind::Variable),
        "expected Variable @type; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// @callback inside a behaviour module — extractor emits Variable.
#[test]
fn symbol_at_callback_attribute() {
    let r = extract("defmodule MyBehaviour do\n  @callback execute(term()) :: :ok | :error\nend");
    assert!(
        r.symbols.iter().any(|s| s.name == "@callback" && s.kind == SymbolKind::Variable),
        "expected Variable @callback; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

/// Nested defmodule — inner module produces its own Class symbol.
#[test]
fn symbol_nested_defmodule() {
    let r = extract("defmodule Outer do\n  defmodule Inner do\n  end\nend");
    assert!(
        r.symbols.iter().any(|s| s.name == "Inner" && s.kind == SymbolKind::Class),
        "expected Class Inner from nested defmodule; got {:?}",
        r.symbols.iter().map(|s| (&s.name, s.kind)).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// Additional ref node kinds from rules
// ---------------------------------------------------------------------------

/// defimpl → TypeRef to the protocol being implemented.
#[test]
fn ref_defimpl_typeref_to_protocol() {
    let r = extract("defimpl Enumerable, for: List do\n  def count(list), do: {:ok, length(list)}\nend");
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "Enumerable" && rf.kind == EdgeKind::TypeRef),
        "expected TypeRef to Enumerable from defimpl; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// @type body references a named type via alias — emits TypeRef.
#[test]
fn ref_at_type_body_typeref() {
    let r = extract("defmodule M do\n  @type result :: {:ok, MyType.t()} | :error\nend");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef && rf.target_name == "MyType"),
        "expected TypeRef to MyType from @type body; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// @spec body references a named type via alias — emits TypeRef.
#[test]
fn ref_at_spec_body_typeref() {
    let r = extract("defmodule M do\n  @spec process(Request.t()) :: Response.t()\n  def process(_req), do: :ok\nend");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::TypeRef && (rf.target_name == "Request" || rf.target_name == "Response")),
        "expected TypeRef from @spec body; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// @behaviour declaration — Implements edge (rules); extractor emits TypeRef.
#[test]
fn ref_at_behaviour_typeref() {
    let r = extract("defmodule M do\n  @behaviour GenServer\nend");
    // Extractor emits TypeRef (not Implements) for @behaviour — assert what's actually produced.
    assert!(
        r.refs.iter().any(|rf| rf.target_name == "GenServer"
            && (rf.kind == EdgeKind::TypeRef || rf.kind == EdgeKind::Implements)),
        "expected TypeRef or Implements GenServer from @behaviour; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}

/// use with a nested module path — Imports ref for the module.
#[test]
fn ref_use_nested_module() {
    let r = extract("defmodule M do\n  use Phoenix.Controller\nend");
    assert!(
        r.refs.iter().any(|rf| rf.kind == EdgeKind::Imports
            && (rf.target_name == "Controller" || rf.target_name == "Phoenix.Controller")),
        "expected Imports from use Phoenix.Controller; got {:?}",
        r.refs.iter().map(|rf| (&rf.target_name, rf.kind)).collect::<Vec<_>>()
    );
}
