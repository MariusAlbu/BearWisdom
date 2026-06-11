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
    let state = build_using_injection_map(&[ex_file("lib/data_case.ex", src)]);
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
    let state = build_using_injection_map(&[ex_file("lib/conn_case.ex", src)]);
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
    let state = build_using_injection_map(&[ex_file("lib/web_case.ex", src)]);
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
    let state = build_using_injection_map(&[ex_file("lib/case.ex", src)]);
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
fn collect_use_sites_picks_use_not_alias_or_import() {
    let src = r#"
defmodule MyApp.SomeTest do
  use MyApp.DataCase
  alias MyApp.Other
  import MyApp.Helper
end
"#;
    let sites = collect_use_sites(src);
    assert!(sites.contains(&"MyApp.DataCase".to_string()));
    assert!(!sites.contains(&"MyApp.Other".to_string()));
    assert!(!sites.contains(&"MyApp.Helper".to_string()));
}

#[test]
fn expand_use_is_transitive_across_nested_use() {
    // `use DataCase` → DataCase's quote `use TestUtils` → TestUtils's quote
    // `import TestUtils`. The terminal import must reach the expanded set.
    let data_case = r#"
defmodule MyApp.DataCase do
  defmacro __using__(_) do
    quote do
      use MyApp.TestUtils
    end
  end
end
"#;
    let test_utils = r#"
defmodule MyApp.TestUtils do
  defmacro __using__(_) do
    quote do
      import MyApp.TestUtils
    end
  end
end
"#;
    let state = build_using_injection_map(&[
        ex_file("lib/data_case.ex", data_case),
        ex_file("lib/test_utils.ex", test_utils),
    ]);

    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    expand_use("MyApp.DataCase", &state, &mut out, &mut seen);

    assert!(
        out.iter().any(|e| e.module_path.as_deref() == Some("MyApp.TestUtils")
            && e.imported_name == "TestUtils"),
        "transitive import of TestUtils missing: {out:?}"
    );
}

#[test]
fn expand_use_guards_against_cycles() {
    // A ↔ B mutual `use` must terminate.
    let a = r#"
defmodule A do
  defmacro __using__(_) do
    quote do
      use B
    end
  end
end
"#;
    let b = r#"
defmodule B do
  defmacro __using__(_) do
    quote do
      use A
    end
  end
end
"#;
    let state = build_using_injection_map(&[ex_file("a.ex", a), ex_file("b.ex", b)]);
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    // Terminates (no stack overflow / infinite loop) and binds nothing.
    expand_use("A", &state, &mut out, &mut seen);
    assert!(out.is_empty());
}

#[test]
fn alias_injection_expands_to_module_qname_import_entry() {
    let mut map = std::collections::HashMap::new();
    map.insert(
        "MyApp.Repo".to_string(),
        vec![ElixirInjection::Alias {
            local: "Repo".to_string(),
            module: "MyApp.Repo".to_string(),
        }],
    );
    let state = ElixirProjectState::from_map(map);

    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    expand_use("MyApp.Repo", &state, &mut out, &mut seen);

    assert_eq!(out.len(), 1);
    assert_eq!(out[0].imported_name, "Repo");
    assert_eq!(out[0].module_path.as_deref(), Some("MyApp.Repo"));
    // local == last segment, so no `as:` rename.
    assert_eq!(out[0].alias, None);
}

#[test]
fn no_using_block_yields_no_injection_entry() {
    let src = r#"
defmodule MyApp.Plain do
  def hello, do: :ok
end
"#;
    let state = build_using_injection_map(&[ex_file("lib/plain.ex", src)]);
    assert!(state.injections_for("MyApp.Plain").is_none());
}
