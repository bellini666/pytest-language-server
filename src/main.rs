mod config;
mod fixtures;
mod providers;

use clap::{Parser, Subcommand};
use fixtures::FixtureDatabase;
use providers::Backend;

use std::path::PathBuf;
use std::sync::Arc;
use tower_lsp_server::{LspService, Server};
use tracing::info;

/// A blazingly fast Language Server Protocol implementation for pytest
#[derive(Parser)]
#[command(name = "pytest-language-server")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "A Language Server Protocol implementation for pytest", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Fixture-related commands
    Fixtures {
        #[command(subcommand)]
        command: FixtureCommands,
    },
}

#[derive(Subcommand)]
enum FixtureCommands {
    /// List all fixtures in a hierarchical tree view
    List {
        /// Path to the directory containing test files
        path: PathBuf,

        /// Skip unused fixtures from the output
        #[arg(long)]
        skip_unused: bool,

        /// Show only unused fixtures
        #[arg(long, conflicts_with = "skip_unused")]
        only_unused: bool,
    },
    /// Check for unused fixtures (exits with code 1 if found)
    Unused {
        /// Path to the directory containing test files
        path: PathBuf,

        /// Output format: "text" (default) or "json"
        #[arg(long, default_value = "text")]
        format: String,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Fixtures { command }) => match command {
            FixtureCommands::List {
                path,
                skip_unused,
                only_unused,
            } => {
                handle_fixtures_list(path, skip_unused, only_unused);
            }
            FixtureCommands::Unused { path, format } => {
                handle_fixtures_unused(path, &format);
            }
        },
        None => {
            // No subcommand provided - start LSP server
            start_lsp_server().await;
        }
    }
}

fn handle_fixtures_list(path: PathBuf, skip_unused: bool, only_unused: bool) {
    // Convert to absolute path
    let absolute_path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(&path)
    };

    if !absolute_path.exists() {
        eprintln!("Error: Path does not exist: {}", absolute_path.display());
        std::process::exit(1);
    }

    if !absolute_path.is_dir() {
        eprintln!(
            "Error: Path is not a directory: {}",
            absolute_path.display()
        );
        std::process::exit(1);
    }

    // Canonicalize the path to resolve symlinks and relative components
    let canonical_path = absolute_path.canonicalize().unwrap_or(absolute_path);

    // Create a fixture database and scan the directory
    let fixture_db = FixtureDatabase::new();
    fixture_db.scan_workspace(&canonical_path);

    // Print the tree
    fixture_db.print_fixtures_tree(&canonical_path, skip_unused, only_unused);
}

fn handle_fixtures_unused(path: PathBuf, format: &str) {
    use colored::Colorize;

    // Convert to absolute path
    let absolute_path = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(&path)
    };

    if !absolute_path.exists() {
        eprintln!("Error: Path does not exist: {}", absolute_path.display());
        std::process::exit(1);
    }

    if !absolute_path.is_dir() {
        eprintln!(
            "Error: Path is not a directory: {}",
            absolute_path.display()
        );
        std::process::exit(1);
    }

    // Canonicalize the path to resolve symlinks and relative components
    let canonical_path = absolute_path.canonicalize().unwrap_or(absolute_path);

    // Create a fixture database and scan the directory
    let fixture_db = FixtureDatabase::new();
    fixture_db.scan_workspace(&canonical_path);

    // Get unused fixtures
    let unused = fixture_db.get_unused_fixtures();

    if unused.is_empty() {
        if format == "json" {
            println!("[]");
        } else {
            println!("{}", "No unused fixtures found.".green());
        }
        std::process::exit(0);
    }

    // Output in requested format
    if format == "json" {
        let json_output: Vec<serde_json::Value> = unused
            .iter()
            .map(|(file_path, fixture_name)| {
                let relative_path = file_path
                    .strip_prefix(&canonical_path)
                    .unwrap_or(file_path)
                    .to_string_lossy()
                    .to_string();
                serde_json::json!({
                    "file": relative_path,
                    "fixture": fixture_name
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&json_output).unwrap());
    } else {
        println!(
            "{} {} unused fixture(s):\n",
            "Found".red().bold(),
            unused.len()
        );

        for (file_path, fixture_name) in &unused {
            let relative_path = file_path
                .strip_prefix(&canonical_path)
                .unwrap_or(file_path)
                .to_string_lossy();
            println!(
                "  {} {} in {}",
                "•".red(),
                fixture_name.yellow(),
                relative_path.dimmed()
            );
        }

        println!(
            "\n{}",
            "Tip: Remove unused fixtures or add tests that use them.".dimmed()
        );
    }

    // Exit with code 1 to signal unused fixtures found (useful for CI)
    std::process::exit(1);
}

async fn start_lsp_server() {
    // Set up stderr logging with env-filter support
    // Users can control verbosity with RUST_LOG env var:
    // RUST_LOG=debug pytest-language-server
    // RUST_LOG=info pytest-language-server
    // RUST_LOG=warn pytest-language-server (default)
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    info!("pytest-language-server starting");

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let fixture_db = Arc::new(FixtureDatabase::new());

    let (service, socket) = LspService::new(|client| Backend::new(client, fixture_db.clone()));

    info!("LSP server ready");
    Server::new(stdin, stdout, socket).serve(service).await;
    // Note: serve() typically won't return - process exit is handled by shutdown()
}
