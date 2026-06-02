// GSP language hooks. Routes Grails Server Pages onto the shared resolution
// engine: `build_file_context` supplies the per-file context the engine loop
// requires before it runs any strategy, and `resolve_ref` binds the one ref
// shape GSP emits — `<g:render template="...">` (an `Imports` ref) — to the
// target partial's `.gsp` Class symbol. The generic bare resolver declines
// `Imports` refs, so partial binding lands here, mirroring the handlebars /
// markdown partial resolvers.

use std::path::{Component, Path, PathBuf};

use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    FileContext, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct GspHooks;

/// Candidate file paths a `<g:render template="X">` may refer to. Grails
/// resolves `template="x"` to `_x.gsp` in the rendering view's directory, and a
/// directory-qualified `template="shared/foo"` relative to that same directory.
/// The extractor already strips a leading underscore from the attribute value,
/// so `X` is the bare template name; both the underscored partial-file
/// convention and a directly-named file are probed.
///
/// A leading-slash `template="/shared/foo"` is views-root-relative in Grails,
/// but the engine has no views-root anchor — resolving it against the source
/// view's directory would mis-bind to a coincidental file — so it is declined
/// (no candidates) rather than guessed.
pub(crate) fn template_path_candidates(source_dir: &Path, target: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::with_capacity(4);
    let trimmed = target.trim();
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return out;
    }
    let rel = Path::new(trimmed);
    let dir = rel.parent().filter(|p| !p.as_os_str().is_empty());
    let stem = rel
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(trimmed);

    // A path with a directory segment (`shared/foo`) resolves relative to the
    // source view's directory; a bare name (`foo`) resolves in the same dir.
    let base = match dir {
        Some(d) => source_dir.join(d),
        None => source_dir.to_path_buf(),
    };
    for name in [format!("_{stem}.gsp"), format!("{stem}.gsp")] {
        out.push(lexical_normalize(&base.join(name)));
    }
    out
}

/// Resolve `.`/`..` segments lexically without touching the filesystem, so the
/// candidate paths match the forward-slash form stored on indexed symbols.
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

impl LanguageEngineHooks for GspHooks {
    fn build_file_context(
        &self,
        file: &ParsedFile,
        _project_ctx: Option<&ProjectContext>,
    ) -> Option<FileContext> {
        Some(FileContext {
            file_path: file.path.clone(),
            language: "gsp".to_string(),
            imports: Vec::new(),
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
        for candidate in template_path_candidates(source_dir, target) {
            let path_str = candidate.to_string_lossy().replace('\\', "/");
            for sym in lookup.in_file(&path_str) {
                if sym.kind == "class" {
                    return Some(Resolution {
                        target_symbol_id: sym.id,
                        confidence: 1.0,
                        strategy: "gsp_template",
                        resolved_yield_type: None,
                        flow_emit: None,
                    });
                }
            }
        }
        None
    }
}

pub static GSP_HOOKS: GspHooks = GspHooks;

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod hooks_tests;
