//! `pigeon`: fetch files from a paired Pigeon agent.
//!
//! - `pigeon pair <INVITE-LINK> [--name NAME]`: pair once with an agent; asks
//!   for the 4-word code shown on the agent's screen.
//! - `pigeon ls [PATH]`, `pigeon get <PATH> [-o OUT]`: use the paired agent
//!   (`--agent NAME` if there are several). No ticket, nothing secret on the
//!   command line: this device is recognised by its key.
//! - `pigeon agents`, `pigeon forget <NAME>`.
//! - `pigeon whoami [--name NAME]`: this computer's name and fingerprint.
//!   One key per computer: `pigeon-agent` on the same computer uses the same
//!   one (PAIRING.md D9).
//!
//! The pairing code is a secret, so it is never a command-line argument
//! (shell history, process list, clap error echo; TH-14). It is read from
//! `--code-file FILE`, the `PIGEON_PAIR_CODE` environment variable, or typed
//! at the prompt, in that order.
//!
//! `ls --relay-only` / `get --relay-only` disable direct (UDP hole-punched)
//! paths, so every byte goes through the relay. Useful for testing the
//! fallback path.

use std::{
    io::{BufRead, IsTerminal, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use iroh::Endpoint;
use pigeon_core::{
    client,
    config::Home,
    devices::{AGENTS_FILE, AgentList, PairedAgent, trusted_relay_hint},
    display_safe, fingerprint,
    identity::{self, Identity},
    keystore::SecretStore,
    pair::{self, InviteLink, PairingCode},
};
use zeroize::Zeroizing;

/// Environment variable that may hold the pairing code.
const CODE_ENV: &str = "PIGEON_PAIR_CODE";

#[derive(Parser)]
#[command(name = "pigeon", version, about = "Your files know the way home.")]
struct Cli {
    /// Show the full chain of causes when something fails.
    #[arg(short, long, global = true)]
    verbose: bool,
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
        /// Folder inside the share, with `/` between parts ("" = the top).
        #[arg(default_value = "")]
        path: String,
        /// Which paired agent (name or fingerprint), if there are several.
        #[arg(long)]
        agent: Option<String>,
        /// Never use a direct connection; send everything through the relay.
        #[arg(long)]
        relay_only: bool,
    },
    /// Download one file (resumes from OUT.part if present). Never overwrites:
    /// if OUT exists, saves as "OUT (1)", "OUT (2)", ...
    Get {
        /// File inside the share, with `/` between parts, as `ls` shows it.
        path: String,
        /// Output file (default: the file name from PATH, in the current folder).
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Which paired agent (name or fingerprint), if there are several.
        #[arg(long)]
        agent: Option<String>,
        /// Never use a direct connection; send everything through the relay.
        #[arg(long)]
        relay_only: bool,
    },
    /// List the agents this device is paired with.
    Agents,
    /// Forget an agent on this device. To cut this device off on the agent
    /// as well, run `pigeon-agent revoke` there.
    Forget {
        /// Agent name or fingerprint, as shown by `pigeon agents`.
        agent: String,
    },
    /// Show this computer's name and fingerprint (one key for fetching and
    /// sharing; `pigeon-agent whoami` shows the same).
    Whoami {
        /// Rename this computer, e.g. "Home PC".
        #[arg(long)]
        name: Option<String>,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let verbose = cli.verbose;
    match real_main(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}", render_error(&e, verbose));
            ExitCode::FAILURE
        }
    }
}

/// N3: by default one line plus the root cause (once); `-v` shows every cause.
fn render_error(e: &anyhow::Error, verbose: bool) -> String {
    if verbose {
        return format!("Error: {e:?}");
    }
    let top = e.to_string();
    let root = e.root_cause().to_string();
    if e.chain().count() > 1 && root != top {
        format!("Error: {top}\n  cause: {root}\n  (run with -v for details)")
    } else {
        format!("Error: {top}")
    }
}

