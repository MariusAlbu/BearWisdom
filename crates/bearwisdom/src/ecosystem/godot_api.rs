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
// Activation: any `.gd` file in the project. If the JSON isn't findable,
// the ecosystem degrades silently (returns no files) and unresolved rates
// rise honestly — matching BearWisdom's "toolchains must be installed for
// full resolution" policy.
// =============================================================================

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::debug;

use super::{
    Ecosystem, EcosystemActivation, EcosystemId, EcosystemKind, LocateContext,
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

    fn locate_roots(&self, _: &LocateContext<'_>) -> Vec<ExternalDepRoot> {
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
        // No source walk; extension_api.json drives synthesis via
        // parse_metadata_only.
        Vec::new()
    }

    fn parse_metadata_only(&self, dep: &ExternalDepRoot) -> Option<Vec<ParsedFile>> {
        parse_extension_api_json(&dep.root).map(Some).unwrap_or(None)
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

    fn parse_metadata_only(&self, _project_root: &Path) -> Option<Vec<ParsedFile>> {
        // The indexer's legacy metadata-only call passes project_root; for a
        // stdlib ecosystem there's no project-scoped root. Delegate to the
        // probe.
        let path = probe_extension_api_json()?;
        parse_extension_api_json(&path)
    }
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
    None
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
        connection_points: Vec::new(),
        demand_contributions: Vec::new(),
    }
}

/// Godot doesn't have true interfaces, but a handful of abstract-ish base
/// classes (Reference counting managers, etc.) behave more like interfaces.
/// Conservative default: everything is a class. Override here if a more
/// accurate kind emerges.
fn is_interface_like(_name: &str) -> bool { false }

/// Process-wide shared instance.
pub fn shared_locator() -> Arc<dyn ExternalSourceLocator> {
    use std::sync::OnceLock;
    static LOCATOR: OnceLock<Arc<GodotApiEcosystem>> = OnceLock::new();
    LOCATOR.get_or_init(|| Arc::new(GodotApiEcosystem)).clone()
}
