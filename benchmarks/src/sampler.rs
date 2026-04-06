// =============================================================================
// sampler.rs  —  Generate BenchmarkTask instances from an indexed project
// =============================================================================

use anyhow::{Context, Result};
use chrono::Utc;
use std::path::Path;
use uuid::Uuid;

use bearwisdom::{
    db::Database,
    query::{
        architecture, blast_radius as blast_radius_mod, call_hierarchy, concepts as concepts_mod,
        references, symbol_info,
    },
    resolve_db_path,
};

use crate::task::{BenchmarkTask, GroundTruth, TaskCategory, TaskSet};

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Open (or create) the BearWisdom index for `project_root` and generate
/// benchmark tasks.  Indexing is triggered automatically when no index exists.
pub fn generate_tasks(project_root: &Path, count_per_category: usize) -> Result<TaskSet> {
    let db_path = resolve_db_path(project_root)
        .context("Failed to resolve DB path")?;

    // Index if the database doesn't exist yet.
    let db = if !db_path.exists() {
        tracing::info!("No index found — running full index on {}", project_root.display());
        let mut db = Database::open(&db_path).context("Failed to open database")?;
        bearwisdom::full_index(&mut db, project_root, None, None, None)
            .context("Failed to index project")?;
        db
    } else {
        Database::open(&db_path).context("Failed to open database")?
    };

    let project_str = project_root
        .to_str()
        .unwrap_or("<unknown>")
        .to_owned();

    let mut tasks: Vec<BenchmarkTask> = Vec::new();

    tasks.extend(generate_impact_analysis(&db, &project_str, count_per_category)?);
    tasks.extend(generate_call_hierarchy(&db, &project_str, count_per_category)?);
    tasks.extend(generate_architecture_overview(&db, &project_str)?);
    tasks.extend(generate_cross_file_references(&db, &project_str, count_per_category)?);
    tasks.extend(generate_concept_discovery(&db, &project_str, count_per_category)?);
    tasks.extend(generate_symbol_search(&db, &project_str, count_per_category)?);

    Ok(TaskSet::new(tasks, project_str))
}

// ---------------------------------------------------------------------------
// ImpactAnalysis
// ---------------------------------------------------------------------------

fn generate_impact_analysis(
    db: &Database,
    project_path: &str,
    count: usize,
) -> Result<Vec<BenchmarkTask>> {
    let overview = architecture::get_overview(db).context("Failed to get architecture overview")?;

    let hotspots: Vec<_> = overview
        .hotspots
        .iter()
        .take(count)
        .cloned()
        .collect();

    let mut tasks = Vec::new();

    for hotspot in &hotspots {
        let br = blast_radius_mod::blast_radius(db, &hotspot.qualified_name, 3, 500)
            .with_context(|| format!("blast_radius failed for {}", hotspot.qualified_name))?;

        let (affected_names, affected_files) = match br {
            Some(ref result) => {
                let names: Vec<String> = result
                    .affected
                    .iter()
                    .map(|a| a.qualified_name.clone())
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect();
                let files: Vec<String> = result
                    .affected
                    .iter()
                    .map(|a| a.file_path.clone())
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect();
                (names, files)
            }
            None => (vec![], vec![]),
        };

        let question = format!(
            "If I modify `{}` defined in `{}`, what other symbols would be affected? \
             List all callers and transitive dependents up to 3 hops away.",
            hotspot.name, hotspot.file_path
        );

        tasks.push(BenchmarkTask {
            id: Uuid::new_v4().to_string(),
            category: TaskCategory::ImpactAnalysis,
            question,
            target_symbol: Some(hotspot.qualified_name.clone()),
            ground_truth: GroundTruth {
                expected_items: affected_names,
                expected_files: affected_files,
                metadata: serde_json::json!({
                    "center_symbol": hotspot.qualified_name,
                    "incoming_refs": hotspot.incoming_refs,
                }),
            },
            generated_at: Utc::now(),
            project_path: project_path.to_owned(),
        });
    }

    Ok(tasks)
}

// ---------------------------------------------------------------------------
// CallHierarchy
// ---------------------------------------------------------------------------

