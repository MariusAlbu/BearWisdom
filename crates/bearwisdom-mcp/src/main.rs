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
    // Fail fast on panics so the pipeline can't hang silently — see
    // bearwisdom::panic_hook for the full rationale.
    bearwisdom::install_fail_fast_panic_hook();

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

    // BW core owns indexing. The MCP server is just a query consumer:
    // it opens an `IndexService`, hands the pool to the request handler,
    // and lets the service's file watcher keep the index fresh.
    let db_path = resolve_db_path(&project)?;
    let index_service = std::sync::Arc::new(
        bearwisdom::IndexService::open(
            &db_path,
            &project,
            bearwisdom::IndexServiceOptions::default(),
        )
        .with_context(|| format!("open index service for {}", project.display()))?,
    );

    // Start MCP server FIRST so we respond to `initialize` immediately.
    let mcp_server = server::BearWisdomServer::new(index_service.pool().clone(), project.clone());
    eprintln!("MCP server ready — listening on stdio");

    let transport = rmcp::transport::io::stdio();
    let service = mcp_server.serve(transport).await?;

    // Initial reindex runs on a background blocking task so it doesn't
    // delay tool calls. The watcher started in `IndexService::open` keeps
    // the DB fresh after this initial pass.
    let bg_service = index_service.clone();
    let _bg_handle = tokio::task::spawn_blocking(move || match bg_service.reindex_now() {
        Ok(bearwisdom::ReindexStats::Full(stats)) => {
            eprintln!(
                "Full index: {} files, {} symbols, {} edges ({:.2}s){}",
                stats.file_count,
                stats.symbol_count,
                stats.edge_count,
                stats.duration_ms as f64 / 1000.0,
                if stats.files_with_errors > 0 {
                    format!(", {} with errors", stats.files_with_errors)
                } else {
                    String::new()
                },
            );
        }
        Ok(bearwisdom::ReindexStats::Incremental(inc)) => {
            eprintln!(
                "Incremental reindex: +{} added, ~{} modified, -{} deleted, {} unchanged ({:.2}s)",
                inc.files_added,
                inc.files_modified,
                inc.files_deleted,
                inc.files_unchanged,
                inc.duration_ms as f64 / 1000.0,
            );
        }
        Err(e) => eprintln!("Index error: {e:#}"),
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
