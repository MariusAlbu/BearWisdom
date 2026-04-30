// =============================================================================
// languages/robot/resolve.rs — Robot Framework resolution rules
//
// Robot Framework references:
//
//   Library    SeleniumLibrary      → Imports, target_name = "SeleniumLibrary"
//   Resource   common/keywords.robot → Imports, target_name = "common/keywords.robot"
//   Variables  vars/config.yaml     → Imports, target_name = "vars/config.yaml"
//
//   Log  Hello World                → Calls, target_name = "Log"
//   Should Be Equal  ${a}  ${b}     → Calls, target_name = "Should Be Equal"
//   My Custom Keyword               → Calls, target_name = "My Custom Keyword"
//   SeleniumLibrary.Click Element   → Calls, target_name = "SeleniumLibrary.Click Element"
//                                              (qualified — library-scoped)
//   ${MY_VAR}                       → Calls, target_name = "${MY_VAR}" (variable ref)
//
// Robot keyword names are case-insensitive and spaces are treated as
// underscores/normalized when matching.
//
// Resolution strategy:
//   1. Qualified `Library.Keyword` → external (library name as namespace).
//   2. Variable reference `${name}` / `@{name}` / `&{name}` → same-file/suite variable symbol.
//   3. Same-file: keywords defined in the same `.robot` file.
//   4. Imported resource file keywords: for each Resource import, look in that file
//      by normalized name (case-insensitive, spaces == underscores).
//   5. Global name lookup via resolve_common (handles remaining cross-file cases).
//   6. Library keywords and BuiltIn are external.
// =============================================================================

use super::predicates;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolInfo, SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile, SymbolKind};

pub struct RobotResolver;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return true if `name` is a Robot variable reference: starts with `${`, `@{`, or `&{`.
fn is_variable_ref(name: &str) -> bool {
    (name.starts_with("${") || name.starts_with("@{") || name.starts_with("&{"))
        && name.ends_with('}')
}

/// Strip `${…}` / `@{…}` / `&{…}` delimiters → inner name, normalised.
/// `${MY_VAR}` → `"my_var"`, `@{LIST}` → `"list"`.
fn variable_inner_normalized(name: &str) -> Option<String> {
    if name.len() < 4 {
        return None;
    }
    // First char is `$`, `@`, or `&`; second is `{`; last is `}`.
    let inner = &name[2..name.len() - 1];
    if inner.is_empty() {
        return None;
    }
    Some(predicates::normalize_robot_name(inner))
}

/// Extract the library prefix from a qualified `Library.Keyword Name` target.
/// Returns `Some(library_name)` if the target is qualified.
fn qualified_library_prefix(target: &str) -> Option<&str> {
    // Robot qualified syntax: `LibraryName.Keyword Name`
    // The library name is the part before the first `.`.
    let dot = target.find('.')?;
    let prefix = &target[..dot];
    // Sanity: prefix must be a non-empty identifier (no spaces, no `${`).
    if prefix.is_empty() || prefix.contains(' ') || prefix.contains('{') {
        return None;
    }
    Some(prefix)
}

/// Check if `library_name` is imported as a Library (not a Resource) in this file.
fn is_library_import(file_ctx: &FileContext, library_name: &str) -> bool {
    let norm_lib = predicates::normalize_robot_name(library_name);
    file_ctx.imports.iter().any(|imp| {
        // Library imports have is_wildcard=false (set below) but we identify them
        // by the fact that their module_path does NOT end with .robot/.resource/.yaml/.py.
        // We also match directly on the imported_name (the library name).
        let norm_imp = predicates::normalize_robot_name(&imp.imported_name);
        norm_imp == norm_lib
            && imp.module_path.as_deref().map_or(true, |p| {
                !p.ends_with(".robot") && !p.ends_with(".resource")
            })
    })
}

/// Find a Variable symbol matching the inner name of a variable ref.
fn resolve_variable<'a>(
    normalized_inner: &str,
    symbols: &'a [SymbolInfo],
) -> Option<&'a SymbolInfo> {
    symbols.iter().find(|s| {
        s.kind == SymbolKind::Variable.as_str()
            && predicates::normalize_robot_name(&s.name) == normalized_inner
    })
}

impl LanguageResolver for RobotResolver {
    fn language_ids(&self) -> &[&str] {
        &["robot"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
        let mut imports = Vec::new();

        for r in &file.refs {
            if r.kind != EdgeKind::Imports {
                continue;
            }
            let path = r.module.as_deref().unwrap_or(&r.target_name);
            // Resource / Variables imports are file-based — follow them for wildcard lookup.
            // Library imports are external packages — mark is_wildcard=false so resolve_common
            // step 2 doesn't incorrectly try to find library keywords in project files.
            let is_file_import =
                path.ends_with(".robot") || path.ends_with(".resource");
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: r.module.clone().or_else(|| Some(r.target_name.clone())),
                alias: None,
                is_wildcard: is_file_import,
            });
        }