fn generate_call_hierarchy(
    db: &Database,
    project_path: &str,
    count: usize,
) -> Result<Vec<BenchmarkTask>> {
    let overview = architecture::get_overview(db).context("Failed to get architecture overview")?;

    // Use hotspots as candidates — they have callers by definition.
    let candidates: Vec<_> = overview
        .hotspots
        .iter()
        .take(count)
        .cloned()
        .collect();

    let mut tasks = Vec::new();

    for candidate in &candidates {
        let callers =
            call_hierarchy::incoming_calls(db, &candidate.qualified_name, 50)
                .with_context(|| format!("incoming_calls failed for {}", candidate.qualified_name))?;

        if callers.is_empty() {
            continue;
        }

        let caller_names: Vec<String> = callers
            .iter()
            .map(|c| c.qualified_name.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let caller_files: Vec<String> = callers
            .iter()
            .map(|c| c.file_path.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        let question = format!(
            "What functions or methods call `{}`? List all callers with their file locations.",
            candidate.name
        );

        tasks.push(BenchmarkTask {
            id: Uuid::new_v4().to_string(),
            category: TaskCategory::CallHierarchy,
            question,
            target_symbol: Some(candidate.qualified_name.clone()),
            ground_truth: GroundTruth {
                expected_items: caller_names,
                expected_files: caller_files,
                metadata: serde_json::json!({
                    "target": candidate.qualified_name,
                    "caller_count": callers.len(),
                }),
            },
            generated_at: Utc::now(),
            project_path: project_path.to_owned(),
        });
    }

    Ok(tasks)
}

// ---------------------------------------------------------------------------
// ArchitectureOverview  (one task per project)
// ---------------------------------------------------------------------------

fn generate_architecture_overview(
    db: &Database,
    project_path: &str,
) -> Result<Vec<BenchmarkTask>> {
    let overview = architecture::get_overview(db).context("Failed to get architecture overview")?;

    let lang_names: Vec<String> = overview
        .languages
        .iter()
        .map(|l| l.language.clone())
        .collect();

    let hotspot_names: Vec<String> = overview
        .hotspots
        .iter()
        .map(|h| h.qualified_name.clone())
        .collect();

    let entry_names: Vec<String> = overview
        .entry_points
        .iter()
        .map(|e| e.qualified_name.clone())
        .collect();

    let mut expected_items = Vec::new();
    expected_items.extend(lang_names.clone());
    expected_items.extend(hotspot_names.clone());
    // Entry points can be numerous; cap at 20 for scoring purposes.
    expected_items.extend(entry_names.iter().take(20).cloned());
    expected_items.sort();
    expected_items.dedup();

    let expected_files: Vec<String> = overview
        .entry_points
        .iter()
        .take(20)
        .map(|e| e.file_path.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let task = BenchmarkTask {
        id: Uuid::new_v4().to_string(),
        category: TaskCategory::ArchitectureOverview,
        question: "Give me a high-level architectural overview of this codebase. \
                   What languages are used, what are the main entry points, \
                   and which symbols are the most depended-on hotspots?"
            .to_owned(),
        target_symbol: None,
        ground_truth: GroundTruth {
            expected_items,
            expected_files,
            metadata: serde_json::json!({
                "total_files": overview.total_files,
                "total_symbols": overview.total_symbols,
                "total_edges": overview.total_edges,
                "languages": lang_names,
                "hotspot_names": hotspot_names,
            }),
        },
        generated_at: Utc::now(),
        project_path: project_path.to_owned(),
    };

    Ok(vec![task])
}

// ---------------------------------------------------------------------------
// CrossFileReferences
// ---------------------------------------------------------------------------

fn generate_cross_file_references(
    db: &Database,
    project_path: &str,
    count: usize,
) -> Result<Vec<BenchmarkTask>> {
    // Find type-like symbols that are referenced across files.
    // Includes struct/type_alias for Go/TS, and accepts NULL visibility for TS/JS.
    let candidates: Vec<(String, String, String)> = {
        let mut stmt = db.prepare(
            "SELECT s.name, s.qualified_name, f.path
             FROM symbols s
             JOIN files f ON f.id = s.file_id
             WHERE s.kind IN ('interface', 'trait', 'class', 'struct', 'type_alias')
               AND (s.visibility = 'public' OR s.visibility IS NULL)
             ORDER BY (
                 SELECT COUNT(*) FROM edges e WHERE e.target_id = s.id
             ) DESC
             LIMIT 100",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.filter_map(|r| r.ok()).collect()
    };

    let mut tasks = Vec::new();

    for (name, qualified_name, file_path) in candidates.iter().take(count * 3) {
        let refs = references::find_references(db, qualified_name, 50)
            .with_context(|| format!("find_references failed for {qualified_name}"))?;

        if refs.is_empty() {
            continue;
        }

        let ref_symbols: Vec<String> = refs
            .iter()
            .map(|r| r.referencing_symbol.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let ref_files: Vec<String> = refs
            .iter()
            .map(|r| r.file_path.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        let question = format!(
            "Find all implementations and usages of `{}` defined in `{}`. \
             List every symbol that references or implements it.",
            name, file_path
        );

        tasks.push(BenchmarkTask {
            id: Uuid::new_v4().to_string(),
            category: TaskCategory::CrossFileReferences,
            question,
            target_symbol: Some(qualified_name.clone()),
            ground_truth: GroundTruth {
                expected_items: ref_symbols,
                expected_files: ref_files,
                metadata: serde_json::json!({
                    "target": qualified_name,
                    "reference_count": refs.len(),
                }),
            },
            generated_at: Utc::now(),
            project_path: project_path.to_owned(),
        });

        if tasks.len() >= count {
            break;
        }
    }

    Ok(tasks)
}

// ---------------------------------------------------------------------------
// ConceptDiscovery
// ---------------------------------------------------------------------------

fn generate_concept_discovery(
    db: &Database,
    project_path: &str,
    count: usize,
) -> Result<Vec<BenchmarkTask>> {
    // Run auto-discovery + assignment so concepts are populated.
    concepts_mod::discover_concepts(db).ok();
    concepts_mod::auto_assign_concepts(db).ok();

    let all_concepts = concepts_mod::list_concepts(db)?;

    // Only use concepts that have members.
    let usable: Vec<_> = all_concepts
        .iter()
        .filter(|c| c.member_count > 0)
        .take(count)
        .cloned()
        .collect();

    if usable.is_empty() {
        return Ok(vec![]);
    }

    let concept_names: Vec<String> = usable.iter().map(|c| c.name.clone()).collect();

    // For the "what are main concepts" question, expected items = concept names.
    // For each concept we also add its member symbols.
    let mut expected_items = concept_names.clone();

    for concept in &usable {
        let members = concepts_mod::concept_members(db, &concept.name, 20)
            .unwrap_or_default();
        for m in members {
            expected_items.push(m.qualified_name);
        }
    }
    expected_items.sort();
    expected_items.dedup();

    // Pick one representative concept for the per-concept question.
    let representative = &usable[0];
    let rep_members = concepts_mod::concept_members(db, &representative.name, 30)
        .unwrap_or_default();
    let rep_member_names: Vec<String> =
        rep_members.iter().map(|m| m.qualified_name.clone()).collect();

    let question = format!(
        "What are the main domain concepts in this codebase? \
         List all concepts and which symbols belong to the `{}` concept.",
        representative.name
    );

    let task = BenchmarkTask {
        id: Uuid::new_v4().to_string(),
        category: TaskCategory::ConceptDiscovery,
        question,
        target_symbol: None,
        ground_truth: GroundTruth {
            expected_items,
            expected_files: vec![],
            metadata: serde_json::json!({
                "concept_names": concept_names,
                "representative_concept": representative.name,
                "representative_members": rep_member_names,
            }),
        },
        generated_at: Utc::now(),
        project_path: project_path.to_owned(),
    };

    Ok(vec![task])
}

// ---------------------------------------------------------------------------
// SymbolSearch
// ---------------------------------------------------------------------------

fn generate_symbol_search(
    db: &Database,
    project_path: &str,
    count: usize,
) -> Result<Vec<BenchmarkTask>> {
    let overview = architecture::get_overview(db)?;

    // Use entry points as search targets — public, well-defined symbols.
    let targets: Vec<_> = overview
        .entry_points
        .iter()
        .take(count)
        .cloned()
        .collect();

    let mut tasks = Vec::new();

    for target in &targets {
        let details = symbol_info::symbol_info(db, &target.qualified_name, &bearwisdom::query::QueryOptions::full())
            .with_context(|| format!("symbol_info failed for {}", target.qualified_name))?;

        if details.is_empty() {
            continue;
        }

        let detail = &details[0];
        let question = format!(
            "Find the symbol `{}`. Where is it defined, what kind is it, \
             and what is its signature?",
            target.name
        );

        tasks.push(BenchmarkTask {
            id: Uuid::new_v4().to_string(),
            category: TaskCategory::SymbolSearch,
            question,
            target_symbol: Some(target.qualified_name.clone()),
            ground_truth: GroundTruth {
                expected_items: vec![target.qualified_name.clone()],
                expected_files: vec![target.file_path.clone()],
                metadata: serde_json::json!({
                    "qualified_name": detail.qualified_name,
                    "kind": detail.kind,
                    "file_path": detail.file_path,
                    "signature": detail.signature,
                    "start_line": detail.start_line,
                }),
            },
            generated_at: Utc::now(),
            project_path: project_path.to_owned(),
        });
    }

    Ok(tasks)
}
