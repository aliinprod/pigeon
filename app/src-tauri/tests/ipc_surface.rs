//! The webview's reach into the backend stays exactly what the security
//! review approved (SEC-40, SEC-41; Warden review of PR #19):
//!
//! - one capability file, for the `main` window only, no remote origins;
//! - it allows the app's own commands one by one and nothing else: no
//!   `core:*`, no plugin permissions (`dialog:*`, `opener:*`, `fs:*`, ...);
//! - the commands declared in `build.rs`, allowed in the capability and
//!   registered in `main.rs` are the same set (a new command cannot slip in
//!   without a permission, or a permission linger without its command);
//! - a strict CSP: scripts and styles from the app only, no `unsafe-*`, IPC
//!   as the only connection, prototype frozen;
//! - the UI loads nothing from the network.
//!
//! Pure file checks: no window, no Tauri runtime.

use std::{collections::BTreeSet, path::PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn json(rel: &str) -> serde_json::Value {
    serde_json::from_str(&read(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// The quoted names inside the first `[ ... ]` after `marker`.
fn quoted_list(src: &str, marker: &str) -> BTreeSet<String> {
    let start = src.find(marker).unwrap_or_else(|| panic!("no {marker}")) + marker.len();
    let rest = &src[start..];
    let end = rest.find(']').expect("unterminated list");
    rest[..end]
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

/// The bare identifiers inside `generate_handler![ ... ]`.
fn handler_list(src: &str) -> BTreeSet<String> {
    let marker = "generate_handler![";
    let start = src.find(marker).expect("no generate_handler!") + marker.len();
    let rest = &src[start..];
    let end = rest.find(']').expect("unterminated generate_handler!");
    rest[..end]
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[test]
fn capability_allows_only_the_apps_own_commands() {
    let caps: Vec<_> = std::fs::read_dir(root().join("capabilities"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(caps, ["main.json"], "one capability file only");

    let cap = json("capabilities/main.json");
    assert_eq!(cap["windows"], serde_json::json!(["main"]));
    assert!(
        cap.get("remote").is_none(),
        "no remote origins may call IPC"
    );
    assert!(cap.get("webviews").is_none(), "no extra webviews");
    assert!(cap.get("platforms").is_none(), "same rules on every OS");

    let perms: BTreeSet<String> = cap["permissions"]
        .as_array()
        .expect("permissions list")
        .iter()
        .map(|p| {
            p.as_str()
                .unwrap_or_else(|| panic!("scoped permission objects are not allowed: {p}"))
                .to_string()
        })
        .collect();
    for p in &perms {
        assert!(
            !p.contains(':'),
            "{p}: no core or plugin permission (the picker and opener run in Rust)"
        );
        assert!(p.starts_with("allow-"), "{p}: only allow-<command>");
    }

    let declared = quoted_list(&read("build.rs"), ".commands(&[");
    let registered = handler_list(&read("src/main.rs"));
    let allowed: BTreeSet<String> = perms
        .iter()
        .map(|p| p.trim_start_matches("allow-").replace('-', "_"))
        .collect();
    assert_eq!(declared, registered, "build.rs vs generate_handler!");
    assert_eq!(declared, allowed, "build.rs vs capabilities/main.json");
}

#[test]
fn csp_is_strict_and_the_ui_loads_nothing_remote() {
    let conf = json("tauri.conf.json");
    let sec = &conf["app"]["security"];
    let csp = sec["csp"].as_str().expect("a CSP string");
    let directive = |name: &str| -> Vec<String> {
        csp.split(';')
            .map(str::trim)
            .find(|d| d.split_whitespace().next() == Some(name))
            .unwrap_or_else(|| panic!("CSP has no {name}"))
            .split_whitespace()
            .skip(1)
            .map(str::to_string)
            .collect()
    };
    assert!(
        !csp.contains("unsafe-"),
        "no unsafe-inline / unsafe-eval: {csp}"
    );
    assert_eq!(directive("default-src"), ["'self'"]);
    assert_eq!(directive("script-src"), ["'self'"]);
    assert_eq!(directive("style-src"), ["'self'"]);
    assert_eq!(directive("object-src"), ["'none'"]);
    assert_eq!(directive("base-uri"), ["'none'"]);
    assert_eq!(directive("frame-ancestors"), ["'none'"]);
    assert_eq!(directive("connect-src"), ["ipc:", "http://ipc.localhost"]);
    assert_eq!(sec["freezePrototype"], true);
    assert!(sec.get("dangerousDisableAssetCspModification").is_none());
    assert!(
        sec.get("assetProtocol").is_none_or(|a| a["enable"] != true),
        "no asset protocol: the webview never reads local files"
    );
    for w in conf["app"]["windows"].as_array().unwrap() {
        assert!(
            w.get("url")
                .is_none_or(|u| !u.as_str().unwrap_or("").contains("://"))
        );
    }

    for f in ["../ui/index.html", "../ui/app.js", "../ui/app.css"] {
        let src = read(f);
        // No remote resources, no inline scripts (the CSP would block them
        // anyway; keep them out so nobody relaxes the CSP to make them work).
        for bad in ["http://", "https://", "<script>", " onclick=", " onload="] {
            assert!(!src.contains(bad), "{f} contains {bad:?}");
        }
    }
}
