extern crate sqlite_vec;

mod register;
mod server;

use anyhow::{Context, Result};
use clap::Parser;
use rmcp::ServiceExt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
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

    // Open or create the index database
    let db_path = resolve_db_path(&project)?;
    let db = bearwisdom::Database::open_with_vec(&db_path)
        .with_context(|| format!("Failed to open database at {}", db_path.display()))?;
    let db = Arc::new(Mutex::new(db));

    // Start MCP server FIRST so we respond to initialize immediately.
    // Indexing runs in background — tool calls during indexing will block
    // on the mutex until the current batch finishes, which is acceptable.
    let mcp_server = server::BearWisdomServer::new(db.clone(), project.clone());
    eprintln!("MCP server ready — listening on stdio");

    let transport = rmcp::transport::io::stdio();
    let service = mcp_server.serve(transport).await?;

    // Full index in background
    let bg_db = db.clone();
    let bg_project = project.clone();
    let _bg_handle = tokio::task::spawn(async move {
        let idx_db = bg_db;
        let idx_project = bg_project;
        // Index first (hold lock briefly), then release lock and embed separately.
        let index_result = tokio::task::spawn_blocking(move || {
            let mut db = idx_db.lock().expect("index lock");
            bearwisdom::full_index(&mut db, &idx_project, None, None)
        })
        .await;

        match index_result {
            Ok(Ok(stats)) => {
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
                    }
                );
            }
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
