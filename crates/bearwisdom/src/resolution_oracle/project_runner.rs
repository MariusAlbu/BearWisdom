use super::*;
use crate::type_checker::core::types::TypeArena;
use crate::{
    indexer::{
        parse_file::{build_parse_pool, parse_file_with_arena},
        resolve::engine::{compilation::Compilation, pipeline::resolve_from_tree},
        symbol_ids::SymbolIds,
        write::write_parsed_files_with_origin,
    },
    Database,
};
use rayon::prelude::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Instant;

#[cfg(test)]
#[path = "project_runner_tests.rs"]
mod tests;

pub(super) fn run(
    manifest: &ProjectManifest,
    revision: CorpusRevision,
    mode: ProjectBindingMode,
) -> Result<ProjectReport> {
    let started = Instant::now();
    let arena = Arc::new(TypeArena::new());
    let inputs: Vec<_> = manifest
        .files
        .iter()
        .filter(|f| f.language.is_some())
        .collect();
    let progress = AtomicUsize::new(0);
    eprintln!("oracle: parsing {} supplied files", inputs.len());
    let parsed = build_parse_pool()?.install(|| {
        inputs
            .par_iter()
            .map(|file| {
                let language = match file.language.as_deref().unwrap() {
                    "typescript" => "typescript",
                    "tsx" => "tsx",
                    "javascript" => "javascript",
                    "jsx" => "jsx",
                    _ => unreachable!(),
                };
                let result = parse_file_with_arena(
                    &crate::walker::WalkedFile {
                        relative_path: file.index_path.clone(),
                        absolute_path: file.path.clone(),
                        language,
                    },
                    crate::languages::default_registry(),
                    &arena,
                )?;
                ensure!(
                    result.content_hash == file.sha256,
                    "Source changed during parsing: {}",
                    file.path.display()
                );
                let done = progress.fetch_add(1, Ordering::Relaxed) + 1;
                if done % 25 == 0 || done == inputs.len() {
                    eprintln!("oracle: parsed {done}/{}", inputs.len());
                }
                Ok(result)
            })
            .collect::<Result<Vec<_>>>()
    })?;
    let mut db = Database::open_in_memory()?;
    let (file_rows, ids) = write_parsed_files_with_origin(&db, &parsed, "internal", Some(&arena))?;
    let files: HashMap<_, _> = inputs
        .iter()
        .map(|f| (file_rows[&f.index_path], f.id))
        .collect();
    let by_path: HashMap<_, _> = inputs.iter().map(|f| (f.index_path.as_str(), *f)).collect();
    let mut configured_source_gaps = Vec::new();
    if mode == ProjectBindingMode::ConfiguredProgram {
        for file in &manifest.files {
            if file.language.is_none() {
                configured_source_gaps.push(ProgramSourceGap {
                    file: file.id,
                    reason: ProgramSourceGapKind::UnsupportedLanguage,
                });
            }
            if matches!(
                file.source_scope,
                Some(crate::indexer::programs::SourceScope::Unknown)
            ) {
                configured_source_gaps.push(ProgramSourceGap {
                    file: file.id,
                    reason: ProgramSourceGapKind::UnknownScope,
                });
            }
        }
        for file in &parsed {
            let reason = match file.flow.lexical.as_ref().and_then(|g| g.globals.as_ref()) {
                None => Some(ProgramSourceGapKind::MissingGlobalCapture),
                Some(globals) if !globals.complete => {
                    Some(ProgramSourceGapKind::IncompleteGlobalCapture)
                }
                _ => None,
            };
            if let Some(reason) = reason {
                eprintln!(
                    "oracle: configured source barrier {reason:?}: {}",
                    file.path
                );
                configured_source_gaps.push(ProgramSourceGap {
                    file: by_path[file.path.as_str()].id,
                    reason,
                });
            }
        }
    }
    let mut extracted = Vec::new();
    let mut seen = HashSet::new();
    let mut duplicates = 0;
    for file in parsed.iter().filter(|f| by_path[f.path.as_str()].selected) {
        for reference in file.refs.iter().filter(|r| r.kind == EdgeKind::Calls) {
            let site = ReferenceSite {
                file: by_path[file.path.as_str()].id,
                kind: EdgeKind::Calls,
                byte_offset: reference
                    .chain
                    .as_ref()
                    .and_then(|c| c.segments.last())
                    .map(|s| s.byte_offset)
                    .unwrap_or(reference.byte_offset),
            };
            if seen.insert(site) {
                extracted.push(site);
            } else {
                duplicates += 1;
            }
        }
    }
    let labels: Vec<_> = manifest
        .calls
        .iter()
        .filter_map(|c| {
            c.target.map(|target| ExpectedReference {
                site: c.site,
                target: Some(target),
            })
        })
        .collect();
    let compiler_sites: HashSet<_> = manifest.calls.iter().map(|c| c.site).collect();
    let mut unlabelled = BTreeMap::new();
    for call in &manifest.calls {
        if let Some(reason) = &call.reason {
            *unlabelled.entry(reason.clone()).or_default() += 1;
        }
    }
    eprintln!("oracle: building compilation ({mode:?})");
    let mut context = crate::indexer::project_context::build_project_context(&manifest.root);
    if mode == ProjectBindingMode::ConfiguredProgram {
        use crate::indexer::programs::{Program, ProgramSource, SourceScope};
        context.programs = Some(vec![Program {
            key: manifest.config.to_string_lossy().replace('\\', "/"),
            fingerprint: format!("{:x}", Sha256::digest(revision.0)),
            complete: manifest.files.iter().all(|file| file.language.is_some()),
            callable_policy: crate::resolution_oracle::callable_policy::typescript_options(
                &manifest.compiler_options,
            ),
            compiler_intrinsics:
                crate::resolution_oracle::compiler_intrinsic_policy::typescript_options(
                    &manifest.compiler_options,
                ),
            source_binding_order: manifest.source_binding_order.as_ref().and_then(|order| {
                order
                    .iter()
                    .map(|id| {
                        manifest
                            .files
                            .iter()
                            .find(|file| file.id == *id)
                            .map(|file| file.index_path.clone())
                    })
                    .collect()
            }),
            sources: manifest
                .files
                .iter()
                .map(|file| ProgramSource {
                    path: file.index_path.clone(),
                    content_hash: file.sha256.clone(),
                    scope: file.source_scope.unwrap_or(SourceScope::Unknown),
                })
                .collect(),
        }]);
    }
    let tree = Compilation::build_with_context(
        &parsed,
        &ids,
        Arc::clone(&arena),
        Some(&context),
        &Default::default(),
    );
    tree.persist_type_info(db.conn())?;
    let primary: Vec<_> = parsed
        .into_iter()
        .filter(|f| by_path[f.path.as_str()].selected)
        .collect();
    eprintln!("oracle: resolving {} selected files (fresh)", primary.len());
    crate::indexer::parse_file::with_resolve_pool(|| {
        resolve_from_tree(&mut db, tree, &primary, &ids, Some(&context))
    })?;
    let observations = crate::query::oracle_evidence::selector_observations(
        &db,
        &files,
        &[EdgeKind::Calls],
        &extracted,
    )?;
    let fresh = evaluate(revision, &labels, &observations)?;
    eprintln!("oracle: restoring and resolving cold snapshot");
    let restored = Arc::new(TypeArena::new());
    let snapshot: String = db.conn().query_row(
        "SELECT value FROM _bearwisdom_meta WHERE key='type_arena_snapshot'",
        [],
        |r| r.get(0),
    )?;
    restored.restore_snapshot(&snapshot);
    let mut cold_tree = Compilation::build(&[], &SymbolIds::default(), restored);
    cold_tree.ingest_from_db(db.conn());
    crate::indexer::parse_file::with_resolve_pool(|| {
        resolve_from_tree(&mut db, cold_tree, &primary, &ids, Some(&context))
    })?;
    let observations = crate::query::oracle_evidence::selector_observations(
        &db,
        &files,
        &[EdgeKind::Calls],
        &extracted,
    )?;
    let cold = evaluate(revision, &labels, &observations)?;
    let changes = compare(&fresh, &cold)?;
    Ok(ProjectReport {
        binding_mode: mode,
        configured_source_gaps,
        compiler: manifest.compiler.clone(),
        selection: manifest.selection.clone(),
        compiler_diagnostics: manifest.diagnostics.len(),
        compiler_calls: manifest.calls.len(),
        unlabelled_compiler_calls: unlabelled,
        compiler_sites_not_extracted: compiler_sites.difference(&seen).count(),
        extractor_only_sites: seen.difference(&compiler_sites).count(),
        duplicate_extractor_emissions: duplicates,
        selected_files: primary.len(),
        supplied_files: inputs.len(),
        elapsed_ms: started.elapsed().as_millis(),
        fresh_declaration_kind_disagreements: declaration_kind_disagreements(&fresh),
        cold_declaration_kind_disagreements: declaration_kind_disagreements(&cold),
        fresh,
        cold,
        snapshot_changes: changes,
        gate_eligible: false,
        limitations: manifest.limitations.clone(),
    })
}
