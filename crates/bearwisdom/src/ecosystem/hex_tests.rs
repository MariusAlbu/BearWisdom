use super::*;

#[test]
fn ecosystem_identity() {
    let h = HexEcosystem;
    assert_eq!(h.id(), ID);
    assert_eq!(Ecosystem::kind(&h), EcosystemKind::Package);
    assert_eq!(Ecosystem::languages(&h), &["elixir", "erlang", "gleam"]);
}

#[test]
fn legacy_locator_tag_is_hex() {
    assert_eq!(ExternalSourceLocator::ecosystem(&HexEcosystem), "hex");
}

#[test]
fn detect_hex_language_covers_extensions() {
    assert_eq!(detect_hex_language("foo.ex"), Some(("elixir", "elixir")));
    assert_eq!(detect_hex_language("foo.exs"), Some(("elixir", "elixir")));
    assert_eq!(detect_hex_language("bar.erl"), Some(("erlang", "erlang")));
    assert_eq!(detect_hex_language("bar.hrl"), Some(("erlang", "erlang")));
    assert_eq!(detect_hex_language("baz.gleam"), Some(("gleam", "gleam")));
    assert_eq!(detect_hex_language("readme.md"), None);
}

// --- Elixir/mix tests ---

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn make_elixir_fixture(tmp: &Path, deps: &[&str]) {
    std::fs::create_dir_all(tmp).unwrap();
    let mut mix = String::from(
        "defmodule MyApp.MixProject do\n  use Mix.Project\n  defp deps do\n    [\n",
    );
    for name in deps {
        mix.push_str(&format!("      {{:{name}, \"~> 1.0\"}},\n"));
    }
    mix.push_str("    ]\n  end\nend\n");
    std::fs::write(tmp.join("mix.exs"), mix).unwrap();

    for name in deps {
        let pkg = tmp.join("deps").join(name);
        let lib = pkg.join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(
            lib.join(format!("{name}.ex")),
            format!(
                "defmodule {} do\n  def hello, do: :world\nend\n",
                capitalize(name)
            ),
        )
        .unwrap();
        std::fs::write(
            pkg.join("mix.exs"),
            format!(
                "defmodule {}.MixProject do\n  @version \"1.2.3\"\nend\n",
                capitalize(name)
            ),
        )
        .unwrap();
        std::fs::create_dir_all(pkg.join("test")).unwrap();
        std::fs::write(pkg.join("test").join("should_skip.exs"), "# test\n").unwrap();
        std::fs::create_dir_all(pkg.join("priv")).unwrap();
        std::fs::write(pkg.join("priv").join("seeds.exs"), "# priv\n").unwrap();
    }
}

#[test]
fn mix_locator_finds_deps_directories() {
    let tmp = std::env::temp_dir().join("bw-test-hex-mix-find");
    let _ = std::fs::remove_dir_all(&tmp);
    make_elixir_fixture(&tmp, &["phoenix", "ecto", "plug"]);

    let roots = discover_mix_roots(&tmp, &[]);
    assert_eq!(roots.len(), 3);
    let names: std::collections::HashSet<String> =
        roots.iter().map(|r| r.module_path.clone()).collect();
    assert!(names.contains("phoenix"));
    assert!(names.contains("ecto"));
    assert!(names.contains("plug"));
    assert!(roots.iter().all(|r| r.version == "1.2.3"));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn mix_walk_excludes_test_priv_and_config() {
    let tmp = std::env::temp_dir().join("bw-test-hex-mix-walk");
    let _ = std::fs::remove_dir_all(&tmp);
    make_elixir_fixture(&tmp, &["phoenix"]);

    let roots = discover_mix_roots(&tmp, &[]);
    assert_eq!(roots.len(), 1);
    let walked = walk_hex_root(&roots[0]);
    assert_eq!(walked.len(), 1);
    let file = &walked[0];
    assert!(file.relative_path.starts_with("ext:elixir:phoenix/"));
    assert!(file.relative_path.ends_with("lib/phoenix.ex"));
    assert_eq!(file.language, "elixir");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn mix_returns_empty_without_mix_exs() {
    let tmp = std::env::temp_dir().join("bw-test-hex-mix-no-manifest");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let roots = discover_mix_roots(&tmp, &[]);
    assert!(roots.is_empty());
    let _ = std::fs::remove_dir_all(&tmp);
}

// --- rebar (Erlang) tests ---

#[test]
fn erlang_parses_rebar_deps_git() {
    let content = r#"{deps, [
{cowlib,".*",{git,"https://github.com/ninenines/cowlib",{tag,"2.16.0"}}},
{ranch,".*",{git,"https://github.com/ninenines/ranch",{tag,"1.8.1"}}}
]}."#;
    let deps = parse_rebar_deps(content);
    assert_eq!(deps, vec!["cowlib", "ranch"]);
}

