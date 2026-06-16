// Robot Framework language hooks: file-context construction (resource-path
// resolution + dynamic-library keyword import entries) and external
// classification. Bare-name resolution runs through the generic engine — see
// `ROBOT_PROFILE`'s `name_normalization` + `file_scoped_imports`.

use super::predicates;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::legacy::{FileContext, ImportEntry, RefContext, SymbolLookup};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

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

fn is_variable_ref(name: &str) -> bool {
    (name.starts_with("${") || name.starts_with("@{") || name.starts_with("&{"))
        && name.ends_with('}')
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
    if let Some((lib, _)) =
        resolve_qualified_library(file_ctx, ref_ctx.extracted_ref.module.as_deref(), target)
    {
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
            let is_file_import = raw_path.ends_with(".robot") || raw_path.ends_with(".resource");
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
                                super::library_map::pick_resource_for_importer(paths, &file.path)
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
                        if let Some(dyn_kws) = robot_state.dynamic_keywords.get(&lib.py_file_path) {
                            for kw in dyn_kws {
                                imports.push(ImportEntry {
                                    imported_name: kw.normalized_name.clone(),
                                    // Scope the alias-decode lookup to the file
                                    // that defines the keyword's symbol. For a
                                    // DynamicCore package the method lives in a
                                    // member module (`keywords/*.py`), not the
                                    // aggregating `__init__.py` the library
                                    // binds to.
                                    module_path: Some(
                                        kw.source_file
                                            .clone()
                                            .unwrap_or_else(|| lib.py_file_path.clone()),
                                    ),
                                    alias: encode_dynamic_alias(&kw.class_name, &kw.method_name),
                                    // Wildcard so the alias-decode pass of the
                                    // file-scoped-import strategy (gated on
                                    // `wildcard_only`) reaches this entry.
                                    is_wildcard: true,
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
}

pub static ROBOT_HOOKS: RobotHooks = RobotHooks;
