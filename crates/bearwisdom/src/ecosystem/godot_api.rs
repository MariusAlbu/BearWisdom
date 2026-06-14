// =============================================================================
// ecosystem/godot_api.rs — Godot engine API (stdlib for GDScript)
//
// Godot's scripting surface is entirely runtime-injected: engine singletons
// (Input, OS, ClassDB, ...), built-in classes (Vector2, Array, Dictionary),
// core classes (Node, Object, Resource, ...), global enums, and utility
// functions. None of it lives in user project source.
//
// The engine ships a machine-readable description of its entire API as
// `extension_api.json` (produced by `godot --dump-extension-api` and
// shipped in the Godot source tree under `doc/classes/` alongside the
// per-class XML docs). This ecosystem parses it and synthesizes a
// `ParsedFile` per class with Class/Method/Property/Enum symbols so the
// resolver can turn `Input.is_action_pressed(...)` into a real edge
// instead of an opaque unresolved ref.
//
// Activation: any `.gd` file in the project. GDScript is exclusive to
// Godot, so language presence is a sound substrate signal. The strict
// project gate sits in locate_roots: a `project.godot` manifest must be
// present in the workspace tree, otherwise the API JSON probe is skipped
// (the project is a stray GDScript file, not a Godot project, so the
// extension API would be irrelevant noise).
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext, SymbolLocationIndex,
};
use crate::ecosystem::externals::{ExternalDepRoot, ExternalSourceLocator};
use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility};
use crate::walker::WalkedFile;

pub const ID: EcosystemId = EcosystemId::new("godot-api");
const LEGACY_ECOSYSTEM_TAG: &str = "godot-api";
const LANGUAGES: &[&str] = &["gdscript"];

pub struct GodotApiEcosystem;

impl Ecosystem for GodotApiEcosystem {
    fn id(&self) -> EcosystemId { ID }
    fn kind(&self) -> EcosystemKind { EcosystemKind::Stdlib }
    fn languages(&self) -> &'static [&'static str] { LANGUAGES }

    fn activation(&self) -> EcosystemActivation {
        EcosystemActivation::LanguagePresent("gdscript")
    }

    fn locate_roots(&self, ctx: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
        // Project gate: require a project.godot manifest in the workspace
        // before probing for the extension API JSON. Stray .gd files in a
        // non-Godot project (test fixtures, examples) do not trigger SDK
        // indexing.
        if !project_has_godot_manifest(ctx.project_root) {
            debug!("godot-api: no project.godot in workspace; skipping API probe");
            return Vec::new();
        }
        match probe_extension_api_json() {
            Some(path) => vec![ExternalDepRoot {
                module_path: "godot-api".to_string(),
                version: String::new(),
                root: path,
                ecosystem: LEGACY_ECOSYSTEM_TAG,
                package_id: None,
                requested_imports: Vec::new(),
            }],
            None => Vec::new(),
        }
    }

    fn walk_root(&self, _dep: &ExternalDepRoot) -> Vec<WalkedFile> {
        Vec::new()
    }

    fn uses_demand_driven_parse(&self) -> bool {
        true
    }

    fn parse_metadata_only(&self, dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        parse_extension_api_json(&dep.root).map(Some).unwrap_or(None)
    }

    /// Build a `(module, name) → json-file` index covering every class name,
    /// qualified method name (`ClassName.method_name`), singleton, global
    /// function, global enum, and global constant from the extension API JSON.
    ///
    /// All names point at the same json file (`dep.root`) because the entire
    /// Godot API surface lives in that single artefact. The demand loop uses
    /// this index to confirm that a given GDScript ref is a Godot API symbol
    /// before issuing a `parse_metadata_only` call to synthesise its
    /// `ParsedFile`.
    fn build_symbol_index(&self, dep_roots: &[ExternalDepRoot]) -> SymbolLocationIndex {
        let mut idx = SymbolLocationIndex::new();
        for dep in dep_roots {
            index_extension_api_json(&dep.root, &dep.module_path, &mut idx);
        }
        if !idx.is_empty() {
            debug!("godot-api: indexed {} symbol locations", idx.len());
        }
        idx
    }
}

