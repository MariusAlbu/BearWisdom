// =============================================================================
// bw-bench  —  BearWisdom utility benchmark
//
// Measures the utility of BearWisdom MCP tools versus native Claude Code
// tooling (Read / Grep / Glob / ListDir) on real code analysis tasks.
//
// USAGE
// -----
//   bw-bench generate --project <PATH> [--output tasks.json] [--count 3]
//       Sample tasks from an indexed project and write them to a JSON file.
//
//   bw-bench run --tasks <FILE> --model <MODEL> --output <DIR>
//               [--conditions both|bw|native]
//       Execute all tasks in a task file via the Claude API and write results.
//       Requires ANTHROPIC_API_KEY in the environment.
//
//   bw-bench report --results <DIR> [--output report.md]
//       Score persisted results and generate a Markdown + JSON report.
//
//   bw-bench full --project <PATH> --model <MODEL> --output <DIR> [--count 3]
//       generate → run (both conditions) → report in one command.
// =============================================================================

extern crate sqlite_vec;

mod report;
mod runner;
mod sampler;
mod scorer;
mod task;

use anyhow::{bail, Context, Result};
use bearwisdom::db::Database;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing::info;

use runner::{Condition, Runner};
use task::TaskSet;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(
    name = "bw-bench",
    version,
    about = "BearWisdom utility benchmark — BW tools vs native tooling"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate benchmark tasks from an indexed project.
    Generate {
        /// Path to the project root to index and sample tasks from.
        #[arg(long)]
        project: PathBuf,

        /// Output path for the generated task set (JSON).
        #[arg(long, default_value = "tasks.json")]
        output: PathBuf,

        /// Number of tasks to generate per category.
        #[arg(long, default_value_t = 3)]
        count: usize,
    },

    /// Execute tasks and write per-run result files.
    Run {
        /// Path to a tasks.json file produced by `generate`.
        #[arg(long)]
        tasks: PathBuf,

        /// Claude model ID (e.g. sonnet, opus, or full model name).
        #[arg(long, default_value = "sonnet")]
        model: String,

        /// Output directory where result JSON files will be written.
        #[arg(long)]
        output: PathBuf,

        /// Which conditions to run: all | both | bw | bw-cli | native (default: all).
        #[arg(long, default_value = "all")]
        conditions: String,

        /// Backend to use: "api" (requires ANTHROPIC_API_KEY) or "cli" (uses `claude` CLI from subscription).
        #[arg(long, default_value = "cli")]
        backend: String,
    },

    /// Score persisted results and generate a Markdown + JSON report.
    Report {
        /// Directory containing result JSON files from `run`.
        #[arg(long)]
        results: PathBuf,

        /// Output path for the Markdown report (default: report.md in results dir).
        #[arg(long)]
        output: Option<PathBuf>,
    },

    /// Diagnose a project's index to check benchmark readiness.
    /// Reports visibility/kind distributions, sampler-relevant counts, and red flags.
    Diagnose {
        /// Path to the project root (must already be indexed).
        #[arg(long)]
        project: PathBuf,
    },

    /// Run generate → run (both conditions) → report in one shot.
    Full {
        /// Path to the project root to index and sample tasks from.
        #[arg(long)]
        project: PathBuf,

        /// Claude model ID.
        #[arg(long, default_value = "sonnet")]
        model: String,

        /// Output directory (tasks.json + results + report will be written here).
        #[arg(long)]
        output: PathBuf,

        /// Number of tasks to generate per category.
        #[arg(long, default_value_t = 3)]
        count: usize,

        /// Backend: "cli" (Claude Code subscription) or "api" (ANTHROPIC_API_KEY).
        #[arg(long, default_value = "cli")]
        backend: String,
    },
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Generate { project, output, count } => {
            cmd_generate(&project, &output, count)?;
        }
        Commands::Run { tasks, model, output, conditions, backend } => {
            let conditions = parse_conditions(&conditions)?;
            let task_set = TaskSet::load(&tasks)?;
            let project_root = PathBuf::from(&task_set.project_path);

            match backend.as_str() {
                "cli" => {
                    let cli_runner = runner::CliRunner::new(model, project_root)?;
                    cli_runner.run_all(&task_set, &conditions, &output).await?;
                }
                "api" => {
                    let api_key = api_key()?;
                    let runner = Runner::new(api_key, model, project_root)?;
                    runner.run_all(&task_set, &conditions, &output).await?;
                }
                other => bail!("Unknown backend '{other}': expected cli | api"),
            }
        }
        Commands::Report { results, output } => {
            let output_path = output.unwrap_or_else(|| results.join("report.md"));
            cmd_report(&results, &output_path)?;
        }
        Commands::Diagnose { project } => {
            cmd_diagnose(&project)?;
        }
        Commands::Full { project, model, output, count, backend } => {
            std::fs::create_dir_all(&output)
                .with_context(|| format!("Failed to create output dir {}", output.display()))?;

            let tasks_path = output.join("tasks.json");
            cmd_generate(&project, &tasks_path, count)?;

            let task_set = TaskSet::load(&tasks_path)?;
            let project_root = PathBuf::from(&task_set.project_path);
            let conditions = Condition::all().to_vec();

            match backend.as_str() {
                "cli" => {
                    let cli_runner = runner::CliRunner::new(model, project_root)?;
                    cli_runner.run_all(&task_set, &conditions, &output).await?;
                }
                "api" => {
                    let api_key = api_key()?;
                    let runner = Runner::new(api_key, model, project_root)?;
                    runner.run_all(&task_set, &conditions, &output).await?;
                }
                other => bail!("Unknown backend '{other}': expected cli | api"),
            }

            let report_path = output.join("report.md");
            cmd_report(&output, &report_path)?;

            info!("Report written to {}", report_path.display());
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Command implementations
// ---------------------------------------------------------------------------

fn cmd_generate(project: &std::path::Path, output: &std::path::Path, count: usize) -> Result<()> {
    info!("Generating tasks from project: {}", project.display());
    let task_set = sampler::generate_tasks(project, count)
        .context("Failed to generate tasks")?;
    info!("Generated {} tasks total", task_set.tasks.len());
    task_set.save(output)?;
    info!("Task set written to {}", output.display());
    Ok(())
}

fn cmd_diagnose(project: &std::path::Path) -> Result<()> {
    let db_path = bearwisdom::resolve_db_path(project)?;
    if !db_path.exists() {
        bail!("No index found at {}. Run `bw open` first.", db_path.display());
    }
    let db = Database::open(&db_path)?;

    // 1. Basic stats
    let (total_files, total_symbols, total_edges): (i64, i64, i64) = db.conn.query_row(
        "SELECT
            (SELECT COUNT(*) FROM files),
            (SELECT COUNT(*) FROM symbols),
            (SELECT COUNT(*) FROM edges)",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    println!("=== {} ===", project.display());
    println!("  Files: {total_files}  Symbols: {total_symbols}  Edges: {total_edges}");

    // 2. Visibility distribution
    println!("\n  Visibility distribution:");
    let mut stmt = db.conn.prepare(
        "SELECT COALESCE(visibility, 'NULL'), COUNT(*) FROM symbols GROUP BY visibility ORDER BY COUNT(*) DESC"
    )?;
    let mut rows = Vec::new();
    let mut query_rows = stmt.query([])?;
    while let Some(row) = query_rows.next()? {
        let vis: String = row.get(0)?;
        let count: i64 = row.get(1)?;
        rows.push((vis, count));
    }
    for (vis, count) in &rows {
        println!("    {vis}: {count}");
    }

    // 3. Kind distribution
    println!("\n  Kind distribution (top 15):");
    let mut stmt = db.conn.prepare(
        "SELECT kind, COUNT(*) FROM symbols GROUP BY kind ORDER BY COUNT(*) DESC LIMIT 15"
    )?;
    let mut rows = Vec::new();
    let mut query_rows = stmt.query([])?;
    while let Some(row) = query_rows.next()? {
        let kind: String = row.get(0)?;
        let count: i64 = row.get(1)?;
        rows.push((kind, count));
    }
    for (kind, count) in &rows {
        println!("    {kind}: {count}");
    }

    // 4. Sampler-critical: type-like symbols by kind and visibility
    println!("\n  Type-like symbols (interface/trait/class/struct/type_alias):");
    let mut stmt = db.conn.prepare(
        "SELECT kind, COALESCE(visibility, 'NULL'), COUNT(*)
         FROM symbols
         WHERE kind IN ('interface', 'trait', 'class', 'struct', 'type_alias')
         GROUP BY kind, visibility
         ORDER BY kind, COUNT(*) DESC"
    )?;
    let mut rows = Vec::new();
    let mut query_rows = stmt.query([])?;
    while let Some(row) = query_rows.next()? {
        let kind: String = row.get(0)?;
        let vis: String = row.get(1)?;
        let count: i64 = row.get(2)?;
        rows.push((kind, vis, count));
    }
    for (kind, vis, count) in &rows {
        println!("    {kind} [{vis}]: {count}");
    }

    // 5. Sampler query simulation: what the current sampler would find
    let current_sampler_count: i64 = db.conn.query_row(
        "SELECT COUNT(*) FROM symbols
         WHERE kind IN ('interface', 'trait', 'class')
           AND visibility = 'public'",
        [],
        |row| row.get(0),
    )?;
    println!("\n  Current sampler CrossFileRef candidates (interface/trait/class + public): {current_sampler_count}");

    // What the fixed sampler would find
    let fixed_sampler_count: i64 = db.conn.query_row(
        "SELECT COUNT(*) FROM symbols
         WHERE kind IN ('interface', 'trait', 'class', 'struct', 'type_alias')
           AND (visibility = 'public' OR visibility IS NULL)",
        [],
        |row| row.get(0),
    )?;
    println!("  Fixed sampler CrossFileRef candidates (+ struct/type_alias, + NULL vis): {fixed_sampler_count}");

    // Of those, how many have incoming edges (actually referenced)?
    let referenced_count: i64 = db.conn.query_row(
        "SELECT COUNT(*) FROM symbols s
         WHERE s.kind IN ('interface', 'trait', 'class', 'struct', 'type_alias')
           AND (s.visibility = 'public' OR s.visibility IS NULL)
           AND EXISTS (SELECT 1 FROM edges e WHERE e.target_id = s.id)",
        [],
        |row| row.get(0),
    )?;
    println!("  Of those, referenced (has incoming edges): {referenced_count}");

    // 6. Architecture overview viability
    let hotspot_count: i64 = db.conn.query_row(
        "SELECT COUNT(*) FROM (
            SELECT s.id FROM symbols s
            JOIN edges e ON e.target_id = s.id
            GROUP BY s.id
            HAVING COUNT(*) >= 3
         )",
        [],
        |row| row.get(0),
    )?;
    println!("\n  Symbols with >=3 incoming edges (hotspot candidates): {hotspot_count}");

    // 7. find_references viability: pick top 3 referenced symbols and test
    println!("\n  Top 5 most-referenced symbols:");
    let mut stmt = db.conn.prepare(
        "SELECT s.qualified_name, s.kind, COALESCE(s.visibility, 'NULL'), COUNT(*) as ref_count
         FROM symbols s
         JOIN edges e ON e.target_id = s.id
         GROUP BY s.id
         ORDER BY ref_count DESC
         LIMIT 5"
    )?;
    let mut rows = Vec::new();
    let mut query_rows = stmt.query([])?;
    while let Some(row) = query_rows.next()? {
        let qname: String = row.get(0)?;
        let kind: String = row.get(1)?;
        let vis: String = row.get(2)?;
        let refs: i64 = row.get(3)?;
        rows.push((qname, kind, vis, refs));
    }
    for (qname, kind, vis, refs) in &rows {
        println!("    {qname} ({kind}, {vis}): {refs} refs");
    }

    // 8. Verdict
    let mut issues: Vec<String> = Vec::new();
    if current_sampler_count == 0 {
        issues.push(format!(
            "CrossFileReferences: current sampler finds 0 candidates (fixed sampler would find {fixed_sampler_count})"
        ));
    }
    if hotspot_count < 3 {
        issues.push(format!("Only {hotspot_count} hotspot candidates — ImpactAnalysis/CallHierarchy tasks may be weak"));
    }
    if total_edges == 0 {
        issues.push("No edges at all — index is empty or broken".to_owned());
    }

    if issues.is_empty() {
        println!("\n  VERDICT: READY for benchmarking");
    } else {
        println!("\n  VERDICT: ISSUES FOUND");
        for issue in &issues {
            println!("    - {issue}");
        }
    }

    println!();
    Ok(())
}

fn cmd_report(results_dir: &std::path::Path, output_path: &std::path::Path) -> Result<()> {
    info!("Loading results from {}", results_dir.display());

    let results = runner::load_results(results_dir)?;
    if results.is_empty() {
        bail!(
            "No result files found in {}. Run `bw-bench run` first.",
            results_dir.display()
        );
    }
    info!("Loaded {} run results", results.len());

    // Load the task set from the same directory (tasks.json placed there by `full`).
    let tasks_path = results_dir.join("tasks.json");
    let tasks: Vec<task::BenchmarkTask> = if tasks_path.exists() {
        TaskSet::load(&tasks_path)?.tasks
    } else {
        // No tasks.json: reconstruct minimal tasks from result task_ids so scoring still works.
        // The missing ground truth will yield zero recall — acceptable for reporting runs
        // where the tasks file has been separated from the results.
        tracing::warn!(
            "tasks.json not found in {}; ground truth will be empty for all tasks.",
            results_dir.display()
        );
        vec![]
    };

    report::generate_report(&tasks, &results, output_path)?;
    info!("Report written to {}", output_path.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn api_key() -> Result<String> {
    std::env::var("ANTHROPIC_API_KEY")
        .context("ANTHROPIC_API_KEY environment variable is not set")
}

fn parse_conditions(s: &str) -> Result<Vec<Condition>> {
    match s {
        "all" => Ok(Condition::all().to_vec()),
        "both" => Ok(vec![Condition::UseBearWisdom, Condition::NoBearWisdom]),
        "bw" => Ok(vec![Condition::UseBearWisdom]),
        "bw-cli" => Ok(vec![Condition::UseBearWisdomCli]),
        "native" => Ok(vec![Condition::NoBearWisdom]),
        other => bail!("Unknown condition '{other}': expected all | both | bw | bw-cli | native"),
    }
}
