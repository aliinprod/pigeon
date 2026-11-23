//! `pigeon-agent share <DIR>`: shares one folder (read-only) over iroh.
//!
//! The agent only connects outward (to the iroh relay); it does not open any
//! listening socket of its own beyond the UDP sockets iroh uses.

use std::{path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use iroh::{Endpoint, endpoint::presets};
use pigeon_core::{
    ALPN, Ticket,
    agent::{ServeOptions, Share, serve},
};

#[derive(Parser)]
#[command(name = "pigeon-agent", version, about = "Share a folder with Pigeon")]
struct Cli {
    /// Print details of failed connection attempts.
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Share DIR (read-only) and print a ticket for the client.
    Share { dir: PathBuf },
}

#[tokio::main]
async fn main() -> Result<()> {
    let Cli {
        verbose,
        command: Command::Share { dir },
    } = Cli::parse();
    let root = dir
        .canonicalize()
        .with_context(|| format!("folder not found: {}", dir.display()))?;
    anyhow::ensure!(root.is_dir(), "not a folder: {}", root.display());

    let endpoint = Endpoint::builder(presets::N0)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await?;

    // Wait until we have a home relay so the ticket is dialable from anywhere.
    if tokio::time::timeout(Duration::from_secs(15), endpoint.online())
        .await
        .is_err()
    {
        eprintln!("warning: relay not reachable yet; the ticket may only work on this network");
    }

    let ticket = Ticket {
        addr: endpoint.addr(),
        token: pigeon_core::new_token()?,
    };
    println!("ticket: {ticket}");
    eprintln!("sharing {} (Ctrl+C to stop)", root.display());

    let share = Share {
        root,
        token: ticket.token,
    };
    tokio::select! {
        res = serve(endpoint.clone(), share, ServeOptions { verbose, ..Default::default() }) => res?,
        _ = tokio::signal::ctrl_c() => eprintln!("stopping"),
    }
    endpoint.close().await;
    Ok(())
}
