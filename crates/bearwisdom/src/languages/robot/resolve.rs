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
//
// Robot keyword names are case-insensitive and spaces are treated as
// underscores/normalized when matching.
//
// Resolution strategy:
//   1. Same-file: keywords defined in the same `.robot` file.
//   2. Imported resource keywords: for each Resource import, look in that file.
//   3. Global name lookup (case-insensitive normalized name match).
//   4. Library keywords and BuiltIn are external.
// =============================================================================

use super::builtins;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, LanguageResolver, RefContext, Resolution,
    SymbolLookup,
};
use crate::indexer::project_context::ProjectContext;
use crate::types::{EdgeKind, ParsedFile};

pub struct RobotResolver;

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
            imports.push(ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: r.module.clone().or_else(|| Some(r.target_name.clone())),
                alias: None,
                // Library imports bring all keywords into scope — treat as wildcard.
                is_wildcard: true,
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

        // Robot BuiltIn library keywords are external.
        if builtins::is_robot_builtin(target) {
            return None;
        }

        // Robot keyword names are compared normalized (lowercase, spaces → underscores).
        let normalized_target = builtins::normalize_robot_name(target);

        // Step 1: Same-file keyword resolution.
        for sym in lookup.in_file(&file_ctx.file_path) {
            if builtins::normalize_robot_name(&sym.name) == normalized_target {
                return Some(Resolution {
                    target_symbol_id: sym.id,
                    confidence: 1.0,
                    strategy: "robot_same_file",
                });
            }
        }

        // Step 2: Imported resource file keywords.
        for import in &file_ctx.imports {
            let Some(path) = &import.module_path else {
                continue;
            };
            // Skip library imports (they're external) — only follow .robot/.resource files.
            if !path.ends_with(".robot") && !path.ends_with(".resource") {
                continue;
            }
            for sym in lookup.in_file(path) {
                if builtins::normalize_robot_name(&sym.name) == normalized_target {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "robot_resource_import",
                    });
                }
            }
        }

        // Step 3: Common resolution (handles import-based, scope chain, qualified names).
        // Robot has no scope chain, but resolve_common covers cross-file imports and
        // qualified lookups without adding a raw by_name fallback.
        engine::resolve_common("robot", file_ctx, ref_ctx, lookup, builtins::kind_compatible)
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
            if !path.contains('/') && !path.contains('\\') && !path.ends_with(".robot") {
                return Some("robot".to_string());
            }
            return None;
        }

        engine::infer_external_common(file_ctx, ref_ctx, builtins::is_robot_builtin)
            .map(|_| "robot".to_string())
    }
}