async fn real_main(cli: Cli) -> Result<()> {
    let home = Home::from_env()?;
    let agents_path = home.dir.join(AGENTS_FILE);
    match cli.command {
        Command::Agents => return agents_cmd(&home),
        Command::Forget { agent } => return forget_cmd(&home, &agent),
        Command::Whoami { name } => return whoami_cmd(&home, name.as_deref()),
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
    let relay_only = matches!(
        cli.command,
        Command::Ls {
            relay_only: true,
            ..
        } | Command::Get {
            relay_only: true,
            ..
        }
    );

    // A stable device identity, the same one `pigeon-agent` uses here: the
    // agent's allow-list holds its public key.
    let (me, store) = identity(&home)?;
    // Usually the device key; for an agent paired before one-key-per-device,
    // the old client key that agent knows (PAIRING.md D9).
    let (key, target) = match &cli.command {
        Command::Ls { agent, .. } | Command::Get { agent, .. } => {
            let target = AgentList::load(&agents_path)?
                .pick(agent.as_deref())?
                .clone();
            (me.key_for(store.as_ref(), &target)?, Some(target))
        }
        _ => (me.key.clone(), None),
    };
    let mut builder = client::endpoint_builder(key);
    if relay_only {
        builder = builder.clear_ip_transports();
    }
    let endpoint = builder.bind().await?;
    let result = run(&endpoint, &home, &me, target, cli.command, code).await;
    endpoint.close().await;
    result
}

/// This computer's identity (created or migrated on first use); prints what
/// the migration did, once.
fn identity(home: &Home) -> Result<(Identity, Box<dyn SecretStore>)> {
    let (me, store) = home.identity()?;
    for note in &me.notes {
        eprintln!("{note}");
    }
    Ok((me, store))
}

/// Reads a secret (`what`) from `file`, else the environment variable `env`,
/// else one line on stdin (with a prompt on a terminal). The value is never
/// echoed in errors, and every copy we make is zeroed when dropped (W-7).
fn read_secret(what: &str, file: Option<&Path>, env: &str) -> Result<Zeroizing<String>> {
    let text = Zeroizing::new(if let Some(file) = file {
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
        let mut line = Zeroizing::new(String::new());
        stdin
            .lock()
            .read_line(&mut line)
            .with_context(|| format!("could not read the {what} from stdin"))?;
        std::mem::take(&mut *line)
    });
    let text = Zeroizing::new(text.trim().to_string());
    anyhow::ensure!(
        !text.is_empty(),
        "no {what} given: type it at the prompt, or use --code-file FILE or ${env}"
    );
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
    home: &Home,
    me: &Identity,
    target: Option<PairedAgent>,
    command: Command,
    code: Option<PairingCode>,
) -> Result<()> {
    let agents_path = home.dir.join(AGENTS_FILE);
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
                // W-6: keep only a default (n0) relay as a hint.
                relay_url: trusted_relay_hint(link.addr.relay_urls().next()),
                paired_at: pair::now_secs(),
                // Paired with the device key.
                legacy_key: None,
            };
            let replaced = AgentList::update(&agents_path, |list| {
                let old = list.agents.iter().find(|a| a.id == agent.id).cloned();
                list.upsert(agent.clone());
                Ok(old)
            })?;
            println!(
                "paired with {} (fingerprint {fp}). Try `pigeon ls`.",
                display_safe(&agent.name)
            );
            println!(
                "this computer is {}; the agent lists it in `pigeon-agent devices`.",
                me.summary()
            );
            if let Some(old) = replaced.and_then(|a| a.legacy_key) {
                println!(
                    "This computer used its old key {old_fp} with that agent before. To remove the \
                     old entry there, run `pigeon-agent revoke {old_fp}` on the agent.",
                    old_fp = fingerprint(&old)
                );
                // Drops the old client key once no remembered agent needs it.
                identity(home)?;
            }
        }
        Command::Ls { path, .. } => {
            let agent = target.context("no agent chosen")?;
            for e in client::list(endpoint, &agent.dial_addr(), &path).await? {
                let kind = if e.is_dir { "dir " } else { "file" };
                // Names come from the agent: never let them drive the terminal.
                let name = display_safe(&e.name);
                println!("{kind}  {:>14}  {:>10}  {name}", e.size, e.mtime);
            }
        }
        Command::Get { path, out, .. } => {
            let agent = target.context("no agent chosen")?;
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
        Command::Agents | Command::Forget { .. } | Command::Whoami { .. } => {
            unreachable!("handled before going online")
        }
    }
    Ok(())
}

