use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod cli;
mod daemon;

#[derive(Parser, Debug)]
#[command(name = "rg-tree-sitter")]
#[command(about = "Lightweight AST-aware symbol search powered by tree-sitter")]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Find symbol definitions
    Define(cli::SearchArgs),
    /// Find symbol references
    Refs(cli::SearchArgs),
    /// Filter external rg output (read from stdin)
    Filter {
        #[arg(long, short)]
        lang: String,
        #[arg(long, value_enum, default_value = "plain")]
        format: cli::OutputFormat,
        #[arg(long)]
        socket: Option<PathBuf>,
    },
    /// Start daemon (Phase 2)
    Daemon {
        #[arg(long)]
        socket: PathBuf,
        #[arg(long, short)]
        dir: PathBuf,
        #[arg(long)]
        watch: bool,
    },
    /// Check daemon status
    DaemonStatus {
        #[arg(long)]
        socket: PathBuf,
    },
    /// Stop daemon
    DaemonStop {
        #[arg(long)]
        socket: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    match &args.command {
        Commands::Define(search_args) => cli::run_define(search_args)?,
        Commands::Refs(search_args) => cli::run_refs(search_args)?,
        Commands::Filter { lang, format, socket } => cli::run_filter(lang, *format, socket.as_ref())?,
        Commands::Daemon {
            socket,
            dir,
            watch,
        } => daemon::run_daemon(socket, dir, *watch).await?,
        Commands::DaemonStatus { socket } => daemon::run_daemon_status(socket)?,
        Commands::DaemonStop { socket } => daemon::run_daemon_stop(socket)?,
    }

    Ok(())
}
