// =============================================================================
// languages/handlebars/resolve.rs — Handlebars partial-include resolution
//
// The host extractor emits one Imports ref per `{{> partial-name}}` and the
// shared resolution engine doesn't do path-based file lookup, so without
// this resolver every partial reference lands in unresolved_refs.
//
// Resolution model mirrors MarkdownResolver:
//   1. Resolve target relative to the source file's parent directory.
//   2. Probe candidate file paths covering Handlebars + Mustache conventions:
//      - Bare: `<target>.hbs`, `<target>.handlebars`, `<target>.mustache`
//      - Mustache underscore-prefix: `_<target>.hbs` etc.
//      - Common partial directories searched upward from the source dir:
//        `partials/<target>`, `_partials/<target>`, `_includes/<target>`
//   3. Each candidate matches against the host class symbol the
//      Handlebars extractor emits per file (`SymbolKind::Class` named
//      after the file stem).
// =============================================================================

use std::path::{Component, Path, PathBuf};

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, ImportEntry, LanguageResolver, RefContext, Resolution, SymbolLookup,
};
use crate::types::{EdgeKind, ParsedFile};

pub struct HandlebarsResolver;

impl LanguageResolver for HandlebarsResolver {
    fn language_ids(&self) -> &[&str] {
        &["handlebars", "hbs", "mustache"]
    }

    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> FileContext {
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
        FileContext {
            file_path: file.path.clone(),
            language: "handlebars".to_string(),
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
                    });
                }
            }
        }
        None
    }

    fn infer_external_namespace(
        &self,
        _file_ctx: &FileContext,
        _ref_ctx: &RefContext,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<String> {
        None
    }
}

/// Generate candidate file paths for a Handlebars partial reference.
///
/// Probes (in order):
///   - As-is, then with each Handlebars extension
///   - Mustache underscore-prefix variant: `_<name>.hbs` etc.
///   - kebab-case variant of camelCase names — Ghost / many JS Handlebars
///     setups normalize partial names between the two
///   - Searching upward up to 4 ancestor dirs for common partial folders
fn path_candidates(source_dir: &Path, target: &str) -> Vec<PathBuf> {
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
        // Mustache convention: underscore-prefixed filename inside the
        // same directory. `{{> footer}}` matches `_footer.mustache`.
        if let (Some(parent), Some(stem)) = (base.parent(), base.file_name().and_then(|n| n.to_str())) {
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

    // Build the set of name variants to probe. Always include the original;
    // also include the kebab-cased version when the original looks camelCase.
    let mut name_variants: Vec<String> = vec![target.to_string()];
    if let Some(kebab) = camel_to_kebab(target) {
        name_variants.push(kebab);
    }

    let partial_dirs = ["partials", "_partials", "_includes", "templates"];
    for variant in &name_variants {
        // 1. Direct: variant relative to source dir.
        let direct = lexical_normalize(&source_dir.join(variant));
        push_with_extensions(&mut out, direct);

        // 2. Climb up to 4 ancestors searching for common partial directories.
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

/// Convert camelCase to kebab-case. Returns None when the input is already
/// all-lowercase or has no internal uppercase letter (no conversion needed).
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

fn lexical_normalize(path: &Path) -> PathBuf {
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

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
