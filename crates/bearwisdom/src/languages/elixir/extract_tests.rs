    use super::*;
    use crate::types::{EdgeKind, SymbolKind, Visibility};

    #[test]
    fn extracts_module_and_functions() {
        let src = r#"
defmodule MyApp.Greeter do
  def hello(name) do
    "Hello, #{name}!"
  end

  defp private_helper do
    :ok
  end
end
"#;
        let r = extract::extract(src);
        let module = r.symbols.iter().find(|s| s.name == "MyApp.Greeter" || s.name == "Greeter").expect("module");
        assert_eq!(module.kind, SymbolKind::Class);

        let hello = r.symbols.iter().find(|s| s.name == "hello").expect("hello");
        assert_eq!(hello.kind, SymbolKind::Method);
        assert_eq!(hello.visibility, Some(Visibility::Public));

        let helper = r.symbols.iter().find(|s| s.name == "private_helper").expect("private_helper");
        assert_eq!(helper.visibility, Some(Visibility::Private));
    }

    #[test]
    fn alias_produces_import_ref() {
        let src = r#"
defmodule Foo do
  alias MyApp.Repo
  alias MyApp.Models.User
end
"#;
        let r = extract::extract(src);
        let imports: Vec<_> = r.refs.iter().filter(|r| r.kind == EdgeKind::Imports).collect();
        let targets: Vec<&str> = imports.iter().map(|r| r.target_name.as_str()).collect();
        assert!(targets.contains(&"Repo"), "missing Repo: {targets:?}");
        assert!(targets.contains(&"User"), "missing User: {targets:?}");
    }

    #[test]
    fn defstruct_produces_struct_symbol() {
        let src = r#"
defmodule MyApp.User do
  defstruct [:name, :email]
end
"#;
        let r = extract::extract(src);
        assert!(
            r.symbols.iter().any(|s| s.kind == SymbolKind::Struct),
            "expected a Struct symbol from defstruct"
        );
    }

    #[test]
    fn behaviour_attribute_emits_typeref() {
        let src = r#"
defmodule MyApp.Worker do
  @behaviour GenServer

  def init(state), do: {:ok, state}
end
"#;
        let r = extract::extract(src);
        let typerefs: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::TypeRef)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(
            typerefs.contains(&"GenServer"),
            "Expected TypeRef for GenServer from @behaviour: {typerefs:?}"
        );
    }

    #[test]
    fn pipe_operator_calls_extracted() {
        let src = r#"
defmodule Transform do
  def run(users) do
    users
    |> Enum.map(fn u -> u.name end)
    |> Enum.filter(fn n -> String.length(n) > 0 end)
  end
end
"#;
        let r = extract::extract(src);
        let call_names: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        // Enum.map and Enum.filter piped calls should appear
        assert!(
            call_names.contains(&"map") || call_names.iter().any(|n| n.contains("map")),
            "Expected 'map' call from pipe: {call_names:?}"
        );
        assert!(
            call_names.contains(&"filter") || call_names.iter().any(|n| n.contains("filter")),
            "Expected 'filter' call from pipe: {call_names:?}"
        );
    }

    #[test]
    fn function_calls_inside_body_extracted() {
        let src = r#"
defmodule MyApp.Repo do
  def save(record) do
    validate(record)
    persist(record)
  end
end
"#;
        let r = extract::extract(src);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"validate"), "Missing 'validate': {calls:?}");
        assert!(calls.contains(&"persist"),  "Missing 'persist': {calls:?}");
    }

    #[test]
    fn defprotocol_extracted_as_interface() {
        let src = r#"
defprotocol Stringify do
  def to_string(value)
end
"#;
        let r = extract::extract(src);
        let proto = r
            .symbols
            .iter()
            .find(|s| s.name == "Stringify")
            .expect("Stringify protocol not found");
        assert_eq!(proto.kind, SymbolKind::Interface, "expected Interface kind for defprotocol");
        // The protocol's callback should be extracted as a child function.
        assert!(
            r.symbols.iter().any(|s| s.name == "to_string"),
            "expected to_string callback; symbols: {:?}",
            r.symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn defimpl_extracted_as_namespace() {
        let src = r#"
defimpl Stringify, for: Integer do
  def to_string(value), do: Integer.to_string(value)
end
"#;
        let r = extract::extract(src);
        let impl_sym = r
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Namespace)
            .expect("defimpl should produce a Namespace symbol");
        assert_eq!(impl_sym.name, "Stringify");
        // Should also produce the to_string function inside the impl.
        assert!(
            r.symbols.iter().any(|s| s.name == "to_string"),
            "expected to_string method inside impl"
        );
    }

    #[test]
    fn deeply_nested_function_calls_in_anonymous_functions() {
        let src = r#"
defmodule DataProcessor do
  def process_items(items) do
    Enum.map(items, fn item ->
      transform(item)
      |> validate()
      |> persist()
    end)
  end

  defp transform(x), do: x
  defp validate(x), do: x
  defp persist(x), do: x
end
"#;
        let r = extract::extract(src);
        let calls: Vec<&str> = r.refs.iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        // Verify all calls including those in anonymous functions are captured
        assert!(calls.contains(&"map"), "expected 'map' call: {calls:?}");
        assert!(calls.contains(&"transform"), "expected 'transform' call: {calls:?}");
        assert!(calls.contains(&"validate"), "expected 'validate' call: {calls:?}");
        assert!(calls.contains(&"persist"), "expected 'persist' call: {calls:?}");
    }

    #[test]
    fn dot_call_receiver_lowercase_is_not_typeref() {
        // `conn.something()` and `assigns.user` are struct/map field accesses,
        // not module references. Receiver must NOT emit a TypeRef.
        // Uppercase receiver (`Enum.map`) is a real module ref and SHOULD emit.
        let src = r#"
defmodule MyApp.Web do
  def show(conn, _params) do
    conn = fetch_session(conn)
    user = conn.assigns.current_user
    Enum.map([1, 2, 3], fn x -> x + 1 end)
  end
end
"#;
        let r = extract::extract(src);
        let typerefs: Vec<&str> = r.refs.iter()
            .filter(|rf| rf.kind == EdgeKind::TypeRef)
            .map(|rf| rf.target_name.as_str())
            .collect();
        assert!(
            !typerefs.contains(&"conn"),
            "lowercase 'conn' (parameter) must NOT emit TypeRef; got {typerefs:?}"
        );
        assert!(
            !typerefs.contains(&"assigns"),
            "lowercase 'assigns' must NOT emit TypeRef; got {typerefs:?}"
        );
        assert!(
            typerefs.contains(&"Enum"),
            "uppercase module 'Enum' SHOULD emit TypeRef; got {typerefs:?}"
        );
    }

    #[test]
    #[ignore]
    fn diag_alias_with_as_ast_dump() {
        use tree_sitter::Parser;
        let src = "defmodule X do\n  alias PlausibleWeb.Api.Helpers, as: H\nend\n";
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_elixir::LANGUAGE.into()).unwrap();
        let tree = parser.parse(src, None).unwrap();
        fn walk(node: tree_sitter::Node, src: &str, depth: usize) {
            let text = if node.start_byte() < src.len() && node.end_byte() <= src.len() {
                &src[node.start_byte()..node.end_byte().min(node.start_byte() + 60)]
            } else { "" };
            eprintln!("{}{} [{}..{}] {:?}",
                "  ".repeat(depth), node.kind(), node.start_byte(), node.end_byte(),
                text.replace('\n', "\\n"));
            let mut c = node.walk();
            for child in node.children(&mut c) {
                walk(child, src, depth + 1);
            }
        }
        walk(tree.root_node(), src, 0);
    }

    #[test]
    fn alias_with_as_uses_alias_as_imported_name() {
        // `alias PlausibleWeb.Api.Helpers, as: H` must emit an Imports ref
        // with target_name = "H" (the alias), not "Helpers" (the last
        // segment of the module). Without this, the resolver's alias
        // lookup loop never matches a user reference to `H`.
        let src = r#"
defmodule MyApp.Controller do
  alias PlausibleWeb.Api.Helpers, as: H

  def show(conn) do
    H.bad_request(conn, "missing")
  end
end
"#;
        let r = extract::extract(src);
        let imports: Vec<(&str, Option<&str>)> = r.refs.iter()
            .filter(|rf| rf.kind == EdgeKind::Imports)
            .map(|rf| (rf.target_name.as_str(), rf.module.as_deref()))
            .collect();
        assert!(
            imports.iter().any(|(t, m)| *t == "H" && *m == Some("PlausibleWeb.Api.Helpers")),
            "expected Imports(target=H, module=PlausibleWeb.Api.Helpers); got {imports:?}"
        );
    }

    #[test]
    fn alias_without_as_keeps_module_last_segment() {
        // Plain `alias MyApp.Foo` (no `as:`) keeps the default behavior:
        // imported_name = "Foo".
        let src = r#"
defmodule MyApp.Bar do
  alias MyApp.Foo
end
"#;
        let r = extract::extract(src);
        let imports: Vec<(&str, Option<&str>)> = r.refs.iter()
            .filter(|rf| rf.kind == EdgeKind::Imports)
            .map(|rf| (rf.target_name.as_str(), rf.module.as_deref()))
            .collect();
        assert!(
            imports.iter().any(|(t, m)| *t == "Foo" && *m == Some("MyApp.Foo")),
            "expected Imports(target=Foo, module=MyApp.Foo); got {imports:?}"
        );
    }

    #[test]
    fn dot_call_receiver_lowercase_is_not_call() {
        // `session.acquisition_channel` is field access, not a function call.
        // tree-sitter-elixir parses it as a `call` node anyway; the extractor
        // must NOT emit a Calls edge with `module=session` because `session`
        // isn't a module — it's a local variable.
        // Real module calls like `Enum.map(...)` and bare calls like `foo()`
        // SHOULD still emit.
        let src = r#"
defmodule MyApp.Test do
  def check(session, email) do
    assert session.acquisition_channel == "Cross-network"
    email.html_body
    Enum.map([1, 2], fn x -> x end)
    helper()
  end

  defp helper, do: :ok
end
"#;
        let r = extract::extract(src);
        let calls: Vec<(&str, Option<&str>)> = r.refs.iter()
            .filter(|rf| rf.kind == EdgeKind::Calls)
            .map(|rf| (rf.target_name.as_str(), rf.module.as_deref()))
            .collect();
        assert!(
            !calls.iter().any(|(t, m)| *t == "acquisition_channel" && *m == Some("session")),
            "lowercase 'session.acquisition_channel' must NOT emit Calls; got {calls:?}"
        );
        assert!(
            !calls.iter().any(|(t, m)| *t == "html_body" && *m == Some("email")),
            "lowercase 'email.html_body' must NOT emit Calls; got {calls:?}"
        );
        assert!(
            calls.iter().any(|(t, m)| *t == "map" && *m == Some("Enum")),
            "uppercase 'Enum.map' SHOULD emit Calls; got {calls:?}"
        );
        assert!(
            calls.iter().any(|(t, m)| *t == "helper" && m.is_none()),
            "bare call 'helper()' SHOULD emit Calls with no module; got {calls:?}"
        );
    }