#[test]
fn erlang_parses_rebar_deps_hex_shorthand() {
    let content = r#"{deps, [{cowlib, "~> 2.12"}, {ranch, "~> 1.8"}]}."#;
    let deps = parse_rebar_deps(content);
    assert_eq!(deps, vec!["cowlib", "ranch"]);
}

#[test]
fn erlang_parses_rebar_lock_versions() {
    let tmp = std::env::temp_dir().join("bw-test-hex-rebar-lock");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(
        tmp.join("rebar.lock"),
        r#"{"1.2.0",
[{<<"cowlib">>,{pkg,<<"cowlib">>,<<"2.16.0">>,<<"HASH1">>,<<"HASH2">>},0},
 {<<"ranch">>,{pkg,<<"ranch">>,<<"1.8.1">>,<<"HASH3">>,<<"HASH4">>},0}]}."#,
    )
    .unwrap();
    let versions = parse_rebar_lock(&tmp);
    assert_eq!(versions.get("cowlib").map(String::as_str), Some("2.16.0"));
    assert_eq!(versions.get("ranch").map(String::as_str), Some("1.8.1"));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn erlang_discovers_build_deps() {
    let tmp = std::env::temp_dir().join("bw-test-hex-rebar-build");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(
        tmp.join("rebar.config"),
        r#"{deps, [{cowlib,".*",{git,"url",{tag,"1.0"}}},{ranch,".*",{git,"url",{tag,"1.0"}}}]}."#,
    )
    .unwrap();
    let deps_dir = tmp.join("_build").join("default").join("lib");
    let cowlib_src = deps_dir.join("cowlib").join("src");
    std::fs::create_dir_all(&cowlib_src).unwrap();
    std::fs::write(cowlib_src.join("cowlib.erl"), "-module(cowlib).\n").unwrap();

    let empty_hex = tmp.join("empty-hex");
    std::fs::create_dir_all(&empty_hex).unwrap();
    std::env::set_var("BEARWISDOM_HEX_PACKAGES", &empty_hex);

    let roots = discover_rebar_roots(&tmp, &[]);
    std::env::remove_var("BEARWISDOM_HEX_PACKAGES");

    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].module_path, "cowlib");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn walk_skips_suite_and_test_files() {
    let tmp = std::env::temp_dir().join("bw-test-hex-walk-skip");
    let _ = std::fs::remove_dir_all(&tmp);
    let pkg_root = tmp.join("cowlib");
    let src = pkg_root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("cowlib.erl"), "-module(cowlib).\n").unwrap();
    std::fs::write(src.join("cowlib_SUITE.erl"), "% test suite\n").unwrap();
    std::fs::write(src.join("cowlib_tests.erl"), "% unit tests\n").unwrap();

    let dep = ExternalDepRoot {
        module_path: "cowlib".into(),
        version: "2.16.0".into(),
        root: pkg_root,
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let walked = walk_hex_root(&dep);
    assert_eq!(walked.len(), 1);
    assert!(walked[0].relative_path.ends_with("src/cowlib.erl"));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[allow(dead_code)]
fn _ensure_shared_locator_typed() -> Arc<dyn ExternalSourceLocator> {
    shared_locator()
}

// --- erlang.mk tests ---

#[test]
fn erlang_mk_discovers_deps_dir_subdirs() {
    let tmp = std::env::temp_dir().join("bw-test-hex-erlangmk-find");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("erlang.mk"), "# erlang.mk stub\n").unwrap();
    std::fs::write(tmp.join("Makefile"), "DEPS = cowboy ranch\n").unwrap();

    let cowboy_src = tmp.join("deps").join("cowboy").join("src");
    std::fs::create_dir_all(&cowboy_src).unwrap();
    std::fs::write(cowboy_src.join("cowboy.erl"), "-module(cowboy).\n").unwrap();
    let ranch_src = tmp.join("deps").join("ranch").join("src");
    std::fs::create_dir_all(&ranch_src).unwrap();
    std::fs::write(ranch_src.join("ranch.erl"), "-module(ranch).\n").unwrap();

    let roots = discover_erlang_mk_roots(&tmp, &[]);
    let names: std::collections::HashSet<String> =
        roots.iter().map(|r| r.module_path.clone()).collect();
    assert!(names.contains("cowboy"));
    assert!(names.contains("ranch"));
    assert_eq!(roots.len(), 2);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn erlang_mk_returns_empty_without_marker_file() {
    let tmp = std::env::temp_dir().join("bw-test-hex-erlangmk-no-marker");
    let _ = std::fs::remove_dir_all(&tmp);
    let cowboy = tmp.join("deps").join("cowboy");
    std::fs::create_dir_all(&cowboy).unwrap();
    // No erlang.mk file → don't activate (this could be an Elixir mix project).
    let roots = discover_erlang_mk_roots(&tmp, &[]);
    assert!(roots.is_empty());
    let _ = std::fs::remove_dir_all(&tmp);
}

// -----------------------------------------------------------------
// R3 — module-ref scan + narrowed walk
// -----------------------------------------------------------------

#[test]
fn elixir_alias_extracts_module() {
    let mut out = std::collections::HashSet::new();
    extract_elixir_module_refs(
        "alias Phoenix.Endpoint\nalias Ecto.{Schema, Changeset}\nimport Plug.Conn\nuse Phoenix.Controller\n",
        &mut out,
    );
    assert!(out.contains("Phoenix.Endpoint"));
    assert!(out.contains("Ecto.Schema"));
    assert!(out.contains("Ecto.Changeset"));
    assert!(out.contains("Plug.Conn"));
    assert!(out.contains("Phoenix.Controller"));
}

#[test]
fn erlang_call_extracts_module_name() {
    let mut out = std::collections::HashSet::new();
    extract_erlang_module_refs(
        "-include(\"my_header.hrl\").\n-include_lib(\"cowboy/include/cowboy.hrl\").\nfoo() -> lists:reverse(io_lib:format(\"hi\", [])).\n",
        &mut out,
    );
    assert!(out.contains("my_header.hrl"));
    assert!(out.contains("cowboy.hrl"));
    assert!(out.contains("lists.erl"));
    assert!(out.contains("io_lib.erl"));
}

#[test]
fn gleam_import_extracts_module() {
    let mut out = std::collections::HashSet::new();
    extract_gleam_module_refs("import gleam/list\nimport gleam/string.{contains}\n", &mut out);
    assert!(out.contains("gleam:gleam/list"));
    assert!(out.contains("gleam:gleam/string"));
}

#[test]
fn elixir_module_to_snake_path() {
    let suffixes = requested_to_path_suffixes(&["Phoenix.Endpoint".to_string()]);
    assert!(suffixes.contains("phoenix/endpoint.ex"));
}

#[test]
fn narrowed_walk_includes_siblings() {
    let tmp = std::env::temp_dir().join("bw-test-hex-r3-narrow");
    let _ = std::fs::remove_dir_all(&tmp);
    let dep_root = tmp.join("phoenix");
    let lib = dep_root.join("lib").join("phoenix");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::create_dir_all(dep_root.join("lib").join("plug")).unwrap();
    std::fs::write(lib.join("endpoint.ex"), "defmodule Phoenix.Endpoint do end\n").unwrap();
    std::fs::write(lib.join("controller.ex"), "defmodule Phoenix.Controller do end\n").unwrap();
    std::fs::write(
        dep_root.join("lib").join("plug").join("conn.ex"),
        "defmodule Plug.Conn do end\n",
    ).unwrap();

    let dep = ExternalDepRoot {
        module_path: "phoenix".to_string(),
        version: "1.7".to_string(),
        root: dep_root.clone(),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: vec!["Phoenix.Endpoint".to_string()],
    };
    let files = walk_hex_narrowed(&dep);
    let paths: std::collections::HashSet<_> =
        files.iter().map(|f| f.absolute_path.clone()).collect();
    // Endpoint matched directly, Controller pulled in by sibling rule.
    assert!(paths.contains(&lib.join("endpoint.ex")));
    assert!(paths.contains(&lib.join("controller.ex")));
    // Unreferenced sub-package not walked.
    assert!(!paths.contains(&dep_root.join("lib/plug/conn.ex")));

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn narrowed_walk_falls_back_when_no_imports() {
    let tmp = std::env::temp_dir().join("bw-test-hex-r3-fallback");
    let _ = std::fs::remove_dir_all(&tmp);
    let lib = tmp.join("foo").join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::write(lib.join("a.ex"), "defmodule A do end\n").unwrap();

    let dep = ExternalDepRoot {
        module_path: "foo".to_string(),
        version: "1.0".to_string(),
        root: tmp.join("foo"),
        ecosystem: LEGACY_ECOSYSTEM_TAG,
        package_id: None,
        requested_imports: Vec::new(),
    };
    let files = walk_hex_narrowed(&dep);
    assert_eq!(files.len(), 1);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn elixir_header_scanner_captures_module_and_defs() {
    let src = r#"
defmodule Demo.Repo do
  def list_all, do: []
  defp secret, do: :ok
  defmacro trace(expr), do: expr
  defstruct [:id, :name]
end
"#;
    let names = scan_elixir_header(src);
    assert!(names.contains(&"Demo.Repo".to_string()) || names.contains(&"Demo".to_string()),
            "expected module name, got {names:?}");
    assert!(names.contains(&"list_all".to_string()), "{names:?}");
    assert!(names.contains(&"secret".to_string()), "{names:?}");
}

#[test]
fn gleam_header_scanner_captures_top_level_decls() {
    let src = "pub fn add(x: Int, y: Int) -> Int { x + y }\npub type Option { Some(Int) None }\npub const max = 10\n";
    let names = scan_gleam_header(src);
    assert!(names.contains(&"add".to_string()), "{names:?}");
}

#[test]
fn hex_build_symbol_index_empty_returns_empty() {
    assert!(build_hex_symbol_index(&[]).is_empty());
}
