// =============================================================================
// ecosystem/lua_stdlib.rs — Lua standard library (stdlib ecosystem)
//
// Lua's standard library is implemented in C, not Lua. There is no `.lua`
// source to feed the Lua extractor — the only on-disk artifact describing
// the stdlib surface is the Lua interpreter's own C source tree, where each
// module registers its functions in a `luaL_Reg` table:
//
//     static const luaL_Reg strlib[] = {
//       {"gsub", str_gsub},
//       {"find", str_find},
//       ...
//       {NULL, NULL}
//     };
//
//     LUAMOD_API int luaopen_string (lua_State *L) {
//       luaL_newlib(L, strlib);
//       ...
//     }
//
// The `luaopen_<module>` function names the module (`string`); its body
// registers a `luaL_Reg` table via `luaL_newlib(L, <table>)` or
// `luaL_setfuncs(L, <table>, n)`. Parsing the two together yields, per module,
// the set of exported function names — discovered from the real source, never
// hand-listed.
//
// Discovery: a Lua source checkout (lstrlib.c, lbaselib.c, …). Probed via
// `$BEARWISDOM_LUA_SRC`, then `~/lua-source`. When absent, the walker emits
// nothing and warns, consistent with the trait's degrade-honestly contract.
//
// Activation: `LanguagePresent("lua")` — every Lua project uses these names
// (string, table, math, os, io, …) unqualified-by-import as language substrate.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::{debug, warn};

use super::{Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("lua-stdlib");
const TAG: &str = "lua-stdlib";
const LANGUAGES: &[&str] = &["lua"];

/// The `base` module registers into the global table, so its functions are
/// global names (`print`, `pairs`, `assert`) with no module prefix. Every
/// other module's functions are qualified `<module>.<name>`.
const BASE_MODULE: &str = "base";

pub struct LuaStdlibEcosystem;

impl Ecosystem for LuaStdlibEcosystem {
    fn id(&self) -> EcosystemId {
        ID
    }
    fn kind(&self) -> EcosystemKind {
        EcosystemKind::Stdlib
    }
    fn languages(&self) -> &'static [&'static str] {
        LANGUAGES
    }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("lua")
    }

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        discover_lua_source()
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        // The C source is not walked through a language extractor; symbols are
        // synthesized from the luaL_Reg tables via `parse_metadata_only`.
        Vec::new()
    }

    fn parse_metadata_only(&self, dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        Some(synthesize_from_lua_source(&dep.root))
    }

    fn supports_reachability(&self) -> bool {
        true
    }
    fn uses_demand_driven_parse(&self) -> bool {
        true
    }
}

impl ExternalSourceLocator for LuaStdlibEcosystem {
    fn ecosystem(&self) -> &'static str {
        TAG
    }

    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        discover_lua_source()
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        let roots = discover_lua_source();
        let mut out = Vec::new();
        for root in roots {
            out.extend(synthesize_from_lua_source(&root.root));
        }
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }
}

pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<LuaStdlibEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(LuaStdlibEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Probe for a Lua source checkout in priority order:
///   1. `$BEARWISDOM_LUA_SRC` — explicit override.
///   2. `~/lua-source` — the conventional clone location.
///
/// A directory qualifies only when it contains `lstrlib.c` (the string
/// library source), the load-bearing marker that this is a Lua interpreter
/// source tree rather than an arbitrary directory.
pub(super) fn discover_lua_source() -> Vec<ExternalDepRoot> {
    for candidate in candidate_source_roots() {
        if candidate.join("lstrlib.c").is_file() {
            debug!("lua-stdlib: using Lua source at {}", candidate.display());
            return vec![ExternalDepRoot {
                module_path: TAG.to_string(),
                version: String::new(),
                root: candidate,
                ecosystem: TAG,
                package_id: None,
                requested_imports: Vec::new(),
            }];
        }
    }
    warn!(
        "lua-stdlib: no Lua source checkout found \
         (set BEARWISDOM_LUA_SRC or clone https://github.com/lua/lua to ~/lua-source)"
    );
    Vec::new()
}

fn candidate_source_roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(explicit) = std::env::var_os("BEARWISDOM_LUA_SRC") {
        out.push(PathBuf::from(explicit));
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        out.push(PathBuf::from(home).join("lua-source"));
    }
    out
}

