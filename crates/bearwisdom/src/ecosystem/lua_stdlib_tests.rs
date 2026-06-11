use super::*;
use std::fs;

// ---------------------------------------------------------------------------
// luaL_Reg table parsing
// ---------------------------------------------------------------------------

const STRLIB_SRC: &str = r#"
static const luaL_Reg strlib[] = {
  {"byte", str_byte},
  {"find", str_find},
  {"format", str_format},
  {"gsub", str_gsub},
  {"len", str_len},
  {"sub", str_sub},
  {NULL, NULL}
};

LUAMOD_API int luaopen_string (lua_State *L) {
  luaL_newlib(L, strlib);
  createmetatable(L);
  return 1;
}
"#;

#[test]
fn parses_strlib_table_into_string_module() {
    let modules = parse_lua_lib_source(STRLIB_SRC);
    assert_eq!(modules.len(), 1, "one module expected, got {modules:?}");
    let (module, names) = &modules[0];
    assert_eq!(module, "string");
    assert!(names.contains(&"gsub".to_string()), "{names:?}");
    assert!(names.contains(&"find".to_string()), "{names:?}");
    assert!(names.contains(&"sub".to_string()), "{names:?}");
    // The {NULL, NULL} sentinel must not become a symbol.
    assert!(
        !names.iter().any(|n| n == "NULL"),
        "sentinel leaked into names: {names:?}"
    );
    assert_eq!(names.len(), 6, "exactly the 6 named entries, got {names:?}");
}

const BASELIB_SRC: &str = r#"
static const luaL_Reg base_funcs[] = {
  {"assert", luaB_assert},
  {"pairs", luaB_pairs},
  {"print", luaB_print},
  {"type", luaB_type},
  {LUA_GNAME, NULL},
  {"_VERSION", NULL},
  {NULL, NULL}
};

LUAMOD_API int luaopen_base (lua_State *L) {
  lua_pushglobaltable(L);
  luaL_setfuncs(L, base_funcs, 0);
  return 1;
}
"#;

#[test]
fn base_functions_register_via_setfuncs_and_get_global_qnames() {
    let modules = parse_lua_lib_source(BASELIB_SRC);
    assert_eq!(modules.len(), 1);
    let (module, names) = &modules[0];
    assert_eq!(module, "base");
    assert!(names.contains(&"print".to_string()), "{names:?}");
    assert!(names.contains(&"assert".to_string()), "{names:?}");
    // `_VERSION` has a NULL function value — placeholder, not a callable.
    assert!(
        !names.contains(&"_VERSION".to_string()),
        "NULL-valued placeholder must be dropped: {names:?}"
    );
    // `LUA_GNAME` is not a string literal — must not be captured.
    assert!(
        !names.iter().any(|n| n.contains("GNAME")),
        "non-literal entry leaked: {names:?}"
    );
}

#[test]
fn synthesized_base_symbols_are_global_others_are_qualified() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("lbaselib.c"), BASELIB_SRC).unwrap();
    fs::write(tmp.path().join("lstrlib.c"), STRLIB_SRC).unwrap();

    let files = synthesize_from_lua_source(tmp.path());
    assert_eq!(files.len(), 1, "one synthetic ParsedFile expected");
    let pf = &files[0];
    assert_eq!(pf.language, "lua");
    assert!(pf.path.starts_with("ext:lua-stdlib:"));

    // base.print → bare global qname `print`.
    let print = pf
        .symbols
        .iter()
        .find(|s| s.name == "print")
        .expect("print symbol");
    assert_eq!(print.qualified_name, "print");

    // string.gsub → `string.gsub` qname.
    let gsub = pf
        .symbols
        .iter()
        .find(|s| s.name == "gsub")
        .expect("gsub symbol");
    assert_eq!(gsub.qualified_name, "string.gsub");

    // ParsedFile parallel-array invariant.
    assert_eq!(pf.symbol_origin_languages.len(), pf.symbols.len());
    assert_eq!(pf.symbol_from_snippet.len(), pf.symbols.len());
}

#[test]
fn module_without_matching_table_yields_nothing() {
    // luaopen names a table that has no luaL_Reg definition in this file.
    let src = r#"
LUAMOD_API int luaopen_ghost (lua_State *L) {
  luaL_newlib(L, ghost_funcs);
  return 1;
}
"#;
    let modules = parse_lua_lib_source(src);
    assert!(modules.is_empty(), "no table → no module: {modules:?}");
}

#[test]
fn forward_declaration_is_not_treated_as_definition() {
    // A prototype (ends in `;`) must not register a module table.
    let src = r#"
LUAMOD_API int luaopen_string (lua_State *L);

static const luaL_Reg strlib[] = {
  {"sub", str_sub},
  {NULL, NULL}
};

LUAMOD_API int luaopen_string (lua_State *L) {
  luaL_newlib(L, strlib);
  return 1;
}
"#;
    let modules = parse_lua_lib_source(src);
    assert_eq!(modules.len(), 1, "exactly the definition, got {modules:?}");
    assert_eq!(modules[0].0, "string");
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

#[test]
fn discover_uses_env_override_when_lstrlib_present() {
    let tmp = tempfile::tempdir().unwrap();
    fs::write(tmp.path().join("lstrlib.c"), STRLIB_SRC).unwrap();

    std::env::set_var("BEARWISDOM_LUA_SRC", tmp.path());
    let roots = discover_lua_source();
    std::env::remove_var("BEARWISDOM_LUA_SRC");

    assert!(!roots.is_empty(), "override with lstrlib.c must produce a root");
    assert_eq!(roots[0].module_path, TAG);
    assert_eq!(roots[0].root, tmp.path());
}

#[test]
fn discover_rejects_override_without_lstrlib() {
    let tmp = tempfile::tempdir().unwrap();
    // Dir exists but is not a Lua source tree.
    std::env::set_var("BEARWISDOM_LUA_SRC", tmp.path());
    let roots = discover_lua_source();
    std::env::remove_var("BEARWISDOM_LUA_SRC");

    assert!(
        roots.iter().all(|r| r.root != tmp.path()),
        "a directory without lstrlib.c must not qualify as a Lua source tree"
    );
}

#[test]
fn synthesize_returns_empty_for_dir_without_lib_sources() {
    let tmp = tempfile::tempdir().unwrap();
    let files = synthesize_from_lua_source(tmp.path());
    assert!(files.is_empty(), "no lib C sources → no symbols");
}

#[test]
fn ecosystem_identity_and_flags() {
    let e = LuaStdlibEcosystem;
    assert_eq!(e.id(), ID);
    assert_eq!(Ecosystem::kind(&e), EcosystemKind::Stdlib);
    assert_eq!(Ecosystem::languages(&e), &["lua"]);
    assert!(e.uses_demand_driven_parse());
    assert!(e.supports_reachability());
    assert!(matches!(
        e.activation(),
        EcosystemActivation::LanguagePresent("lua")
    ));
}
