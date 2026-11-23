//! `pigeon`: fetch files from a paired Pigeon agent.
//!
//! - `pigeon pair <INVITE-LINK> [--name NAME]`: pair once with an agent; asks
//!   for the 4-word code shown on the agent's screen.
//! - `pigeon ls [PATH]`, `pigeon get <PATH> [-o OUT]`: use the paired agent
//!   (`--agent NAME` if there are several). No ticket, nothing secret on the
//!   command line: this device is recognised by its key.
//! - `pigeon agents`, `pigeon forget <NAME>`.
//!
//! The pairing code is a secret, so it is never a command-line argument
//! (shell history, process list, clap error echo; TH-14). It is read from
//! `--code-file FILE`, the `PIGEON_PAIR_CODE` environment variable, or typed
//! at the prompt, in that order.
//!
//! `--relay-only` disables direct (UDP hole-punched) paths, so every byte goes
//! through the relay. Useful for testing the fallback path.

use std::{
    io::{BufRead, IsTerminal, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use iroh::{Endpoint, endpoint::presets};
use pigeon_core::{
    client,
    config::Home,
    devices::{AGENTS_FILE, AgentList, PairedAgent},
    display_safe, fingerprint,
    keystore::CLIENT_KEY,
    pair::{self, InviteLink, PairingCode},
};

/// Environment variable that may hold the pairing code.
const CODE_ENV: &str = "PIGEON_PAIR_CODE";

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
    /// Pair this device with an agent, using the invite link from
    /// `pigeon-agent pair` and the code shown on the agent's screen.
    Pair {
        /// The invite link (`pigeon-invite-...`). Not secret.
        link: InviteLink,
        /// What to call this agent here, e.g. "Office Mac".
        #[arg(long)]
        name: Option<String>,
        /// Read the pairing code from this file (otherwise $PIGEON_PAIR_CODE,
        /// otherwise you are asked to type it).
        #[arg(long, value_name = "FILE")]
        code_file: Option<PathBuf>,
    },
    /// List a folder on the agent.
    Ls {
        #[arg(default_value = "")]
        path: String,
        /// Which paired agent (name or fingerprint), if there are several.
        #[arg(long)]
        agent: Option<String>,
    },
    /// Download one file (resumes from OUT.part if present). Never overwrites:
    /// if OUT exists, saves as "OUT (1)", "OUT (2)", ...
    Get {
        path: String,
        /// Output file (default: the file name from PATH, in the current folder).
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Which paired agent (name or fingerprint), if there are several.
        #[arg(long)]
        agent: Option<String>,
    },
    /// List the agents this device is paired with.
    Agents,
    /// Forget a paired agent on this device (revoke it on the agent too).
    Forget { agent: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let home = Home::from_env()?;
    let agents_path = home.dir.join(AGENTS_FILE);
    match cli.command {
        Command::Agents => return agents_cmd(&agents_path),
        Command::Forget { agent } => return forget_cmd(&agents_path, &agent),
        _ => {}
    }

    // Read the code before going online, so a typo fails fast and locally.
    let code = match &cli.command {
        Command::Pair { code_file, .. } => Some(PairingCode::parse(&read_secret(
            "pairing code",
            code_file.as_deref(),
            CODE_ENV,
        )?)?),
        _ => None,
    };

    // A stable device identity: the agent's allow-list holds its public key.
    let (key, _store, _created) = home.device_key(CLIENT_KEY)?;
    let mut builder = Endpoint::builder(presets::N0).secret_key(key);
    if cli.relay_only {
        builder = builder.clear_ip_transports();
    }
    let endpoint = builder.bind().await?;
    let result = run(&endpoint, &agents_path, cli.command, code).await;
    endpoint.close().await;
    result
}

/// Reads a secret (`what`) from `file`, else the environment variable `env`,
/// else one line on stdin (with a prompt on a terminal). The value is never
/// echoed in errors.
fn read_secret(what: &str, file: Option<&Path>, env: &str) -> Result<String> {
    let text = if let Some(file) = file {
        std::fs::read_to_string(file)
            .with_context(|| format!("could not read the {what} from {}", file.display()))?
    } else if let Ok(v) = std::env::var(env) {
        v
    } else {
        let stdin = std::io::stdin();
        if stdin.is_terminal() {
            eprint!("Type the {what} shown on the agent and press Enter: ");
            std::io::stderr().flush()?;
        }
        let mut line = String::new();
        stdin
            .lock()
            .read_line(&mut line)
            .with_context(|| format!("could not read the {what} from stdin"))?;
        line
    };
    let text = text.trim().to_string();
    anyhow::ensure!(!text.is_empty(), "no {what} given");
    Ok(text)
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

async fn run(
    endpoint: &Endpoint,
    agents_path: &Path,
    command: Command,
    code: Option<PairingCode>,
) -> Result<()> {
    match command {
        Command::Pair { link, name, .. } => {
            let code = code.context("no pairing code")?;
            let paired = pair::pair(endpoint, &link, &code).await?;
            let fp = fingerprint(&paired.agent);
            let agent = PairedAgent {
                id: paired.agent,
                name: name
                    .map(|n| n.trim().to_string())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| format!("agent {fp}")),
                relay_url: link.addr.relay_urls().next().cloned(),
                paired_at: pair::now_secs(),
            };
            let mut list = AgentList::load(agents_path)?;
            list.upsert(agent.clone());
            list.save(agents_path)?;
            println!(
                "paired with {} (fingerprint {fp}). Try `pigeon ls`.",
                display_safe(&agent.name)
            );
            println!(
                "this device's fingerprint is {}; the agent lists it in `pigeon-agent devices`.",
                fingerprint(&endpoint.id())
            );
        }
        Command::Ls { path, agent } => {
            let agent = AgentList::load(agents_path)?
                .pick(agent.as_deref())?
                .clone();
            for e in client::list(endpoint, &agent.dial_addr(), &path).await? {
                let kind = if e.is_dir { "dir " } else { "file" };
                // Names come from the agent: never let them drive the terminal.
                let name = display_safe(&e.name);
                println!("{kind}  {:>14}  {:>10}  {name}", e.size, e.mtime);
            }
        }
        Command::Get { path, out, agent } => {
            let agent = AgentList::load(agents_path)?
                .pick(agent.as_deref())?
                .clone();
            let out = match out {
                Some(out) => out,
                None => default_out(&path)?,
            };
            let out = client::free_path(&out);
            let stats = client::get(endpoint, &agent.dial_addr(), &path, &out).await?;
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
        Command::Agents | Command::Forget { .. } => unreachable!("handled before going online"),
    }
    Ok(())
}

fn agents_cmd(path: &Path) -> Result<()> {
    let list = AgentList::load(path)?;
    if list.agents.is_empty() {
        println!("not paired with any agent yet: run `pigeon pair <INVITE-LINK>`");
    }
    for a in &list.agents {
        println!("{}  {}", fingerprint(&a.id), display_safe(&a.name));
    }
    Ok(())
}

fn forget_cmd(path: &Path, query: &str) -> Result<()> {
    let mut list = AgentList::load(path)?;
    let gone = list.remove(query)?;
    list.save(path)?;
    println!(
        "forgot {} ({}). To cut it off on the agent side too, run `pigeon-agent revoke` there.",
        display_safe(&gone.name),
        fingerprint(&gone.id)
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_is_read_from_a_file_and_never_echoed() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("code.txt");
        std::fs::write(&file, "  Acid ACORN acre-zoom \r\n").unwrap();
        let text = read_secret("pairing code", Some(&file), "PIGEON_TEST_UNSET_VAR").unwrap();
        assert_eq!(
            PairingCode::parse(&text).unwrap().reveal(),
            "acid-acorn-acre-zoom"
        );
        // A code with a typo must not show up in the error.
        std::fs::write(&file, "acid acorn acre zzsecretzz").unwrap();
        let text = read_secret("pairing code", Some(&file), "X").unwrap();
        let err = format!("{:#}", PairingCode::parse(&text).unwrap_err());
        assert!(!err.contains("zzsecretzz"), "{err}");
        // A missing file names the file, not any secret.
        let err = read_secret("pairing code", Some(&dir.path().join("nope")), "X").unwrap_err();
        assert!(format!("{err:#}").contains("could not read the pairing code"));
    }

    #[test]
    fn secrets_are_not_command_line_arguments() {
        // Only the (non-secret) link is an argument; the code has no flag that
        // takes its value, and `ls`/`get` take no ticket any more.
        let link = pigeon_core::pair::InviteLink {
            addr: iroh::EndpointAddr::new(iroh::SecretKey::from_bytes(&[1; 32]).public()),
            invite_id: [0; 16],
            expires_at: 0,
        }
        .to_string();
        let cli = Cli::try_parse_from(["pigeon", "pair", &link, "--code-file", "c.txt"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Pair {
                code_file: Some(_),
                ..
            }
        ));
        assert!(Cli::try_parse_from(["pigeon", "pair", &link, "--code", "acid"]).is_err());
        let cli = Cli::try_parse_from(["pigeon", "ls", "pigeonabc"]).unwrap();
        assert!(matches!(cli.command, Command::Ls { path, .. } if path == "pigeonabc"));
        // A bad link is reported without echoing it back.
        let err = Cli::try_parse_from(["pigeon", "pair", "pigeon-invite-zzsecretzz"])
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("invite link"), "{err}");
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
