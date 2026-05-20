// Handlebars language hooks. Absorbed from the deleted `handlebars/resolve.rs`.

use std::path::{Component, Path, PathBuf};

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct HandlebarsHooks;

pub(crate) fn path_candidates(source_dir: &Path, target: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::with_capacity(96);
    let extensions = ["hbs", "handlebars", "mustache", "html"];

    let push_with_extensions = |out: &mut Vec<PathBuf>, base: PathBuf| {
        out.push(base.clone());
        let already_hbs = base
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| extensions.contains(&e))
            .unwrap_or(false);
        if !already_hbs {
            let base_str = base.to_string_lossy().to_string();
            for ext in extensions {
                out.push(PathBuf::from(format!("{base_str}.{ext}")));
            }
        }
        if let (Some(parent), Some(stem)) =
            (base.parent(), base.file_name().and_then(|n| n.to_str()))
        {
            let underscored = parent.join(format!("_{stem}"));
            out.push(underscored.clone());
            if !already_hbs {
                let und_str = underscored.to_string_lossy().to_string();
                for ext in extensions {
                    out.push(PathBuf::from(format!("{und_str}.{ext}")));
                }
            }
        }
    };

    let mut name_variants: Vec<String> = vec![target.to_string()];
    if let Some(kebab) = camel_to_kebab(target) {
        name_variants.push(kebab);
    }

    let partial_dirs = ["partials", "_partials", "_includes", "templates"];
    for variant in &name_variants {
        let direct = lexical_normalize(&source_dir.join(variant));
        push_with_extensions(&mut out, direct);
        let mut current = Some(source_dir);
        let mut depth = 0;
        while let Some(dir) = current {
            for p in partial_dirs {
                let candidate = lexical_normalize(&dir.join(p).join(variant));
                push_with_extensions(&mut out, candidate);
            }
            depth += 1;
            if depth > 4 {
                break;
            }
            current = dir.parent();
        }
    }
    out
}

fn camel_to_kebab(s: &str) -> Option<String> {
    let has_upper = s.chars().any(|c| c.is_ascii_uppercase());
    if !has_upper {
        return None;
    }
    let mut out = String::with_capacity(s.len() + 4);
    let mut prev_lower = false;
    for ch in s.chars() {
        if ch.is_ascii_uppercase() {
            if prev_lower {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
            prev_lower = false;
        } else {
            out.push(ch);
            prev_lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        }
    }
    Some(out)
}

pub(crate) fn lexical_normalize(path: &Path) -> PathBuf {
    let mut stack: Vec<Component<'_>> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                let pop_ok = matches!(
                    stack.last(),
                    Some(Component::Normal(_)) | Some(Component::CurDir)
                );
                if pop_ok {
                    stack.pop();
                } else {
                    stack.push(comp);
                }
            }
            Component::CurDir => {}
            other => stack.push(other),
        }
    }
    stack.iter().collect()
}

impl LanguageEngineHooks for HandlebarsHooks {
    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        let imports: Vec<ImportEntry> = file
            .refs
            .iter()
            .filter(|r| r.kind == EdgeKind::Imports)
            .map(|r| ImportEntry {
                imported_name: r.target_name.clone(),
                module_path: None,
                alias: None,
                is_wildcard: false,
            })
            .collect();
        Some(FileContext {
            file_path: file.path.clone(),
            language: "handlebars".to_string(),
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
        if ref_ctx.extracted_ref.kind != EdgeKind::Imports {
            return None;
        }
        let target = ref_ctx.extracted_ref.target_name.trim();
        if target.is_empty() {
            return None;
        }
        let source_dir = Path::new(&file_ctx.file_path).parent()?;
        for candidate in path_candidates(source_dir, target) {
            let path_str = candidate.to_string_lossy().replace('\\', "/");
            let stem = candidate
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            let stem_no_underscore = stem.trim_start_matches('_');
            for sym in lookup.in_file(&path_str) {
                if sym.kind == "class"
                    && (sym.name == stem || sym.name == stem_no_underscore)
                {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "handlebars_partial",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }
        None
    }
}

pub static HANDLEBARS_HOOKS: HandlebarsHooks = HandlebarsHooks;
