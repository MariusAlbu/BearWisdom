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

        /// Which conditions to run: both | bw | native (default: both).
        #[arg(long, default_value = "both")]
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
        Commands::Full { project, model, output, count, backend } => {
            std::fs::create_dir_all(&output)
                .with_context(|| format!("Failed to create output dir {}", output.display()))?;

            let tasks_path = output.join("tasks.json");
            cmd_generate(&project, &tasks_path, count)?;

            let task_set = TaskSet::load(&tasks_path)?;
            let project_root = PathBuf::from(&task_set.project_path);
            let conditions = [Condition::UseBearWisdom, Condition::NoBearWisdom];

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
        "both" => Ok(vec![Condition::UseBearWisdom, Condition::NoBearWisdom]),
        "bw" => Ok(vec![Condition::UseBearWisdom]),
        "native" => Ok(vec![Condition::NoBearWisdom]),
        other => bail!("Unknown condition '{other}': expected both | bw | native"),
    }
}
