// Robot Framework language hooks. Absorbed from the deleted `robot/resolve.rs`.

use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolInfo, SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile, SymbolKind};

pub struct RobotHooks;

const DYN_ALIAS_SEP: &str = "::";

fn encode_dynamic_alias(class: &Option<String>, method: &Option<String>) -> Option<String> {
    match (class, method) {
        (None, None) => None,
        (Some(c), None) => Some(c.clone()),
        (Some(c), Some(m)) => Some(format!("{c}{DYN_ALIAS_SEP}{m}")),
        (None, Some(m)) => Some(format!("{DYN_ALIAS_SEP}{m}")),
    }
}

fn decode_dynamic_class_name(alias: Option<&str>) -> Option<&str> {
    let s = alias?;
    let cls = s.split(DYN_ALIAS_SEP).next().unwrap_or(s);
    if cls.is_empty() {
        None
    } else {
        Some(cls)
    }
}

fn decode_dynamic_method_name(import: &ImportEntry) -> Option<&str> {
    let s = import.alias.as_deref()?;
    s.split_once(DYN_ALIAS_SEP)
        .map(|(_, m)| m)
        .filter(|m| !m.is_empty())
}

fn is_variable_ref(name: &str) -> bool {
    (name.starts_with("${") || name.starts_with("@{") || name.starts_with("&{"))
        && name.ends_with('}')
}

fn variable_inner_normalized(name: &str) -> Option<String> {
    if name.len() < 4 {
        return None;
    }
    let inner = &name[2..name.len() - 1];
    if inner.is_empty() {
        return None;
    }
    Some(predicates::normalize_robot_name(inner))
}

fn qualified_library_prefix(target: &str) -> Option<&str> {
    let dot = target.find('.')?;
    let prefix = &target[..dot];
    if prefix.is_empty() || prefix.contains(' ') || prefix.contains('{') {
        return None;
    }
    Some(prefix)
}

fn is_library_import(file_ctx: &FileContext, library_name: &str) -> bool {
    let norm_lib = predicates::normalize_robot_name(library_name);
    file_ctx.imports.iter().any(|imp| {
        let norm_imp = predicates::normalize_robot_name(&imp.imported_name);
        norm_imp == norm_lib
            && imp.module_path.as_deref().map_or(true, |p| {
                !p.ends_with(".robot") && !p.ends_with(".resource")
            })
    })
}

fn resolve_qualified_library<'a>(
    file_ctx: &FileContext,
    module: Option<&'a str>,
    target: &'a str,
) -> Option<(String, &'a str)> {
    if let Some(m) = module {
        if !m.ends_with(".robot") && !m.ends_with(".resource") {
            if is_library_import(file_ctx, m) {
                return Some((m.to_string(), target));
            }
        }
    }
    if let Some(m) = module {
        if !m.ends_with(".robot") && !m.ends_with(".resource") {
            let mut composite = m.to_string();
            let mut consumed: usize = 0;
            for (idx, ch) in target.char_indices() {
                if ch == '.' {
                    let seg = &target[consumed..idx];
                    if seg.is_empty() || seg.contains(' ') || seg.contains('{') {
                        break;
                    }
                    composite.push('.');
                    composite.push_str(seg);
                    consumed = idx + 1;
                    if is_library_import(file_ctx, &composite) {
                        return Some((composite, &target[consumed..]));
                    }
                } else if ch == ' ' || ch == '{' {
                    break;
                }
            }
        }
    }
    if module.is_none() {
        if let Some(prefix) = qualified_library_prefix(target) {
            if is_library_import(file_ctx, prefix) {
                let suffix = &target[prefix.len() + 1..];
                return Some((prefix.to_string(), suffix));
            }
        }
    }
    None
}

fn resolve_variable<'a>(
    normalized_inner: &str,
    symbols: &'a [SymbolInfo],
) -> Option<&'a SymbolInfo> {
    symbols.iter().find(|s| {
        s.kind == SymbolKind::Variable.as_str()
            && predicates::normalize_robot_name(&s.name) == normalized_inner
    })
}

pub(crate) fn infer_external_inner(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    _project_ctx: Option<&ProjectContext>,
) -> Option<String> {
    let target = &ref_ctx.extracted_ref.target_name;
    if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
        let path = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
        if !path.contains('/')
            && !path.contains('\\')
            && !path.ends_with(".robot")
            && !path.ends_with(".resource")
        {
            return Some("robot".to_string());
        }
        return None;
    }
    if let Some((lib, _)) = resolve_qualified_library(
        file_ctx,
        ref_ctx.extracted_ref.module.as_deref(),
        target,
    ) {
        return Some(lib);
    }
    if is_variable_ref(target) {
        return Some("robot".to_string());
    }
    None
}