// ---------------------------------------------------------------------------
// Symbol synthesis from luaL_Reg tables
// ---------------------------------------------------------------------------

/// The C source files that register the standard-library modules. Each holds
/// one or more `luaL_Reg` tables and a `luaopen_<module>` entry point. This is
/// the file set, not a symbol list — the symbols themselves come from parsing
/// the tables inside.
const LIB_SOURCE_FILES: &[&str] = &[
    "lbaselib.c",
    "lstrlib.c",
    "ltablib.c",
    "lmathlib.c",
    "loslib.c",
    "liolib.c",
    "lcorolib.c",
    "lutf8lib.c",
    "ldblib.c",
];

/// Parse every standard-library C source file under `src_root`, mapping each
/// `luaopen_<module>` to its registered `luaL_Reg` table and emitting one
/// `Function` symbol per table entry. Returns a single synthetic `ParsedFile`
/// holding all discovered symbols (one ParsedFile, like the R stdlib path).
pub(super) fn synthesize_from_lua_source(src_root: &Path) -> Vec<ParsedFile> {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();

    for fname in LIB_SOURCE_FILES {
        let path = src_root.join(fname);
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (module, names) in parse_lua_lib_source(&content) {
            for name in names {
                symbols.push(make_sym(&name, &module));
            }
        }
    }

    if symbols.is_empty() {
        debug!(
            "lua-stdlib: no luaL_Reg symbols discovered under {}",
            src_root.display()
        );
        return Vec::new();
    }

    let n = symbols.len();
    vec![ParsedFile {
        path: "ext:lua-stdlib:lua_stdlib_generated.lua".to_string(),
        language: "lua".to_string(),
        content_hash: format!("lua-stdlib-{n}"),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: vec![None; n],
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: vec![false; n],
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
        declared_modules: Vec::new(),
    }]
}

/// Parse one Lua library C source file. Returns `(module, function_names)`
/// pairs.
///
/// Two passes over the source:
///   1. Collect every `static const luaL_Reg <table>[] = { ... }` table and its
///      `{"name", fn}` entries (skipping `{NULL, …}` sentinels and entries
///      whose function pointer is `NULL`, e.g. base's `_VERSION` placeholder).
///   2. For each `luaopen_<module>` function, scan its body for the table it
///      registers via `luaL_newlib(L, <table>)` or `luaL_setfuncs(L, <table>,…)`
///      and attach that table's names to `<module>`.
///
/// `base`'s functions register into the global table, so they are emitted with
/// no module prefix; all other modules emit `<module>.<name>` qnames.
pub(super) fn parse_lua_lib_source(content: &str) -> Vec<(String, Vec<String>)> {
    let tables = collect_reg_tables(content);
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for (module, table) in collect_module_tables(content) {
        if let Some(names) = tables.get(&table) {
            if !names.is_empty() {
                out.push((module, names.clone()));
            }
        }
    }
    out
}

