//! Test-only label ingestion. Markers are removed before parsing, and targets
//! are authored by numeric marker ID before the engine runs.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};

use super::*;
use crate::indexer::parse_file::parse_file_with_arena;
use crate::indexer::resolve::engine::{compilation::Compilation, pipeline::resolve_from_tree};
use crate::indexer::write::write_parsed_files_with_origin;
use crate::type_checker::core::types::TypeArena;
use crate::walker::WalkedFile;

pub(super) struct Fixture<'a> {
    pub path: &'a str,
    pub language: &'static str,
    pub marked_source: &'a str,
}

struct MarkedSource {
    source: String,
    refs: BTreeMap<u32, ReferenceSite>,
    declarations: BTreeMap<u32, DeclarationSite>,
}

fn strip_markers(file: FixtureFileId, input: &str) -> Result<MarkedSource> {
    let mut output = MarkedSource {
        source: String::new(),
        refs: BTreeMap::new(),
        declarations: BTreeMap::new(),
    };
    let mut rest = input;
    while let Some(start) = rest.find("/*@") {
        output.source.push_str(&rest[..start]);
        rest = &rest[start + 3..];
        let end = rest.find("*/").context("Unclosed oracle marker")?;
        let fields: Vec<_> = rest[..end].split(':').collect();
        ensure!(fields.len() >= 2, "Invalid oracle marker");
        let id = fields[1].parse::<u32>()?;
        match fields[0] {
            "ref" => {
                ensure!(fields.len() == 2, "Call markers require one numeric ID");
                let site = ReferenceSite {
                    file,
                    byte_offset: output.source.len().try_into()?,
                    kind: EdgeKind::Calls,
                };
                ensure!(
                    output.refs.insert(id, site).is_none(),
                    "Duplicate reference marker"
                );
            }
            "decl" => {
                ensure!(
                    fields.len() == 3,
                    "Declaration markers require a syntactic kind"
                );
                let line = output
                    .source
                    .bytes()
                    .filter(|b| *b == b'\n')
                    .count()
                    .try_into()?;
                let col = output
                    .source
                    .rsplit('\n')
                    .next()
                    .unwrap()
                    .len()
                    .try_into()?;
                let site = DeclarationSite {
                    file,
                    line,
                    col,
                    kind: fields[2].parse()?,
                };
                ensure!(
                    output.declarations.insert(id, site).is_none(),
                    "Duplicate declaration marker"
                );
            }
            other => anyhow::bail!("Unknown oracle marker: {other}"),
        }
        rest = &rest[end + 2..];
    }
    output.source.push_str(rest);
    Ok(output)
}