impl LanguageEngineHooks for RobotHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        infer_external_inner(file_ctx, ref_ctx, project_ctx)
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        let mut imports = Vec::new();
        imports.push(ImportEntry {
            imported_name: "BuiltIn".to_string(),
            module_path: Some("BuiltIn".to_string()),
            alias: None,
            is_wildcard: false,
        });
        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let raw_path = r.module.as_deref().unwrap_or(&r.target_name);
            let is_file_import =
                raw_path.ends_with(".robot") || raw_path.ends_with(".resource");
            let resolved_path = if is_file_import {
                let lookup_key = std::path::Path::new(raw_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(raw_path);
                project_ctx
                    .and_then(|ctx| {
                        ctx.plugin_state
                            .get::<super::RobotProjectState>()
                            .and_then(|s| s.resource_basenames.get(lookup_key))
                            .and_then(|paths| {
                                super::library_map::pick_resource_for_importer(
                                    paths, &file.path,
                                )
                                .map(String::from)
                            })
                    })
                    .unwrap_or_else(|| raw_path.to_string())
            } else {
                raw_path.to_string()
            };
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: Some(resolved_path),
                alias: None,
                is_wildcard: is_file_import,
            });
        }
        if let Some(ctx) = project_ctx {
            if let Some(robot_state) = ctx.plugin_state.get::<super::RobotProjectState>() {
                if let Some(libs) = robot_state.library_map.get(&file.path) {
                    for lib in libs {
                        imports.push(ImportEntry {
                            imported_name: lib.library_name.clone(),
                            module_path: Some(lib.py_file_path.clone()),
                            alias: None,
                            is_wildcard: true,
                        });
                        if let Some(dyn_kws) =
                            robot_state.dynamic_keywords.get(&lib.py_file_path)
                        {
                            for kw in dyn_kws {
                                imports.push(ImportEntry {
                                    imported_name: kw.normalized_name.clone(),
                                    module_path: Some(lib.py_file_path.clone()),
                                    alias: encode_dynamic_alias(
                                        &kw.class_name,
                                        &kw.method_name,
                                    ),
                                    is_wildcard: false,
                                });
                            }
                        }
                    }
                }
            }
        }
        Some(FileContext {
            file_path: file.path.clone(),
            language: "robot".to_string(),
            imports,
            file_namespace: None,
        })
    }

    fn resolve_ref(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext<'_>,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        if resolve_qualified_library(
            file_ctx,
            ref_ctx.extracted_ref.module.as_deref(),
            target,
        )
        .is_some()
        {
            return None;
        }
        if is_variable_ref(target) {
            if let Some(norm_inner) = variable_inner_normalized(target) {
                if let Some(sym) =
                    resolve_variable(&norm_inner, lookup.in_file(&file_ctx.file_path))
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "robot_variable_same_file",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
                for import in &file_ctx.imports {
                    let Some(path) = &import.module_path else {
                        continue;
                    };
                    if !path.ends_with(".robot") && !path.ends_with(".resource") {
                        continue;
                    }
                    if let Some(sym) = resolve_variable(&norm_inner, lookup.in_file(path)) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.95,
                            strategy: "robot_variable_resource",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }
            return None;
        }
        let normalized_target = predicates::normalize_robot_name(target);
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.kind == SymbolKind::Function.as_str()
                && predicates::normalize_robot_name(&sym.name) == normalized_target
            {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "robot_same_file",
                    resolved_yield_type: None,
                    flow_emit: None,
                });
            }
        }
        for import in &file_ctx.imports {
            let Some(path) = &import.module_path else {
                continue;
            };
            if !path.ends_with(".robot") && !path.ends_with(".resource") {
                continue;
            }
            for sym in lookup.in_file(path) {
                if sym.kind == SymbolKind::Function.as_str()
                    && predicates::normalize_robot_name(&sym.name) == normalized_target
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "robot_resource_import",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }
        for import in &file_ctx.imports {
            let Some(path) = &import.module_path else {
                continue;
            };
            if !path.ends_with(".py") || !import.is_wildcard {
                continue;
            }
            for sym in lookup.in_file(path) {
                let is_callable =
                    matches!(sym.kind.as_str(), "function" | "method" | "test");
                let py_normalized = predicates::normalize_robot_name(&sym.name);
                if is_callable && py_normalized == normalized_target {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.95,
                        strategy: "robot_python_library",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }
        for import in &file_ctx.imports {
            let Some(path) = &import.module_path else {
                continue;
            };
            if !path.ends_with(".py") || import.is_wildcard {
                continue;
            }
            if import.imported_name != normalized_target {
                continue;
            }
            let target_class = decode_dynamic_class_name(import.alias.as_deref());
            let target_method = decode_dynamic_method_name(import);
            if let Some(method_name) = target_method {
                for sym in lookup.in_file(path) {
                    if sym.name == method_name
                        && matches!(sym.kind.as_str(), "function" | "method" | "test")
                    {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.95,
                            strategy: "robot_dynamic_library_method",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }
            if let Some(class_name) = target_class {
                for sym in lookup.in_file(path) {
                    if sym.kind == SymbolKind::Class.as_str() && sym.name == class_name {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.85,
                            strategy: "robot_dynamic_library",
                            resolved_yield_type: None,
                            flow_emit: None,
                        });
                    }
                }
            }
            for sym in lookup.in_file(path) {
                if sym.kind == SymbolKind::Class.as_str() {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 0.75,
                        strategy: "robot_dynamic_library_fallback",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }
        None
    }
}

pub static ROBOT_HOOKS: RobotHooks = RobotHooks;