/// Collect every `luaL_Reg` table → its list of `{"name", ...}` keys.
fn collect_reg_tables(content: &str) -> std::collections::HashMap<String, Vec<String>> {
    let mut tables = std::collections::HashMap::new();
    let bytes = content.as_bytes();
    let needle = "luaL_Reg ";
    let mut search_from = 0usize;
    while let Some(rel) = content[search_from..].find(needle) {
        let decl_start = search_from + rel + needle.len();
        // Table name: identifier chars up to `[`.
        let name: String = content[decl_start..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        search_from = decl_start + name.len();
        if name.is_empty() {
            continue;
        }
        // Find the opening `{` of the initializer after the `[]`.
        let Some(open_rel) = content[search_from..].find('{') else {
            break;
        };
        let body_start = search_from + open_rel + 1;
        // The initializer runs to the matching `}`. Entry bodies use nested
        // braces (`{"name", fn}`), so track brace depth.
        let mut depth = 1i32;
        let mut idx = body_start;
        while idx < bytes.len() && depth > 0 {
            match bytes[idx] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            idx += 1;
        }
        let body = &content[body_start..idx.min(bytes.len())];
        tables.insert(name, parse_reg_entries(body));
        search_from = idx;
    }
    tables
}

/// Extract the `"name"` keys from a `luaL_Reg` initializer body. Each entry is
/// `{"name", function}`; the sentinel `{NULL, NULL}` and placeholder entries
/// whose value is `NULL` (e.g. `{"_VERSION", NULL}`) are dropped.
fn parse_reg_entries(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            // Parse one `{ ... }` entry.
            let Some(close_rel) = body[i..].find('}') else {
                break;
            };
            let entry = &body[i + 1..i + close_rel];
            if let Some(name) = parse_reg_entry(entry) {
                out.push(name);
            }
            i += close_rel + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// Parse a single `"name", function` entry. Returns the name only when it is a
/// string literal AND the function value is not `NULL`.
fn parse_reg_entry(entry: &str) -> Option<String> {
    let first = entry.find('"')?;
    let rest = &entry[first + 1..];
    let end = rest.find('"')?;
    let name = &rest[..end];
    if name.is_empty() {
        return None;
    }
    // The value follows the closing quote and a comma.
    let after = &rest[end + 1..];
    let value = after.trim_start_matches([',', ' ', '\t', '\n', '\r']);
    if value.starts_with("NULL") {
        return None;
    }
    Some(name.to_string())
}

/// Map each `luaopen_<module>` to the `luaL_Reg` table it registers. Scans the
/// function body (to its closing brace) for the first
/// `luaL_newlib(L, <table>)` / `luaL_setfuncs(L, <table>, n)` reference.
fn collect_module_tables(content: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let needle = "luaopen_";
    let bytes = content.as_bytes();
    let mut search_from = 0usize;
    while let Some(rel) = content[search_from..].find(needle) {
        let mod_start = search_from + rel + needle.len();
        let module: String = content[mod_start..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        search_from = mod_start + module.len();
        if module.is_empty() {
            continue;
        }
        // Only treat this as a definition (not a forward declaration) when the
        // body brace follows on the same construct: find the next `{` and walk
        // to its match.
        let Some(open_rel) = content[search_from..].find('{') else {
            continue;
        };
        // A `;` before the `{` means this was a prototype, not the definition.
        if content[search_from..search_from + open_rel].contains(';') {
            continue;
        }
        let body_start = search_from + open_rel + 1;
        let mut depth = 1i32;
        let mut idx = body_start;
        while idx < bytes.len() && depth > 0 {
            match bytes[idx] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            idx += 1;
        }
        let body = &content[body_start..idx.min(bytes.len())];
        if let Some(table) = find_registered_table(body) {
            out.push((module, table));
        }
        search_from = idx;
    }
    out
}

/// Find the `luaL_Reg` table registered inside a `luaopen_*` body via the first
/// `luaL_newlib(L, <table>)` or `luaL_setfuncs(L, <table>, n)` call.
fn find_registered_table(body: &str) -> Option<String> {
    for call in ["luaL_newlib(", "luaL_setfuncs("] {
        if let Some(rel) = body.find(call) {
            let args = &body[rel + call.len()..];
            // First arg is the lua_State (`L`); the table is the second arg.
            let mut parts = args.splitn(3, ',');
            let _state = parts.next();
            if let Some(table_arg) = parts.next() {
                let table: String = table_arg
                    .trim()
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !table.is_empty() {
                    return Some(table);
                }
            }
        }
    }
    None
}

fn make_sym(name: &str, module: &str) -> ExtractedSymbol {
    // base registers into the global table → bare global names. Other modules
    // expose `<module>.<name>`.
    let qualified_name = if module == BASE_MODULE {
        name.to_string()
    } else {
        format!("{module}.{name}")
    };
    ExtractedSymbol {
        name: name.to_string(),
        qualified_name,
        kind: SymbolKind::Function,
        visibility: Some(Visibility::Public),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: Some(format!("-- lua stdlib {module} library")),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    }
}

#[cfg(test)]
#[path = "lua_stdlib_tests.rs"]
mod tests;