impl ExternalSourceLocator for GodotApiEcosystem {
    fn ecosystem(&self) -> &'static str { LEGACY_ECOSYSTEM_TAG }

    fn locate_roots(&self, _project_root: &Path) -> Vec<ExternalDepRoot> {
        Ecosystem::locate_roots(
            self,
            &LocateContext {
                project_root: _project_root,
                manifests: &Default::default(),
                active_ecosystems: &[],
            },
        )
    }

    fn parse_metadata_only(&self, project_root: &Path) -> Option<Vec<ParsedFile>> {
        // The indexer's legacy metadata-only call passes project_root; for a
        // stdlib ecosystem there's no project-scoped root. Delegate to the
        // probe, gated on project.godot presence.
        if !project_has_godot_manifest(project_root) {
            return None;
        }
        let path = probe_extension_api_json()?;
        parse_extension_api_json(&path)
    }
}

// ---------------------------------------------------------------------------
// Project gate
// ---------------------------------------------------------------------------

/// Walk the project tree looking for a `project.godot` manifest. The file
/// always sits at a Godot project root; we cap recursion at depth 4 because
/// canonical Godot projects keep it shallow.
fn project_has_godot_manifest(project_root: &Path) -> bool {
    walk_for_project_godot(project_root, 0)
}

fn walk_for_project_godot(dir: &Path, depth: u32) -> bool {
    if depth >= 4 { return false }
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        let path = entry.path();
        if ft.is_file()
            && path.file_name().and_then(|n| n.to_str()) == Some("project.godot")
        {
            return true;
        }
        if ft.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if matches!(name, ".git" | ".godot" | "node_modules" | "target" | "build") {
                    continue;
                }
            }
            if walk_for_project_godot(&path, depth + 1) {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Probe
// ---------------------------------------------------------------------------

fn probe_extension_api_json() -> Option<PathBuf> {
    if let Some(explicit) = std::env::var_os("BEARWISDOM_GODOT_API_JSON") {
        let p = PathBuf::from(explicit);
        if p.is_file() {
            return Some(p);
        }
    }
    // Adjacent to a Godot binary if pointed to by env.
    for env_key in ["GODOT_BIN", "GODOT_HOME", "GODOT"] {
        let Some(val) = std::env::var_os(env_key) else { continue };
        let base = PathBuf::from(val);
        let candidate = if base.is_file() {
            base.parent().map(|p| p.join("extension_api.json"))
        } else {
            Some(base.join("extension_api.json"))
        };
        if let Some(p) = candidate {
            if p.is_file() { return Some(p); }
        }
    }
    // Common user-local install paths.
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        for sub in [".godot", "godot", "Godot"] {
            let p = PathBuf::from(&home).join(sub).join("extension_api.json");
            if p.is_file() { return Some(p); }
        }
    }
    // BearWisdom-managed cache: ~/.bearwisdom/godot/extension_api.json.
    // Populated on first run when Godot is not installed locally (fetch below).
    if let Some(p) = bw_cache_path() {
        if p.is_file() {
            return Some(p);
        }
        // Cache miss — attempt a one-time network fetch from the godot-cpp
        // repository, which ships a copy of the current stable extension API.
        // Failure is silent; the file is simply absent and the walker degrades
        // gracefully (no external symbols → lower resolution rate).
        if fetch_extension_api_to_cache(&p) {
            return Some(p);
        }
    }
    None
}

/// Returns the BearWisdom-managed cache path for extension_api.json, or None
/// if the home directory cannot be determined.
fn bw_cache_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let dir = PathBuf::from(home).join(".bearwisdom").join("godot");
    Some(dir.join("extension_api.json"))
}