        FileContext {
            file_path: file.path.clone(),
            language: "robot".to_string(),
            imports,
            file_namespace: None,
        }
    }

    fn resolve(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        lookup: &dyn SymbolLookup,
    ) -> Option<Resolution> {
        let target = &ref_ctx.extracted_ref.target_name;
        let edge_kind = ref_ctx.extracted_ref.kind;

        // Import declarations don't resolve to a symbol.
        if edge_kind == EdgeKind::Imports {
            return None;
        }

        // Step 1: Qualified `Library.Keyword` calls are external — never resolve against
        // the project index. Two forms:
        //   a) `module` field set by extractor: `SeleniumLibrary` + target `Click Element`
        //   b) Legacy dotted target (no module split): `SeleniumLibrary.Click Element`
        let library_module: Option<&str> = ref_ctx
            .extracted_ref
            .module
            .as_deref()
            .filter(|m| !m.ends_with(".robot") && !m.ends_with(".resource"))
            .or_else(|| qualified_library_prefix(target));
        if let Some(lib) = library_module {
            if is_library_import(file_ctx, lib) || predicates::is_robot_builtin_library(lib) {
                return None;
            }
        }

        // Step 2: Variable references — `${VAR}`, `@{LIST}`, `&{DICT}`.
        if is_variable_ref(target) {
            if let Some(norm_inner) = variable_inner_normalized(target) {
                // Same-file variable.
                if let Some(sym) = resolve_variable(&norm_inner, lookup.in_file(&file_ctx.file_path)) {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "robot_variable_same_file",
                        resolved_yield_type: None,
                    });
                }
                // Resource-imported file variables.
                for import in &file_ctx.imports {
                    let Some(path) = &import.module_path else { continue };
                    if !path.ends_with(".robot") && !path.ends_with(".resource") {
                        continue;
                    }
                    if let Some(sym) = resolve_variable(&norm_inner, lookup.in_file(path)) {
                        return Some(Resolution {
                            target_symbol_id: sym.id,
                            confidence: 0.95,
                            strategy: "robot_variable_resource",
                            resolved_yield_type: None,
                        });
                    }
                }
            }
            // Variable not found in project — let it be classified external.
            return None;
        }

        // Robot keyword names are compared normalized (lowercase, spaces → underscores).
        let normalized_target = predicates::normalize_robot_name(target);

        // Step 3: Same-file keyword resolution.
        // Checked BEFORE the builtin guard — project keywords shadow library keywords.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if sym.kind == SymbolKind::Function.as_str()
                && predicates::normalize_robot_name(&sym.name) == normalized_target
            {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "robot_same_file",
                    resolved_yield_type: None,
                });
            }
        }

        // Step 4: Imported resource file keywords — normalized name match.
        for import in &file_ctx.imports {
            let Some(path) = &import.module_path else {
                continue;
            };
            // Only follow .robot/.resource imports — skip Library and Variables imports.
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
                    });
                }
            }
        }

        // Step 5: Global cross-file normalized lookup.
        // BuiltIn / stdlib / SeleniumLibrary / Browser keywords resolve here via
        // synthetic symbols populated by the robot_*_synthetics ecosystems.
        //
        // Scoping rule: only bind to a project-internal symbol (non-ext: path) when
        // the file has no library imports — i.e., the project is purely resource-based.
        // When library imports are present, non-synthetic global matches are skipped to
        // prevent coincidentally-named project keywords from capturing library keyword refs.
        let has_library_imports = file_ctx.imports.iter().any(|imp| {
            imp.module_path.as_deref().map_or(true, |p| {
                !p.ends_with(".robot") && !p.ends_with(".resource")
            })
        });

        // Two-pass global lookup: prefer synthetic symbols (ext: paths); fall back to
        // internal symbols only when no library imports are active in the file.
        let pick_from = |syms: &[SymbolInfo], confidence: f64, strategy: &'static str| -> Option<Resolution> {
            let synth = syms.iter().find(|s| {
                s.kind == SymbolKind::Function.as_str()
                    && predicates::normalize_robot_name(&s.name) == normalized_target
                    && s.file_path.starts_with("ext:")
            });
            if let Some(sym) = synth {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence,
                    strategy,
                    resolved_yield_type: None,
                });
            }
            if !has_library_imports {
                let internal = syms.iter().find(|s| {
                    s.kind == SymbolKind::Function.as_str()
                        && predicates::normalize_robot_name(&s.name) == normalized_target
                });
                if let Some(sym) = internal {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence,
                        strategy,
                        resolved_yield_type: None,
                    });
                }
            }
            None
        };

        if let Some(res) = pick_from(lookup.by_name(target), 0.90_f64, "robot_global_name") {
            return Some(res);
        }
        // Also try normalized form lookup — handles `click_element` matching `Click Element`.
        let normalized_snake = normalized_target.replace('_', " ");
        if normalized_snake != target.to_ascii_lowercase() {
            if let Some(res) = pick_from(lookup.by_name(&normalized_snake), 0.85_f64, "robot_global_normalized") {
                return Some(res);
            }
        }

        None
    }

    fn infer_external_namespace(
        &self,
        file_ctx: &FileContext,
        ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        let target = &ref_ctx.extracted_ref.target_name;

        // Library imports: non-file-path imports are external Robot libraries.
        if ref_ctx.extracted_ref.kind == EdgeKind::Imports {
            let path = ref_ctx.extracted_ref.module.as_deref().unwrap_or(target);
            if !path.contains('/') && !path.contains('\\') && !path.ends_with(".robot")
                && !path.ends_with(".resource")
            {
                return Some("robot".to_string());
            }
            return None;
        }

        // Qualified `Library.Keyword` call: namespace is the library name.
        // Check both the module field (set by extractor) and legacy dotted target.
        let library_module: Option<&str> = ref_ctx
            .extracted_ref
            .module
            .as_deref()
            .filter(|m| !m.ends_with(".robot") && !m.ends_with(".resource"))
            .or_else(|| qualified_library_prefix(target));
        if let Some(lib) = library_module {
            if is_library_import(file_ctx, lib) || predicates::is_robot_builtin_library(lib) {
                return Some(lib.to_string());
            }
        }

        // Variable references that weren't resolved are external (env vars, CLI vars, etc.).
        if is_variable_ref(target) {
            return Some("robot".to_string());
        }

        None
    }
}
