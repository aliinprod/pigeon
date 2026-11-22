//! `pigeon`: fetch files from a Pigeon agent.
//!
//! - `pigeon ls <TICKET> [PATH]`
//! - `pigeon get <TICKET> <PATH> [-o OUT]`
//!
//! `--relay-only` disables direct (UDP hole-punched) paths, so every byte goes
//! through the relay. Useful for testing the fallback path.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use iroh::{Endpoint, endpoint::presets};
use pigeon_core::{Ticket, client};

#[derive(Parser)]
#[command(name = "pigeon", version, about = "Your files know the way home.")]
struct Cli {
    /// Never use a direct connection; send everything through the relay.
    #[arg(long, global = true)]
    relay_only: bool,
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
    /// Download one file (resumes from OUT.part if present). Never overwrites:
    /// if OUT exists, saves as "OUT (1)", "OUT (2)", ...
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
    let mut builder = Endpoint::builder(presets::N0);
    if cli.relay_only {
        builder = builder.clear_ip_transports();
    }
    let endpoint = builder.bind().await?;
    let result = run(&endpoint, cli.command, cli.relay_only).await;
    endpoint.close().await;
    result
}

/// In relay-only mode, drop the agent's direct addresses from the ticket so
/// we only ever dial its relay.
fn prepare(mut ticket: Ticket, relay_only: bool) -> Ticket {
    if relay_only {
        ticket.addr.addrs.retain(|a| a.is_relay());
    }
    ticket
}

async fn run(endpoint: &Endpoint, command: Command, relay_only: bool) -> Result<()> {
    match command {
        Command::Ls { ticket, path } => {
            let ticket = prepare(ticket, relay_only);
            for e in client::list(endpoint, &ticket, &path).await? {
                let kind = if e.is_dir { "dir " } else { "file" };
                println!("{kind}  {:>14}  {:>10}  {}", e.size, e.mtime, e.name);
            }
        }
        Command::Get { ticket, path, out } => {
            let ticket = prepare(ticket, relay_only);
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
            let out = client::free_path(&out);
            let stats = client::get(endpoint, &ticket, &path, &out).await?;
            if stats.resumed_from > 0 {
                println!("resumed from byte {}", stats.resumed_from);
            }
            println!(
                "saved {}: {} bytes in {:.2}s ({:.3} MB/s, {}), sha256 {} OK",
                out.display(),
                stats.size,
                stats.elapsed.as_secs_f64(),
                stats.mb_per_sec(),
                stats.route,
                stats.sha256
            );
        }
    }
    Ok(())
}