/// Marker IDs are corpus-global. None means a deliberately unbound call.
/// Every call marker must have an independently supplied label.
pub(super) fn run(
    fixtures: &[Fixture<'_>],
    targets: &[(u32, Option<u32>)],
) -> Result<OracleReport> {
    run_with_anchors(fixtures, targets, false, false, &[])
}

/// Labels are authored at the called identifier, not the whole expression.
/// This is a distinct versioned cohort; existing v1 snapshots stay unchanged.
pub(super) fn run_selectors(
    fixtures: &[Fixture<'_>],
    targets: &[(u32, Option<u32>)],
) -> Result<OracleReport> {
    run_with_anchors(fixtures, targets, true, false, &[])
}

/// Same independent labels after restoring the declaration/type snapshot.
pub(super) fn run_selectors_cold(
    fixtures: &[Fixture<'_>],
    targets: &[(u32, Option<u32>)],
) -> Result<OracleReport> {
    run_with_anchors(fixtures, targets, true, true, &[])
}

/// Configuration is test input, fingerprinted with source and ground truth.
pub(super) fn run_configured_selectors(
    fixtures: &[Fixture<'_>],
    targets: &[(u32, Option<u32>)],
    configuration: &[(&str, &str)],
    cold: bool,
) -> Result<OracleReport> {
    ensure!(
        !configuration.is_empty(),
        "Configured cohort needs configuration inputs"
    );
    run_with_anchors(fixtures, targets, true, cold, configuration)
}

fn run_with_anchors(
    fixtures: &[Fixture<'_>],
    targets: &[(u32, Option<u32>)],
    selectors: bool,
    cold: bool,
    configuration: &[(&str, &str)],
) -> Result<OracleReport> {
    run_with_mutation(fixtures, targets, selectors, cold, configuration, |_| {})
}

/// Mutations change engine input after capture, never the independent labels.
pub(super) fn run_with_mutation(
    fixtures: &[Fixture<'_>],
    targets: &[(u32, Option<u32>)],
    selectors: bool,
    cold: bool,
    configuration: &[(&str, &str)],
    mutate: impl FnMut(&mut crate::types::ParsedFile),
) -> Result<OracleReport> {
    run_inputs(
        fixtures,
        targets,
        selectors,
        cold,
        configuration,
        false,
        mutate,
    )
}

/// Explicit fixture membership is oracle input, never inferred by the engine.
pub(super) fn run_program_selectors(
    fixtures: &[Fixture<'_>],
    targets: &[(u32, Option<u32>)],
    cold: bool,
    mutate: impl FnMut(&mut crate::types::ParsedFile),
) -> Result<OracleReport> {
    run_inputs(fixtures, targets, true, cold, &[], true, mutate)
}

fn run_inputs(
    fixtures: &[Fixture<'_>],
    targets: &[(u32, Option<u32>)],
    selectors: bool,
    cold: bool,
    configuration: &[(&str, &str)],
    program: bool,
    mut mutate: impl FnMut(&mut crate::types::ParsedFile),
) -> Result<OracleReport> {
    let mut refs = BTreeMap::new();
    let mut declarations = BTreeMap::new();
    let mut sources = Vec::new();
    for (index, fixture) in fixtures.iter().enumerate() {
        let file = FixtureFileId((index + 1).try_into()?);
        let marked = strip_markers(file, fixture.marked_source)?;
        for (id, site) in marked.refs {
            ensure!(
                refs.insert(id, site).is_none(),
                "Duplicate corpus reference marker"
            );
        }
        for (id, site) in marked.declarations {
            ensure!(
                declarations.insert(id, site).is_none(),
                "Duplicate corpus declaration marker"
            );
        }
        sources.push(marked.source);
    }
    // Ground truth is finalized before parsing or examining any engine output.
    let mut expected = Vec::new();
    for &(reference, target) in targets {
        expected.push(ExpectedReference {
            site: *refs.get(&reference).context("Unknown reference marker")?,
            target: target
                .map(|id| {
                    declarations
                        .get(&id)
                        .copied()
                        .context("Unknown declaration marker")
                })
                .transpose()?,
        });
    }
    ensure!(
        expected.len() == refs.len(),
        "Every marked reference must be labelled"
    );
    let manifest: Vec<_> = fixtures
        .iter()
        .zip(&sources)
        .map(|(fixture, source)| (fixture.path, fixture.language, source))
        .collect();
    // Includes the cohort definition, file order, content and independent labels.
    let cohort = if selectors {
        "call-selectors-v2"
    } else {
        "calls-v1"
    };
    let fingerprint = if program {
        serde_json::to_vec(&("explicit-program-call-selectors-v1", manifest, &expected))?
    } else if configuration.is_empty() {
        serde_json::to_vec(&(cohort, manifest, &expected))?
    } else {
        serde_json::to_vec(&(
            "configured-call-selectors-v1",
            manifest,
            configuration,
            &expected,
        ))?
    };
    let digest = Sha256::digest(fingerprint);
    let revision = CorpusRevision(digest.into());

    let dir = tempfile::tempdir()?;
    let mut paths = HashSet::new();
    for path in fixtures
        .iter()
        .map(|f| f.path)
        .chain(configuration.iter().map(|f| f.0))
    {
        let path = std::path::Path::new(path);
        ensure!(
            !path.as_os_str().is_empty()
                && path
                    .components()
                    .all(|p| matches!(p, std::path::Component::Normal(_))),
            "Fixture path must stay within the temporary workspace"
        );
        ensure!(
            paths.insert(path.to_path_buf()),
            "Duplicate source/configuration path"
        );
    }
    for &(path, source) in configuration {
        let absolute = dir.path().join(path);
        std::fs::create_dir_all(absolute.parent().context("Configuration parent")?)?;
        std::fs::write(absolute, source)?;
    }
    let arena = Arc::new(TypeArena::new());
    let registry = crate::languages::default_registry();
    let mut parsed = Vec::new();
    for (fixture, source) in fixtures.iter().zip(&sources) {
        let absolute_path = dir.path().join(fixture.path);
        if let Some(parent) = absolute_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&absolute_path, source)?;
        parsed.push(parse_file_with_arena(
            &WalkedFile {
                relative_path: fixture.path.to_owned(),
                absolute_path,
                language: fixture.language,
            },
            &registry,
            &arena,
        )?);
    }
    for file in &mut parsed {
        mutate(file);
    }
    let mut db = crate::Database::open_in_memory()?;
    let (file_rows, symbol_ids) =
        write_parsed_files_with_origin(&db, &parsed, "internal", Some(&arena))?;
    let mut private_rows = HashSet::new();
    for file in &parsed {
        if let Some(graph) = &file.flow.lexical {
            for binding in &graph.lexical_only {
                if let Some(id) = graph
                    .symbol_slots
                    .get(binding)
                    .copied()
                    .flatten()
                    .and_then(|slot| symbol_ids.row_id(&file.path, slot))
                {
                    private_rows.insert(id);
                }
            }
        }
    }
    ensure!(
        private_rows
            == crate::db::lexical_visibility::read(db.conn())?
                .into_iter()
                .collect(),
        "Persisted lexical-only identities differ from exact parsed slots"
    );
    let mut files = HashMap::new();
    let mut extracted = Vec::new();
    let mut extracted_sites = HashSet::new();
    for (index, file) in parsed.iter().enumerate() {
        let fixture_file = FixtureFileId((index + 1).try_into()?);
        files.insert(file_rows[&file.path], fixture_file);
        for reference in file.refs.iter().filter(|r| r.kind == EdgeKind::Calls) {
            let site = ReferenceSite {
                file: fixture_file,
                byte_offset: if selectors {
                    reference
                        .chain
                        .as_ref()
                        .and_then(|c| c.segments.last())
                        .map(|s| s.byte_offset)
                        .unwrap_or(reference.byte_offset)
                } else {
                    reference.byte_offset
                },
                kind: reference.kind,
            };
            // The oracle scores physical source sites, not duplicate extractor
            // emissions. The separate raw census retains those duplicates.
            // Multiple surviving log outcomes at a site still fail closed.
            if extracted_sites.insert(site) {
                extracted.push(site);
            }
        }
    }
    let mut context = (!configuration.is_empty())
        .then(|| crate::indexer::project_context::build_project_context(dir.path()));
    if program {
        use crate::indexer::programs::{Program, ProgramSource, SourceScope};
        context.get_or_insert_with(Default::default).programs = Some(vec![Program {
            key: "oracle".into(),
            fingerprint: format!("{digest:x}"),
            complete: true,
            callable_policy: None,
            compiler_intrinsics: None,
            source_binding_order: None,
            sources: parsed
                .iter()
                .map(|file| ProgramSource {
                    path: file.path.clone(),
                    content_hash: file.content_hash.clone(),
                    scope: SourceScope::Syntax,
                })
                .collect(),
        }]);
    }
    let tree = Compilation::build_with_context(
        &parsed,
        &symbol_ids,
        Arc::clone(&arena),
        context.as_ref(),
        &Default::default(),
    );
    let tree = if cold {
        tree.persist_type_info(db.conn())?;
        let restored = Arc::new(TypeArena::new());
        let snapshot: String = db.conn().query_row(
            "SELECT value FROM _bearwisdom_meta WHERE key='type_arena_snapshot'",
            [],
            |r| r.get(0),
        )?;
        restored.restore_snapshot(&snapshot);
        let mut cold = Compilation::build(
            &[],
            &crate::indexer::symbol_ids::SymbolIds::default(),
            restored,
        );
        cold.ingest_from_db(db.conn());
        cold
    } else {
        tree
    };
    resolve_from_tree(&mut db, tree, &parsed, &symbol_ids, context.as_ref())?;
    let read = if selectors {
        crate::query::oracle_evidence::selector_observations
    } else {
        crate::query::oracle_evidence::observations
    };
    let observed = read(&db, &files, &[EdgeKind::Calls], &extracted)?;
    evaluate(revision, &expected, &observed)
}

#[cfg(test)]
#[path = "fixture_support_tests.rs"]
mod tests;