fn agents_cmd(home: &Home) -> Result<()> {
    let (me, _) = identity(home)?;
    println!("this computer: {}", me.summary());
    let list = AgentList::load(&home.dir.join(AGENTS_FILE))?;
    if list.agents.is_empty() {
        println!("not paired with any agent yet: run `pigeon pair <INVITE-LINK>`");
    }
    for a in &list.agents {
        let via = match a.legacy_key {
            Some(old) if old != me.id() => format!(
                "  (paired with this computer's old key {}; pair again to switch)",
                fingerprint(&old)
            ),
            _ => String::new(),
        };
        println!("{}  {}{via}", fingerprint(&a.id), display_safe(&a.name));
    }
    Ok(())
}

fn forget_cmd(home: &Home, query: &str) -> Result<()> {
    let gone = AgentList::update(&home.dir.join(AGENTS_FILE), |list| list.remove(query))?;
    println!(
        "forgot {} ({}). To cut it off on the agent side too, run `pigeon-agent revoke` there.",
        display_safe(&gone.name),
        fingerprint(&gone.id)
    );
    if gone.legacy_key.is_some() {
        // Drops the old client key once no remembered agent needs it.
        identity(home)?;
    }
    Ok(())
}

fn whoami_cmd(home: &Home, name: Option<&str>) -> Result<()> {
    let (mut me, store) = identity(home)?;
    if let Some(name) = name {
        me.name = identity::set_name(&home.dir, name)?;
        eprintln!("renamed this computer to {}", display_safe(&me.name));
    }
    let agents = AgentList::load(&home.dir.join(AGENTS_FILE))?;
    print!("{}", me.whoami(store.as_ref(), &agents));
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

    /// N7: `--relay-only` exists only where it does something.
    #[test]
    fn relay_only_is_only_on_ls_and_get() {
        let cli = Cli::try_parse_from(["pigeon", "get", "a.txt", "--relay-only"]).unwrap();
        assert!(matches!(
            cli.command,
            Command::Get {
                relay_only: true,
                ..
            }
        ));
        assert!(Cli::try_parse_from(["pigeon", "ls", "--relay-only"]).is_ok());
        assert!(Cli::try_parse_from(["pigeon", "agents", "--relay-only"]).is_err());
        assert!(Cli::try_parse_from(["pigeon", "forget", "x", "--relay-only"]).is_err());
    }

    /// N3: one friendly line plus the root cause by default; everything with -v.
    #[test]
    fn errors_are_short_unless_verbose() {
        let e = anyhow::anyhow!("error 120: peer doesn't support any known protocol")
            .context("aborted by peer")
            .context("could not pair: make a new invite");
        let short = render_error(&e, false);
        assert!(short.starts_with("Error: could not pair"), "{short}");
        assert_eq!(short.matches("error 120").count(), 1, "{short}");
        assert!(!short.contains("aborted by peer"), "{short}");
        let long = render_error(&e, true);
        assert!(
            long.contains("aborted by peer") && long.contains("Caused by"),
            "{long}"
        );
        let single = render_error(&anyhow::anyhow!("not paired"), false);
        assert_eq!(single, "Error: not paired");
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
