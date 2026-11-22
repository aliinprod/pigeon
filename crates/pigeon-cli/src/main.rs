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

/// Local file name for a remote PATH when no `-o` is given: its last
/// component, made safe to create in the current directory on any OS.
/// `.`/`..` are refused (they would name a directory, or its parent); the
/// characters Windows forbids (`<>:"|?*`, controls) become `_`, so a Mac name
/// like `a:b.txt` cannot turn into an NTFS stream or a drive-relative path.
fn default_out(path: &str) -> Result<PathBuf> {
    let name = path
        .rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .filter(|s| *s != "." && *s != "..")
        .context("cannot derive a file name from PATH; use -o")?;
    let mut safe: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    // Windows silently drops trailing dots and spaces.
    while safe.ends_with(['.', ' ']) {
        safe.pop();
    }
    if safe.is_empty() {
        anyhow::bail!("cannot derive a file name from PATH; use -o");
    }
    // Reserved device names (CON, NUL, COM1, ...), with or without extension.
    let stem = safe.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        safe.insert(0, '_');
    }
    Ok(PathBuf::from(safe))
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
                None => default_out(&path)?,
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

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::{EndpointAddr, RelayUrl, SecretKey};

    #[test]
    fn relay_only_drops_direct_addresses_from_the_ticket() {
        let relay: RelayUrl = "https://relay.example.com".parse().unwrap();
        let addr = EndpointAddr::new(SecretKey::generate().public())
            .with_relay_url(relay.clone())
            .with_ip_addr("192.0.2.1:4433".parse().unwrap())
            .with_ip_addr("[2001:db8::1]:4433".parse().unwrap());
        let ticket = Ticket {
            addr,
            token: [7; 32],
        };

        let kept = prepare(ticket.clone(), false);
        assert_eq!(kept.addr.addrs.len(), 3);

        let relayed = prepare(ticket, true);
        assert_eq!(relayed.addr.addrs.len(), 1);
        assert!(relayed.addr.addrs.iter().all(|a| a.is_relay()));
        assert_eq!(relayed.token, [7; 32]);
    }

    #[test]
    fn default_out_takes_the_last_component() {
        assert_eq!(
            default_out("a/b/report.pdf").unwrap(),
            PathBuf::from("report.pdf")
        );
        assert_eq!(
            default_out(r"a\b\notes.txt").unwrap(),
            PathBuf::from("notes.txt")
        );
        assert_eq!(default_out("photos/").unwrap(), PathBuf::from("photos"));
        assert_eq!(
            default_out("Résumé ü.txt").unwrap(),
            PathBuf::from("Résumé ü.txt")
        );
    }

    #[test]
    fn default_out_refuses_dot_names() {
        for bad in ["", "/", ".", "..", "a/..", "a/./", "...", " . "] {
            assert!(default_out(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn default_out_is_safe_on_windows() {
        // ':' would make an NTFS alternate data stream or a drive-relative path.
        assert_eq!(default_out("dir/C:evil").unwrap(), PathBuf::from("C_evil"));
        assert_eq!(default_out("a:b.txt").unwrap(), PathBuf::from("a_b.txt"));
        assert_eq!(
            default_out("what?*<>|\".txt").unwrap(),
            PathBuf::from("what______.txt")
        );
        assert_eq!(default_out("tab\there").unwrap(), PathBuf::from("tab_here"));
        assert_eq!(
            default_out("trailing. ").unwrap(),
            PathBuf::from("trailing")
        );
        assert_eq!(default_out("CON").unwrap(), PathBuf::from("_CON"));
        assert_eq!(default_out("nul.txt").unwrap(), PathBuf::from("_nul.txt"));
        assert_eq!(default_out("com1.log").unwrap(), PathBuf::from("_com1.log"));
        assert_eq!(
            default_out("console.txt").unwrap(),
            PathBuf::from("console.txt")
        );
        assert_eq!(default_out("COM10").unwrap(), PathBuf::from("COM10"));
    }
}
