// Jinja2 language hooks. Absorbed from the deleted `jinja/resolve.rs`.

use std::path::{Component, Path, PathBuf};

use crate::ecosystem::manifest::ManifestKind;
use crate::indexer::project_context::ProjectContext;
use crate::indexer::resolve::engine::{
    self as engine, FileContext, ImportEntry, RefContext, Resolution, SymbolLookup,
};
use crate::type_checker::profile::hooks::LanguageEngineHooks;
use crate::types::{EdgeKind, ParsedFile};

pub struct JinjaHooks;

pub(crate) fn infer_ansible_external(
    target: &str,
    project_ctx: Option<&ProjectContext>,
) -> Option<String> {
    let ctx = project_ctx?;
    let manifest = ctx.manifests.get(&ManifestKind::AnsibleRequirements)?;
    for role in &manifest.dependencies {
        let prefix = format!("{role}_");
        if target.starts_with(prefix.as_str()) || target == role.as_str() {
            return Some(format!("ansible.{role}"));
        }
    }
    None
}

fn kind_compatible(edge_kind: EdgeKind, sym_kind: &str) -> bool {
    match edge_kind {
        EdgeKind::TypeRef => matches!(
            sym_kind,
            "variable" | "field" | "class" | "function" | "type_alias" | "parameter"
        ),
        EdgeKind::Calls => matches!(sym_kind, "function" | "method"),
        _ => true,
    }
}

fn resolve_template_path(
    file_ctx: &FileContext,
    ref_ctx: &RefContext,
    lookup: &dyn SymbolLookup,
) -> Option<Resolution> {
    let target = &ref_ctx.extracted_ref.target_name;
    if target.is_empty() {
        return None;
    }
    let source_dir = Path::new(&file_ctx.file_path).parent()?;
    let raw = source_dir.join(target);
    let normalized = lexical_normalize(&raw);
    for candidate in candidate_paths(&normalized) {
        let candidate_str = candidate.to_string_lossy().replace('\\', "/");
        if let Some(host) = lookup
            .in_file(&candidate_str)
            .into_iter()
            .find(|s| s.kind == "class")
        {
            return Some(Resolution {
                target_symbol_id: host.id,
                confidence: 0.95,
                strategy: "jinja_template_path",
                resolved_yield_type: None,
                flow_emit: None,
            });
        }
    }
    None
}

fn candidate_paths(normalized: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    out.push(normalized.to_path_buf());
    for ext in &["j2", "jinja", "jinja2"] {
        let mut p = normalized.to_path_buf();
        let cur_ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
        if cur_ext.is_empty() {
            p.set_extension(ext);
            out.push(p);
        } else if cur_ext != *ext {
            let mut combo = normalized.to_path_buf();
            let new_name = format!(
                "{}.{}",
                combo.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                ext
            );
            combo.set_file_name(new_name);
            out.push(combo);
        }
    }
    out
}

fn lexical_normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

impl LanguageEngineHooks for JinjaHooks {
    fn classify_external(
        &self,
        ref_ctx: &RefContext<'_>,
        _file_ctx: &FileContext,
        project_ctx: Option<&ProjectContext>,
        _lookup: &dyn SymbolLookup,
    ) -> Option<String> {
        infer_ansible_external(ref_ctx.extracted_ref.target_name.as_str(), project_ctx)
    }

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
            language: "jinja".to_string(),
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
        match ref_ctx.extracted_ref.kind {
            EdgeKind::Imports => resolve_template_path(file_ctx, ref_ctx, lookup),
            _ => engine::resolve_common("jinja", file_ctx, ref_ctx, lookup, kind_compatible),
        }
    }
}

pub static JINJA_HOOKS: JinjaHooks = JinjaHooks;