/// Attempt to download extension_api.json from the godot-cpp GitHub repository
/// (master branch, `gdextension/extension_api.json`) into `dest`.
///
/// Returns true only when the file is fully written and non-empty.
fn fetch_extension_api_to_cache(dest: &Path) -> bool {
    const URL: &str = "https://raw.githubusercontent.com/godotengine/godot-cpp/master/gdextension/extension_api.json";

    if let Some(parent) = dest.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return false;
        }
    }

    // Use a blocking HTTP GET via std::process (no reqwest in ecosystem crates).
    // curl is universally available on the target platforms (Linux, macOS, Windows 10+).
    let status = std::process::Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--location",
            "--max-time", "30",
            "--output", dest.to_str().unwrap_or(""),
            URL,
        ])
        .status();

    match status {
        Ok(s) if s.success() => {
            let ok = dest.is_file()
                && std::fs::metadata(dest).map(|m| m.len() > 10_000).unwrap_or(false);
            if ok {
                debug!("GodotApi: cached extension_api.json at {}", dest.display());
            } else {
                let _ = std::fs::remove_file(dest);
            }
            ok
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// JSON → SymbolLocationIndex (demand-driven offering)
// ---------------------------------------------------------------------------

/// Read `extension_api.json` at `path` and register every top-level name
/// (class, builtin class, singleton, global function, global enum, global
/// constant) plus qualified member names (`ClassName.member`) in `idx`, all
/// pointing at `path`. The module key is `module_path` as set by
/// `locate_roots` (conventionally `"godot-api"`).
fn index_extension_api_json(path: &Path, module_path: &str, idx: &mut SymbolLocationIndex) {
    let Ok(bytes) = std::fs::read(path) else { return };
    let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else { return };

    // Classes and builtin classes: register the class name itself plus every
    // method, property, signal, constant, and enum as `ClassName.member`.
    for section in &["classes", "builtin_classes"] {
        for class in iter_array(&json, section) {
            let Some(name) = class.get("name").and_then(|v| v.as_str()) else { continue };
            idx.insert(module_path, name, path);

            for member_key in &["methods", "properties", "signals", "constants", "enums"] {
                for member in iter_array(class, member_key) {
                    let Some(m_name) = member.get("name").and_then(|v| v.as_str()) else { continue };
                    idx.insert(module_path, &format!("{name}.{m_name}"), path);
                    // Also register the bare member name so a chain walker
                    // looking up a method without a receiver prefix can locate it.
                    idx.insert(module_path, m_name, path);
                }
            }
        }
    }

    // Singletons (Input, OS, ClassDB, ...): top-level global names.
    for sing in iter_array(&json, "singletons") {
        let Some(name) = sing.get("name").and_then(|v| v.as_str()) else { continue };
        idx.insert(module_path, name, path);
    }

    // Utility functions (print, abs, clamp, ...).
    for fun in iter_array(&json, "utility_functions") {
        let Some(name) = fun.get("name").and_then(|v| v.as_str()) else { continue };
        idx.insert(module_path, name, path);
    }

    // Global enums and their values.
    for enu in iter_array(&json, "global_enums") {
        let Some(name) = enu.get("name").and_then(|v| v.as_str()) else { continue };
        idx.insert(module_path, name, path);
        for value in iter_array(enu, "values") {
            let Some(v_name) = value.get("name").and_then(|v| v.as_str()) else { continue };
            idx.insert(module_path, v_name, path);
        }
    }

    // Global constants.
    for cst in iter_array(&json, "global_constants") {
        let Some(name) = cst.get("name").and_then(|v| v.as_str()) else { continue };
        idx.insert(module_path, name, path);
    }
}

// ---------------------------------------------------------------------------
// JSON → ParsedFile synthesis
// ---------------------------------------------------------------------------

fn parse_extension_api_json(path: &Path) -> Option<Vec<ParsedFile>> {
    let bytes = std::fs::read(path).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let mut out: Vec<ParsedFile> = Vec::new();

    for class in iter_array(&json, "classes") {
        if let Some(pf) = synth_class(class, path) {
            out.push(pf);
        }
    }
    for class in iter_array(&json, "builtin_classes") {
        if let Some(pf) = synth_class(class, path) {
            out.push(pf);
        }
    }

    // Globals file — singletons, global enums, utility functions, constants.
    if let Some(pf) = synth_globals(&json, path) {
        out.push(pf);
    }

    debug!(
        "GodotApi: synthesized {} ParsedFile entries from {}",
        out.len(),
        path.display()
    );
    if out.is_empty() { None } else { Some(out) }
}

fn iter_array<'a>(
    json: &'a serde_json::Value,
    key: &str,
) -> impl Iterator<Item = &'a serde_json::Value> {
    json.get(key)
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
}

