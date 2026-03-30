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
        let r = extract(src);
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
        let r = extract(src);
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
        let r = extract(src);
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
        let r = extract(src);
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
        let r = extract(src);
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
        let r = extract(src);
        let calls: Vec<&str> = r
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Calls)
            .map(|r| r.target_name.as_str())
            .collect();
        assert!(calls.contains(&"validate"), "Missing 'validate': {calls:?}");
        assert!(calls.contains(&"persist"),  "Missing 'persist': {calls:?}");
    }


