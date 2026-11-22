//! `pigeon`: fetch files from a Pigeon agent.
//!
//! - `pigeon ls <TICKET> [PATH]`
//! - `pigeon get <TICKET> <PATH> [-o OUT]`

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use iroh::{Endpoint, endpoint::presets};
use pigeon_core::{Ticket, client};

#[derive(Parser)]
#[command(name = "pigeon", version, about = "Your files know the way home.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List a folder on the agent.
    Ls {
        ticket: Ticket,
        #[arg(default_value = "")]
        path: String,
    },
    /// Download one file (resumes from OUT.part if present).
    Get {
        ticket: Ticket,
        path: String,
        /// Output file (default: the file name from PATH, in the current folder).
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let endpoint = Endpoint::bind(presets::N0).await?;
    let result = run(&endpoint, cli.command).await;
    endpoint.close().await;
    result
}

async fn run(endpoint: &Endpoint, command: Command) -> Result<()> {
    match command {
        Command::Ls { ticket, path } => {
            for e in client::list(endpoint, &ticket, &path).await? {
                let kind = if e.is_dir { "dir " } else { "file" };
                println!("{kind}  {:>14}  {:>10}  {}", e.size, e.mtime, e.name);
            }
        }
        Command::Get { ticket, path, out } => {
            let out = match out {
                Some(out) => out,
                None => {
                    let name = path
                        .rsplit(['/', '\\'])
                        .find(|s| !s.is_empty())
                        .context("cannot derive a file name from PATH; use -o")?;
                    PathBuf::from(name)
                }
            };
            let stats = client::get(endpoint, &ticket, &path, &out).await?;
            if stats.resumed_from > 0 {
                println!("resumed from byte {}", stats.resumed_from);
            }
            println!(
                "saved {}: {} bytes in {:.2}s ({:.1} MB/s), sha256 {} OK",
                out.display(),
                stats.size,
                stats.elapsed.as_secs_f64(),
                stats.mb_per_sec(),
                stats.sha256
            );
        }
    }
    Ok(())
}
