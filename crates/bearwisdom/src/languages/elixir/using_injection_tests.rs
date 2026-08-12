use super::*;
use crate::types::{FlowMeta, ParsedFile};

fn ex_file(path: &str, src: &str) -> ParsedFile {
    ParsedFile {
        path: path.to_string(),
        language: "elixir".to_string(),
        content_hash: String::new(),
        size: src.len() as u64,
        line_count: src.lines().count() as u32,
        mtime: None,
        package_id: None,
        symbols: Vec::new(),
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: Some(src.to_string()),
        has_errors: false,
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

#[test]
fn defmacro_using_collects_import_alias_and_nested_use() {
    let src = r#"
defmodule MyApp.DataCase do
  defmacro __using__(_) do
    quote do
      use MyApp.TestUtils
      import MyApp.Factory
      alias MyApp.Repo
    end
  end
end
"#;
    let state = build_using_injection_map(&[ex_file("lib/data_case.ex", src)], std::path::Path::new(""));
    let set = state
        .injections_for("MyApp.DataCase")
        .expect("DataCase has an injection set");
    assert!(set.contains(&ElixirInjection::Use {
        module: "MyApp.TestUtils".to_string()
    }));
    assert!(set.contains(&ElixirInjection::Import {
        module: "MyApp.Factory".to_string()
    }));
    assert!(set.contains(&ElixirInjection::Alias {
        local: "Repo".to_string(),
        module: "MyApp.Repo".to_string()
    }));
}

#[test]
fn using_do_casetemplate_form_recognized() {
    // ExUnit `CaseTemplate` exposes `using do … end` instead of
    // `defmacro __using__`.
    let src = r#"
defmodule MyApp.ConnCase do
  use ExUnit.CaseTemplate

  using do
    quote do
      import MyApp.Factory
      alias MyApp.Repo
    end
  end
end
"#;
    let state = build_using_injection_map(&[ex_file("lib/conn_case.ex", src)], std::path::Path::new(""));
    let set = state
        .injections_for("MyApp.ConnCase")
        .expect("ConnCase has an injection set");
    assert!(set.contains(&ElixirInjection::Import {
        module: "MyApp.Factory".to_string()
    }));
    assert!(set.contains(&ElixirInjection::Alias {
        local: "Repo".to_string(),
        module: "MyApp.Repo".to_string()
    }));
}

#[test]
fn alias_as_rename_records_local_name() {
    let src = r#"
defmodule MyApp.WebCase do
  defmacro __using__(_) do
    quote do
      alias MyApp.Router.Helpers, as: Routes
    end
  end
end
"#;
    let state = build_using_injection_map(&[ex_file("lib/web_case.ex", src)], std::path::Path::new(""));
    let set = state.injections_for("MyApp.WebCase").unwrap();
    assert!(set.contains(&ElixirInjection::Alias {
        local: "Routes".to_string(),
        module: "MyApp.Router.Helpers".to_string()
    }));
}

#[test]
fn multi_alias_expands_each_name() {
    let src = r#"
defmodule MyApp.Case do
  defmacro __using__(_) do
    quote do
      alias MyApp.{User, Repo}
    end
  end
end
"#;
    let state = build_using_injection_map(&[ex_file("lib/case.ex", src)], std::path::Path::new(""));
    let set = state.injections_for("MyApp.Case").unwrap();
    assert!(set.contains(&ElixirInjection::Alias {
        local: "User".to_string(),
        module: "MyApp.User".to_string()
    }));
    assert!(set.contains(&ElixirInjection::Alias {
        local: "Repo".to_string(),
        module: "MyApp.Repo".to_string()
    }));
}

#[test]
fn no_using_block_yields_no_injection_entry() {
    let src = r#"
defmodule MyApp.Plain do
  def hello, do: :ok
end
"#;
    let state = build_using_injection_map(&[ex_file("lib/plain.ex", src)], std::path::Path::new(""));
    assert!(state.injections_for("MyApp.Plain").is_none());
}

#[test]
fn literal_def_in_using_quote_becomes_def_injection() {
    // ExMachina's `__using__` defines `build/2` directly in its quote block —
    // no `import`/`use`/`alias` involved, just a literal function.
    let src = r#"
defmodule MyApp.Machina do
  defmacro __using__(_opts) do
    quote do
      def build(factory_name, attrs \\ %{}) do
        MyApp.Machina.build(__MODULE__, factory_name, attrs)
      end

      defp raise_replaced_error(name) do
        raise name
      end
    end
  end
end
"#;
    let state = build_using_injection_map(&[ex_file("lib/machina.ex", src)], std::path::Path::new(""));
    let set = state.injections_for("MyApp.Machina").unwrap();
    assert!(set.contains(&ElixirInjection::Def {
        name: "build".to_string(),
        is_macro: false
    }));
    assert!(set.contains(&ElixirInjection::Def {
        name: "raise_replaced_error".to_string(),
        is_macro: false
    }));
}

#[test]
fn plain_top_level_use_becomes_own_injection() {
    // `Plausible.Factory`-shaped module: no `using`/`defmacro __using__` of
    // its own, just a plain `use` at the top level. Elixir still compiles
    // whatever that `use` injects directly into this module, so it must
    // count as this module's own injection set too.
    let src = r#"
defmodule MyApp.Factory do
  use MyApp.Machina.Ecto, repo: MyApp.Repo

  def user_factory do
    %MyApp.User{}
  end
end
"#;
    let state = build_using_injection_map(&[ex_file("lib/factory.ex", src)], std::path::Path::new(""));
    let set = state.injections_for("MyApp.Factory").unwrap();
    assert!(set.contains(&ElixirInjection::Use {
        module: "MyApp.Machina.Ecto".to_string()
    }));
}

#[test]
fn flattened_injections_for_resolves_transitive_use_hop() {
    // Factory `use`s Ecto (plain, no using-block of its own); Ecto's
    // `__using__` quote does `use Machina` AND defines `params_for` literally;
    // Machina's `__using__` quote defines `build` literally. A file that
    // `use`s/`import`s Factory should see `build` and `params_for` — two
    // hops away — without ever mentioning Ecto or Machina.
    let src = r#"
defmodule MyApp.Machina do
  defmacro __using__(_opts) do
    quote do
      def build(name, attrs) do
        attrs
      end
    end
  end
end

defmodule MyApp.Machina.Ecto do
  defmacro __using__(_opts) do
    quote do
      use MyApp.Machina

      def params_for(name, attrs) do
        attrs
      end
    end
  end
end

defmodule MyApp.Factory do
  use MyApp.Machina.Ecto, repo: MyApp.Repo
end
"#;
    let state = build_using_injection_map(&[ex_file("lib/all.ex", src)], std::path::Path::new(""));
    let flat = state.flattened_injections_for("MyApp.Factory");
    assert!(flat.contains(&&ElixirInjection::Def {
        name: "build".to_string(),
        is_macro: false
    }));
    assert!(flat.contains(&&ElixirInjection::Def {
        name: "params_for".to_string(),
        is_macro: false
    }));
    // `Use` hops are consumed during flattening, never surfaced directly.
    assert!(!flat.iter().any(|inj| matches!(inj, ElixirInjection::Use { .. })));
}

#[test]
fn flattened_injections_for_stops_on_cycle() {
    let src = r#"
defmodule MyApp.A do
  defmacro __using__(_opts) do
    quote do
      use MyApp.B
      import MyApp.FromA
    end
  end
end

defmodule MyApp.B do
  defmacro __using__(_opts) do
    quote do
      use MyApp.A
      import MyApp.FromB
    end
  end
end
"#;
    let state = build_using_injection_map(&[ex_file("lib/cycle.ex", src)], std::path::Path::new(""));
    let flat = state.flattened_injections_for("MyApp.A");
    assert!(flat.contains(&&ElixirInjection::Import {
        module: "MyApp.FromA".to_string()
    }));
    assert!(flat.contains(&&ElixirInjection::Import {
        module: "MyApp.FromB".to_string()
    }));
}

#[test]
fn case_template_proxy_delegate_is_followed() {
    // ExUnit.CaseTemplate's shape: the outer `__using__` quote imports
    // ExUnit.Assertions directly AND nests a NEW `defmacro __using__` that
    // delegates to a same-module helper (`__proxy__`); the helper's own
    // `quote do ... end` is where `use ExUnit.Case` actually lives.
    let src = r#"
defmodule MyApp.CaseTemplate do
  defmacro __using__(_opts) do
    quote do
      import MyApp.Assertions

      defmacro __using__(opts) do
        unquote(__MODULE__).__proxy__(__MODULE__, opts)
      end
    end
  end

  def __proxy__(module, opts) do
    quote do
      use MyApp.Case, MyApp.Case.__keys__(unquote(opts))
    end
  end
end
"#;
    let state = build_using_injection_map(&[ex_file("lib/case_template.ex", src)], std::path::Path::new(""));
    let set = state.injections_for("MyApp.CaseTemplate").unwrap();
    assert!(set.contains(&ElixirInjection::Import {
        module: "MyApp.Assertions".to_string()
    }));
    assert!(set.contains(&ElixirInjection::Use {
        module: "MyApp.Case".to_string()
    }));
    // The proxy `__using__` redefinition itself is not a real member.
    assert!(!set.contains(&ElixirInjection::Def {
        name: "__using__".to_string(),
        is_macro: true
    }));
}

#[test]
fn case_template_cascade_reaches_test_macro_transitively() {
    // Full plausible-shaped cascade: DataCase plainly `use`s CaseTemplate (no
    // using-block relationship between them) and separately injects its own
    // `using do` surface. A module that `use`s DataCase must transitively
    // see MyApp.Case's own injected `test` surface, reached only through
    // CaseTemplate's proxy delegate.
    let src = r#"
defmodule MyApp.CaseTemplate do
  defmacro __using__(_opts) do
    quote do
      import MyApp.Assertions

      defmacro __using__(opts) do
        unquote(__MODULE__).__proxy__(__MODULE__, opts)
      end
    end
  end

  def __proxy__(module, opts) do
    quote do
      use MyApp.Case, MyApp.Case.__keys__(unquote(opts))
    end
  end
end

defmodule MyApp.Case do
  defmacro __using__(_opts) do
    quote do
      import MyApp.Case, only: [test: 1, test: 3]
    end
  end
end

defmodule MyApp.DataCase do
  use MyApp.CaseTemplate

  using do
    quote do
      import MyApp.Factory
    end
  end
end
"#;
    let state = build_using_injection_map(&[ex_file("lib/all.ex", src)], std::path::Path::new(""));
    let flat = state.flattened_injections_for("MyApp.DataCase");
    assert!(flat.contains(&&ElixirInjection::Import {
        module: "MyApp.Assertions".to_string()
    }));
    assert!(flat.contains(&&ElixirInjection::Import {
        module: "MyApp.Case".to_string()
    }));
    assert!(flat.contains(&&ElixirInjection::Import {
        module: "MyApp.Factory".to_string()
    }));
}
