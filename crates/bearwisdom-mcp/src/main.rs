extern crate sqlite_vec;

mod compact;
mod register;
mod server;

use anyhow::{Context, Result};
use clap::Parser;
use rmcp::ServiceExt;
use std::path::PathBuf;
use tracing::info;

#[derive(Parser, Debug)]
#[command(name = "bw-mcp")]
#[command(about = "BearWisdom code intelligence MCP server")]
struct Cli {
    /// Path to the project root to index
    #[arg(long)]
    project: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(clap::Subcommand, Debug)]
enum Commands {
    /// Register this MCP server in .claude/settings.local.json
    Register {
        /// Path to the project root
        #[arg(long)]
        project: PathBuf,
    },
    /// Unregister this MCP server from .claude/settings.local.json
    Unregister {
        /// Path to the project root
        #[arg(long)]
        project: PathBuf,
    },
}

/// Resolve the database path for a project root: `<project>/.bearwisdom/index.db`.
fn resolve_db_path(project_root: &std::path::Path) -> Result<PathBuf> {
    bearwisdom::resolve_db_path(project_root)
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing — output to stderr only (stdout reserved for MCP JSON-RPC)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    rayon::ThreadPoolBuilder::new()
        .stack_size(8 * 1024 * 1024)
        .build_global()
        .ok();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Register { project }) => {
            let project = project.canonicalize().unwrap_or(project);
            register::register(&project)
        }
        Some(Commands::Unregister { project }) => {
            let project = project.canonicalize().unwrap_or(project);
            register::unregister(&project)
        }
        None => run_server(cli.project).await,
    }
}

async fn run_server(project_arg: Option<PathBuf>) -> Result<()> {
    let project = project_arg.unwrap_or_else(|| PathBuf::from("."));
    let project = project.canonicalize().unwrap_or(project);
    info!("Starting BearWisdom for project: {}", project.display());

    // Open or create the index database via a connection pool (4 connections).
    // Cache and metrics are enabled by default in the core library.
    let db_path = resolve_db_path(&project)?;
    let pool = bearwisdom::DbPool::new(&db_path, 4)
        .with_context(|| format!("Failed to create pool for {}", db_path.display()))?;

    // Start MCP server FIRST so we respond to initialize immediately.
    // Indexing runs in background — tool calls use separate pool connections
    // so they no longer block during indexing.
    let mcp_server = server::BearWisdomServer::new(pool.clone(), project.clone());
    eprintln!("MCP server ready — listening on stdio");

    let transport = rmcp::transport::io::stdio();
    let service = mcp_server.serve(transport).await?;

    // Index in background. Fresh DB → full index; existing DB with an
    // `indexed_commit` → git-aware incremental reindex so MCP startup
    // stays fast on the hot path. Falls back to HashDiff if git is
    // unavailable or the indexed commit isn't reachable.
    let bg_pool = pool.clone();
    let bg_project = project.clone();
    let _bg_handle = tokio::task::spawn(async move {
        let idx_pool = bg_pool;
        let idx_project = bg_project;
        let idx_ref_cache = idx_pool.ref_cache().clone();
        let index_result = tokio::task::spawn_blocking(move || -> anyhow::Result<String> {
            let mut db = idx_pool.get()
                .map_err(|e| anyhow::anyhow!("pool get failed: {e}"))?;
            let prior_commit = bearwisdom::indexer::changeset::get_meta(&db, "indexed_commit");
            if prior_commit.is_some() {
                let inc = bearwisdom::git_reindex(&mut db, &idx_project, Some(&idx_ref_cache))?;
                return Ok(format!(
                    "Git-incremental reindex: +{} added, ~{} modified, -{} deleted, {} unchanged ({:.2}s)",
                    inc.files_added, inc.files_modified, inc.files_deleted, inc.files_unchanged,
                    inc.duration_ms as f64 / 1000.0,
                ));
            }
            let file_count: i64 = db
                .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
                .unwrap_or(0);
            if file_count > 0 {
                let inc = bearwisdom::incremental_index(&mut db, &idx_project, Some(&idx_ref_cache))?;
                return Ok(format!(
                    "Hash-incremental reindex: +{} added, ~{} modified, -{} deleted, {} unchanged ({:.2}s)",
                    inc.files_added, inc.files_modified, inc.files_deleted, inc.files_unchanged,
                    inc.duration_ms as f64 / 1000.0,
                ));
            }
            let stats = bearwisdom::full_index(&mut db, &idx_project, None, None, Some(&idx_ref_cache))?;
            Ok(format!(
                "Full index: {} files, {} symbols, {} edges ({:.2}s){}",
                stats.file_count, stats.symbol_count, stats.edge_count,
                stats.duration_ms as f64 / 1000.0,
                if stats.files_with_errors > 0 {
                    format!(", {} with errors", stats.files_with_errors)
                } else {
                    String::new()
                },
            ))
        })
        .await;

        match index_result {
            Ok(Ok(msg)) => eprintln!("{msg}"),
            Ok(Err(e)) => eprintln!("Index error: {e}"),
            Err(e) => eprintln!("Index task panicked: {e}"),
        }
    });

    tokio::select! {
        result = service.waiting() => {
            if let Err(e) = result {
                eprintln!("MCP transport error: {e}");
            }
        }
        _ = tokio::signal::ctrl_c() => {
            eprintln!("Received shutdown signal");
        }
    }

    eprintln!("BearWisdom MCP server shut down");
    Ok(())
}