fn synth_class(class: &serde_json::Value, json_path: &Path) -> Option<ParsedFile> {
    let name = class.get("name")?.as_str()?.to_string();
    if name.is_empty() { return None; }

    let virtual_path = format!("ext:gdscript-stdlib/{name}.gd");
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();

    let class_kind = if is_interface_like(&name) { SymbolKind::Interface } else { SymbolKind::Class };
    let inherits = class
        .get("inherits")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let signature = if inherits.is_empty() {
        format!("class {name}")
    } else {
        format!("class {name} extends {inherits}")
    };
    symbols.push(ExtractedSymbol {
        name: name.clone(),
        qualified_name: name.clone(),
        kind: class_kind,
        visibility: Some(Visibility::Public),
        start_line: 0, end_line: 0, start_col: 0, end_col: 0,
        signature: Some(signature),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
            declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
});
    let class_index = 0usize;

    for method in iter_array(class, "methods") {
        let Some(m_name) = method.get("name").and_then(|v| v.as_str()) else { continue };
        let return_type = method
            .get("return_value")
            .and_then(|r| r.get("type"))
            .and_then(|t| t.as_str())
            .unwrap_or("void")
            .to_string();
        let args = method
            .get("arguments")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| {
                        let n = a.get("name").and_then(|v| v.as_str())?;
                        let t = a.get("type").and_then(|v| v.as_str())?;
                        Some(format!("{n}: {t}"))
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        symbols.push(ExtractedSymbol {
            name: m_name.to_string(),
            qualified_name: format!("{name}.{m_name}"),
            kind: SymbolKind::Method,
            visibility: Some(Visibility::Public),
            start_line: 0, end_line: 0, start_col: 0, end_col: 0,
            signature: Some(format!("func {m_name}({args}) -> {return_type}")),
            doc_comment: None,
            scope_path: Some(name.clone()),
            parent_index: Some(class_index),
            byte_offset: 0,
                    declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
});
    }

    for prop in iter_array(class, "properties") {
        let Some(p_name) = prop.get("name").and_then(|v| v.as_str()) else { continue };
        let p_type = prop
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("Variant")
            .to_string();
        symbols.push(ExtractedSymbol {
            name: p_name.to_string(),
            qualified_name: format!("{name}.{p_name}"),
            kind: SymbolKind::Property,
            visibility: Some(Visibility::Public),
            start_line: 0, end_line: 0, start_col: 0, end_col: 0,
            signature: Some(format!("var {p_name}: {p_type}")),
            doc_comment: None,
            scope_path: Some(name.clone()),
            parent_index: Some(class_index),
            byte_offset: 0,
                    declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
});
    }

    for sig in iter_array(class, "signals") {
        let Some(s_name) = sig.get("name").and_then(|v| v.as_str()) else { continue };
        symbols.push(ExtractedSymbol {
            name: s_name.to_string(),
            qualified_name: format!("{name}.{s_name}"),
            kind: SymbolKind::Field,
            visibility: Some(Visibility::Public),
            start_line: 0, end_line: 0, start_col: 0, end_col: 0,
            signature: Some(format!("signal {s_name}")),
            doc_comment: None,
            scope_path: Some(name.clone()),
            parent_index: Some(class_index),
            byte_offset: 0,
                    declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
});
    }

    for cst in iter_array(class, "constants") {
        let Some(c_name) = cst.get("name").and_then(|v| v.as_str()) else { continue };
        symbols.push(ExtractedSymbol {
            name: c_name.to_string(),
            qualified_name: format!("{name}.{c_name}"),
            kind: SymbolKind::Field,
            visibility: Some(Visibility::Public),
            start_line: 0, end_line: 0, start_col: 0, end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path: Some(name.clone()),
            parent_index: Some(class_index),
            byte_offset: 0,
                    declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
});
    }

    for en in iter_array(class, "enums") {
        let Some(e_name) = en.get("name").and_then(|v| v.as_str()) else { continue };
        symbols.push(ExtractedSymbol {
            name: e_name.to_string(),
            qualified_name: format!("{name}.{e_name}"),
            kind: SymbolKind::Enum,
            visibility: Some(Visibility::Public),
            start_line: 0, end_line: 0, start_col: 0, end_col: 0,
            signature: Some(format!("enum {e_name}")),
            doc_comment: None,
            scope_path: Some(name.clone()),
            parent_index: Some(class_index),
            byte_offset: 0,
                    declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
});
        for value in iter_array(en, "values") {
            let Some(v_name) = value.get("name").and_then(|v| v.as_str()) else { continue };
            symbols.push(ExtractedSymbol {
                name: v_name.to_string(),
                qualified_name: format!("{name}.{e_name}.{v_name}"),
                kind: SymbolKind::EnumMember,
                visibility: Some(Visibility::Public),
                start_line: 0, end_line: 0, start_col: 0, end_col: 0,
                signature: None,
                doc_comment: None,
                scope_path: Some(format!("{name}.{e_name}")),
                parent_index: None,
                byte_offset: 0,
                            declared_type: None,
                return_type: None,
                param_types: Vec::new(),
                generic_params: Vec::new(),
});
        }
    }

    Some(build_parsed_file(virtual_path, symbols, json_path))
}

fn synth_globals(json: &serde_json::Value, json_path: &Path) -> Option<ParsedFile> {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();

    for sing in iter_array(json, "singletons") {
        let Some(name) = sing.get("name").and_then(|v| v.as_str()) else { continue };
        let ty = sing.get("type").and_then(|v| v.as_str()).unwrap_or(name);
        symbols.push(ExtractedSymbol {
            name: name.to_string(),
            qualified_name: name.to_string(),
            kind: SymbolKind::Variable,
            visibility: Some(Visibility::Public),
            start_line: 0, end_line: 0, start_col: 0, end_col: 0,
            signature: Some(format!("var {name}: {ty}")),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
            byte_offset: 0,
                    declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
});
    }

    for fun in iter_array(json, "utility_functions") {
        let Some(name) = fun.get("name").and_then(|v| v.as_str()) else { continue };
        let return_type = fun
            .get("return_type")
            .and_then(|v| v.as_str())
            .unwrap_or("Variant")
            .to_string();
        let args = fun
            .get("arguments")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| {
                        let n = a.get("name").and_then(|v| v.as_str())?;
                        let t = a.get("type").and_then(|v| v.as_str())?;
                        Some(format!("{n}: {t}"))
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        symbols.push(ExtractedSymbol {
            name: name.to_string(),
            qualified_name: name.to_string(),
            kind: SymbolKind::Function,
            visibility: Some(Visibility::Public),
            start_line: 0, end_line: 0, start_col: 0, end_col: 0,
            signature: Some(format!("func {name}({args}) -> {return_type}")),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
            byte_offset: 0,
                    declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
});
    }

    for enu in iter_array(json, "global_enums") {
        let Some(name) = enu.get("name").and_then(|v| v.as_str()) else { continue };
        symbols.push(ExtractedSymbol {
            name: name.to_string(),
            qualified_name: name.to_string(),
            kind: SymbolKind::Enum,
            visibility: Some(Visibility::Public),
            start_line: 0, end_line: 0, start_col: 0, end_col: 0,
            signature: Some(format!("enum {name}")),
            doc_comment: None,
            scope_path: None,
            parent_index: None,
            byte_offset: 0,
                    declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
});
        for value in iter_array(enu, "values") {
            let Some(v_name) = value.get("name").and_then(|v| v.as_str()) else { continue };
            symbols.push(ExtractedSymbol {
                name: v_name.to_string(),
                qualified_name: format!("{name}.{v_name}"),
                kind: SymbolKind::EnumMember,
                visibility: Some(Visibility::Public),
                start_line: 0, end_line: 0, start_col: 0, end_col: 0,
                signature: None,
                doc_comment: None,
                scope_path: Some(name.to_string()),
                parent_index: None,
                byte_offset: 0,
                            declared_type: None,
                return_type: None,
                param_types: Vec::new(),
                generic_params: Vec::new(),
});
            // Godot convention: enum values ALSO act as global constants
            // (e.g. `SIDE_LEFT`). Emit a top-level variable so project code
            // using the bare name resolves.
            symbols.push(ExtractedSymbol {
                name: v_name.to_string(),
                qualified_name: v_name.to_string(),
                kind: SymbolKind::Variable,
                visibility: Some(Visibility::Public),
                start_line: 0, end_line: 0, start_col: 0, end_col: 0,
                signature: None,
                doc_comment: None,
                scope_path: None,
                parent_index: None,
                byte_offset: 0,
                            declared_type: None,
                return_type: None,
                param_types: Vec::new(),
                generic_params: Vec::new(),
});
        }
    }

    for cst in iter_array(json, "global_constants") {
        let Some(name) = cst.get("name").and_then(|v| v.as_str()) else { continue };
        symbols.push(ExtractedSymbol {
            name: name.to_string(),
            qualified_name: name.to_string(),
            kind: SymbolKind::Variable,
            visibility: Some(Visibility::Public),
            start_line: 0, end_line: 0, start_col: 0, end_col: 0,
            signature: None,
            doc_comment: None,
            scope_path: None,
            parent_index: None,
            byte_offset: 0,
                    declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
});
    }

    if symbols.is_empty() { return None; }
    Some(build_parsed_file(
        "ext:gdscript-stdlib/_globals.gd".to_string(),
        symbols,
        json_path,
    ))
}

fn build_parsed_file(virtual_path: String, symbols: Vec<ExtractedSymbol>, src: &Path) -> ParsedFile {
    let metadata = std::fs::metadata(src).ok();
    let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
    let mtime = metadata
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    let content_hash = format!("{:x}-{}", size, symbols.len());
    ParsedFile {
        path: virtual_path,
        language: "gdscript".to_string(),
        content_hash,
        size,
        line_count: 0,
        mtime,
        package_id: None,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        content: None,
        has_errors: false,
        flow: crate::types::FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),

        plugin_flow_emissions: Vec::new(),
    }
}

/// Godot doesn't have true interfaces, but a handful of abstract-ish base
/// classes (Reference counting managers, etc.) behave more like interfaces.
/// Conservative default: everything is a class. Override here if a more
/// accurate kind emerges.
fn is_interface_like(_name: &str) -> bool { false }

#[cfg(test)]
pub(super) fn _test_index_extension_api_json(
    path: &Path,
    module_path: &str,
    idx: &mut SymbolLocationIndex,
) {
    index_extension_api_json(path, module_path, idx);
}

/// Process-wide shared instance.
pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<GodotApiEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(GodotApiEcosystem)).clone()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[path = "godot_api_tests.rs"]
mod tests;
