extern crate sqlite_vec;

mod compact;
mod register;
mod server;
mod services;

#[cfg(test)]
#[path = "services_tests.rs"]
mod services_tests;

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

    // A stdio MCP server is a query client, not a project index daemon. More
    // than one editor/agent commonly starts one, so giving every process a
    // watcher and an initial sweep races writers and can expose a rebuilding
    // index. This process reads the shared index and its last-complete
    // metadata; a persistent project writer (CLI/watch or a future daemon)
    // owns refreshes.
    let db_path = resolve_db_path(&project)?;
    let default_options = bearwisdom::IndexServiceOptions {
        watch: false,
        allow_refresh: false,
        ..Default::default()
    };
    let default_service = std::sync::Arc::new(
        bearwisdom::IndexService::open(&db_path, &project, default_options.clone())
            .with_context(|| format!("open index service for {}", project.display()))?,
    );

    let services = std::sync::Arc::new(services::ServiceCache::new(10, default_options));
    services.insert(project.clone(), default_service.clone());

    // Start MCP server FIRST so we respond to `initialize` immediately.
    let mcp_server = server::BearWisdomServer::new(project.clone(), services.clone());
    eprintln!("MCP server ready — listening on stdio");

    let transport = rmcp::transport::io::stdio();
    let service = mcp_server.serve(transport).await?;

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
