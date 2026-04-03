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
