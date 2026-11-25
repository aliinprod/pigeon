//! Headless end to end for the app backend: two complete backends ("Work
//! Mac" and "Home PC") in one process, each with ONE iroh endpoint that it
//! uses for both roles (R14 exit criterion). Loopback only, no relay.
//!
//! Flow: A shares a folder and makes an invite; B pastes the link, types a
//! wrong code (costs a try), then the right one; B browses and gets a file
//! (SHA-256 checked against the source, never overwrites); then the reverse
//! direction on the same two endpoints; then A revokes B.

use std::{path::Path, sync::Arc, time::Duration};

use pigeon_app::{Backend, InviteView, Options, PairOutcome, TransferState};
use pigeon_core::{agent::sha256_file, config::Home, identity::DEVICE_KEY};

struct Node {
    backend: Arc<Backend>,
    home: Home,
}

async fn node(root: &Path, name: &str) -> Node {
    let home = Home::at(root.join(format!("{name}-home"))).unwrap();
    pigeon_core::identity::set_name(&home.dir, name).unwrap();
    let backend = Backend::start(Options {
        home: home.clone(),
        download_dir: root.join(format!("{name}-downloads")),
        local_only: true,
    })
    .await
    .unwrap();
    Node { backend, home }
}

async fn stop(n: Node) {
    n.backend.shutdown().await;
    // Real OS keystore (namespaced per temp dir): leave nothing behind.
    let _ = n.home.store().unwrap().delete(DEVICE_KEY);
}

fn write_file(path: &Path, len: usize) {
    let mut x: u32 = 0x1234_5678;
    let bytes: Vec<u8> = (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect();
    std::fs::write(path, bytes).unwrap();
}

fn waiting(v: InviteView) -> (String, Vec<String>, u8) {
    match v {
        InviteView::Waiting {
            link,
            code,
            attempts_left,
            ..
        } => (link, code, attempts_left),
        other => panic!("expected a waiting invite, got {other:?}"),
    }
}

fn wrong_code(code: &[String]) -> String {
    let words = Backend::wordlist();
    let first = if code[0] == words[0] {
        words[1]
    } else {
        words[0]
    };
    format!("{first} {} {} {}", code[1], code[2], code[3])
}

/// Pairs `getter` so it can get files from `sharer`, checking a wrong code first.
async fn pair(sharer: &Node, getter: &Node, device_name: &str, computer_name: &str) {
    let (link, code, left) = waiting(sharer.backend.create_invite(Some(device_name)).unwrap());
    assert_eq!(left, 3);
    let seen = getter.backend.check_link(&link).unwrap();
    assert_eq!(seen.computer, sharer.backend.id().to_string());
    assert!(!seen.expired);

    // A typo is caught locally and costs nothing.
    let typo = getter
        .backend
        .pair(&link, "otter plaza", None)
        .await
        .unwrap();
    assert!(matches!(typo, PairOutcome::Typo { .. }), "{typo:?}");
    // A wrong (but valid) code costs one try, on both screens.
    let wrong = getter
        .backend
        .pair(&link, &wrong_code(&code), None)
        .await
        .unwrap();
    assert!(
        matches!(wrong, PairOutcome::WrongCode { attempts_left: 2 }),
        "{wrong:?}"
    );
    let (_, _, left) = waiting(sharer.backend.status().unwrap().invite.unwrap());
    assert_eq!(left, 2);

    // Any case, any separator.
    let typed = code.join(" ").to_uppercase();
    let ok = getter
        .backend
        .pair(&link, &typed, Some(computer_name))
        .await
        .unwrap();
    match ok {
        PairOutcome::Paired {
            computer,
            my_fingerprint,
        } => {
            assert_eq!(computer.id, sharer.backend.id().to_string());
            assert_eq!(computer.name, computer_name);
            assert_eq!(
                my_fingerprint,
                getter.backend.status().unwrap().me.fingerprint
            );
        }
        other => panic!("pairing failed: {other:?}"),
    }
    // The sharing side shows who paired, and lists the device.
    let mut paired = None;
    for _ in 0..50 {
        if let Some(InviteView::Paired { device }) = sharer.backend.status().unwrap().invite {
            paired = Some(device);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let device = paired.expect("share side never showed the pairing");
    assert_eq!(device.name, device_name);
    assert_eq!(device.id, getter.backend.id().to_string());
    assert!(
        sharer
            .backend
            .status()
            .unwrap()
            .devices
            .iter()
            .any(|d| d.id == device.id)
    );
    sharer.backend.dismiss_invite();
    assert!(sharer.backend.status().unwrap().invite.is_none());

    // N2: the same link again says "already used", not "expired".
    let again = getter
        .backend
        .pair(&link, &code.join(" "), None)
        .await
        .unwrap();
    assert!(matches!(again, PairOutcome::AlreadyUsed), "{again:?}");
}

/// W-21 (SEC-14): the download folder is owner-only. Unix: mode 0700.
/// Windows: a protected DACL that names only this user and SYSTEM, read with
/// `icacls /save` (SDDL, independent of the OS language; PowerShell's
/// Get-Acl is not reliable on CI runners).
fn assert_owner_only(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "{} is {mode:o}", dir.display());
    }
    #[cfg(windows)]
    {
        let scratch = tempfile::tempdir().unwrap();
        let saved = scratch.path().join("acl.txt");
        let out = std::process::Command::new("icacls")
            .arg(dir)
            .arg("/save")
            .arg(&saved)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let bytes = std::fs::read(&saved).unwrap();
        let wide: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        let text = String::from_utf16_lossy(&wide);
        let dacl = &text[text.find("D:").expect(&text)..];
        let dacl = dacl.lines().next().unwrap().trim();
        assert!(dacl.starts_with("D:P"), "not protected: {dacl}");
        let who = std::process::Command::new("whoami")
            .args(["/user", "/fo", "csv", "/nh"])
            .output()
            .unwrap();
        let who = String::from_utf8_lossy(&who.stdout);
        let me = who
            .trim()
            .rsplit(',')
            .next()
            .unwrap()
            .trim_matches('"')
            .to_string();
        assert!(me.starts_with("S-1-"), "{who}");
        let trustees: Vec<&str> = dacl
            .split('(')
            .skip(1)
            .map(|ace| ace.trim_end_matches(')').rsplit(';').next().unwrap())
            .collect();
        assert!(!trustees.is_empty(), "{dacl}");
        for t in trustees {
            // The built-in Administrator (RID 500, the CI runner's user) is "LA".
            let is_me = t == me || (t == "LA" && me.ends_with("-500"));
            assert!(
                is_me || t == "SY",
                "{} grants access to {t}: {dacl}",
                dir.display()
            );
        }
    }
}

async fn wait_done(n: &Node, id: u64) -> pigeon_app::TransferView {
    for _ in 0..600 {
        let t = n
            .backend
            .status()
            .unwrap()
            .transfers
            .into_iter()
            .find(|t| t.id == id)
            .unwrap();
        if t.state != TransferState::Active {
            return t;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("transfer {id} never finished");
}

#[tokio::test(flavor = "multi_thread")]
async fn two_apps_pair_and_get_both_ways_on_one_endpoint_each() {
    let dir = tempfile::tempdir().unwrap();
    let a = node(dir.path(), "Work Mac").await;
    let b = node(dir.path(), "Home PC").await;
    // One endpoint per app, with the device key: the same ID serves (share)
    // and dials (get).
    assert_eq!(a.backend.endpoint().id(), a.backend.id());
    assert_eq!(b.backend.endpoint().id(), b.backend.id());

    // --- A shares ForHome/report.bin -------------------------------------
    let share = dir.path().join("ForHome");
    std::fs::create_dir_all(share.join("Reports")).unwrap();
    let src = share.join("Reports").join("report.bin");
    write_file(&src, 3 * 1024 * 1024 + 7);
    let folder = a.backend.add_folder(&share).unwrap();
    assert_eq!(folder.name, "ForHome");
    assert!(a.backend.add_folder(&share).is_err(), "shared twice");
    assert!(
        a.backend.add_folder(&a.home.dir).is_err(),
        "the config dir must never be shared"
    );

    pair(&a, &b, "Home PC", "Work Mac").await;

    // --- B browses and gets ----------------------------------------------
    let a_id = a.backend.id().to_string();
    let top = b.backend.list(&a_id, "").await.unwrap();
    assert_eq!(top.len(), 1);
    assert_eq!((top[0].name.as_str(), top[0].is_dir), ("ForHome", true));
    let inside = b.backend.list(&a_id, "ForHome/Reports").await.unwrap();
    assert_eq!(inside[0].path, "ForHome/Reports/report.bin");
    assert!(b.backend.list(&a_id, "ForHome/../..").await.is_err());
    assert_eq!(b.backend.status().unwrap().computers[0].online, Some(true));

    let id = b.backend.get(&a_id, &inside[0].path).unwrap();
    let t = wait_done(&b, id).await;
    assert_eq!(t.state, TransferState::Done, "{:?}", t.error);
    let want = pigeon_core::hex(&sha256_file(src.clone()).await.unwrap());
    let saved = b.backend.finished_file(id).unwrap();
    let got = pigeon_core::hex(&sha256_file(saved.clone()).await.unwrap());
    println!(
        "app e2e: {} bytes, route {:?}, sha256 src={want} dst={got}",
        t.size, t.route
    );
    assert_eq!(t.sha256.as_deref(), Some(want.as_str()));
    // W-19: never claimed as verified while the core cannot say so.
    assert_ne!(t.verified, Some(true), "core-file-safety not in yet");
    assert_eq!(got, want);
    assert_eq!(t.done, t.size);
    assert_eq!(
        saved,
        dir.path().join("Home PC-downloads").join("report.bin")
    );
    assert_owner_only(&dir.path().join("Home PC-downloads"));

    // Never overwrites: the second copy is `report (1).bin`.
    // Started from a thread outside the runtime, like a sync Tauri command
    // on the main thread (this used to panic: "no reactor running").
    let id2 = {
        let backend = b.backend.clone();
        let a_id = a_id.clone();
        std::thread::spawn(move || backend.get(&a_id, "ForHome/Reports/report.bin"))
            .join()
            .unwrap()
            .unwrap()
    };
    let t2 = wait_done(&b, id2).await;
    assert_eq!(t2.state, TransferState::Done, "{:?}", t2.error);
    assert_eq!(t2.file_name, "report (1).bin");
    assert_eq!(t2.wanted_name, "report.bin");
    // Open / Show in folder only work on finished transfers.
    assert!(b.backend.finished_file(999).is_err());

    // --- Reverse: B shares, A gets, on the same two endpoints -------------
    let scans = dir.path().join("Scans");
    std::fs::create_dir_all(&scans).unwrap();
    write_file(&scans.join("scan-041.pdf"), 200_000);
    b.backend.add_folder(&scans).unwrap();
    pair(&b, &a, "Work Mac", "Home PC").await;
    let b_id = b.backend.id().to_string();
    let id3 = a.backend.get(&b_id, "Scans/scan-041.pdf").unwrap();
    let t3 = wait_done(&a, id3).await;
    assert_eq!(t3.state, TransferState::Done, "{:?}", t3.error);
    assert_eq!(
        t3.sha256.unwrap(),
        pigeon_core::hex(&sha256_file(scans.join("scan-041.pdf")).await.unwrap())
    );
    // A's share kept working while A was getting (one endpoint, both roles).
    assert!(b.backend.list(&a_id, "ForHome").await.is_ok());

    // --- Stop sharing a folder, then revoke --------------------------------
    b.backend.remove_folder("Scans").unwrap();
    assert!(a.backend.list(&b_id, "Scans").await.is_err());
    assert!(a.backend.list(&b_id, "").await.unwrap().is_empty());

    a.backend
        .revoke_device(&b.backend.id().to_string())
        .unwrap();
    // N3: a friendly message, no CLI commands; the computer answered, so
    // it shows online (the banner must not say it may be asleep).
    let revoked = format!("{:#}", b.backend.list(&a_id, "").await.unwrap_err());
    assert!(
        revoked.starts_with("Work Mac no longer lets this computer get files"),
        "{revoked}"
    );
    assert!(!revoked.contains("pigeon"), "{revoked}");
    assert_eq!(b.backend.status().unwrap().computers[0].online, Some(true));
    // The other direction is separate and untouched.
    assert!(a.backend.list(&b_id, "").await.is_ok());

    stop(a).await;
    stop(b).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelled_and_burned_invites_show_on_the_share_side() {
    let dir = tempfile::tempdir().unwrap();
    let a = node(dir.path(), "Sharer").await;
    let b = node(dir.path(), "Getter").await;

    let (link, code, _) = waiting(a.backend.create_invite(None).unwrap());
    a.backend.cancel_invite();
    assert!(a.backend.status().unwrap().invite.is_none());
    let gone = b.backend.pair(&link, &code.join("-"), None).await.unwrap();
    assert!(matches!(gone, PairOutcome::AlreadyUsed), "{gone:?}");

    let (link, code, _) = waiting(a.backend.create_invite(None).unwrap());
    let bad = wrong_code(&code);
    for _ in 0..2 {
        b.backend.pair(&link, &bad, None).await.unwrap();
    }
    let last = b.backend.pair(&link, &bad, None).await.unwrap();
    assert!(matches!(last, PairOutcome::UsedUp), "{last:?}");
    assert!(matches!(
        a.backend.status().unwrap().invite,
        Some(InviteView::Burned)
    ));
    // The right code no longer works, and nobody was added.
    let late = b.backend.pair(&link, &code.join("-"), None).await.unwrap();
    assert!(!matches!(late, PairOutcome::Paired { .. }), "{late:?}");
    assert!(a.backend.status().unwrap().devices.is_empty());

    // A second app for the same computer is refused (one endpoint per key).
    let again = Backend::start(Options {
        home: a.home.clone(),
        download_dir: dir.path().join("x"),
        local_only: true,
    })
    .await;
    // M2: who holds it, in words; no lock file path, no "try again".
    let msg = format!("{:#}", again.err().expect("second start must fail"));
    assert_eq!(msg, pigeon_app::ALREADY_SHARING);

    stop(a).await;
    stop(b).await;
}

// ---------------------------------------------------------------------------
// Proof (PR #19): risks that had no test.

fn err_text<T: std::fmt::Debug>(r: anyhow::Result<T>) -> String {
    format!("{:#}", r.expect_err("expected an error"))
}

/// Every string the webview sends is capped in the backend before it is
/// parsed, stored or sent to the other computer; a folder that *contains*
/// Pigeon's config dir is refused like the config dir itself.
#[tokio::test(flavor = "multi_thread")]
async fn webview_inputs_are_capped_and_parents_of_the_config_dir_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let a = node(dir.path(), "Sharer").await;
    let b = node(dir.path(), "Getter").await;

    // `dir` holds both config dirs: sharing it would expose the allow-lists.
    assert!(
        err_text(a.backend.add_folder(dir.path())).contains("Pigeon's own settings"),
        "a parent of the config dir must be refused"
    );

    let (link, code, _) = waiting(a.backend.create_invite(None).unwrap());
    let long_code = format!(
        "{} {}",
        code.join(" "),
        "x".repeat(pigeon_app::MAX_CODE_INPUT)
    );
    assert!(
        err_text(b.backend.pair(&link, &long_code, None).await).contains("too long"),
        "an over-long code must be refused before parsing"
    );
    let long_link = format!("{link}{}", "A".repeat(pigeon_app::MAX_LINK_INPUT));
    assert!(err_text(b.backend.check_link(&long_link)).contains("too long"));
    assert!(err_text(b.backend.pair(&long_link, &code.join(" "), None).await).contains("too long"));
    let long_name = "n".repeat(pigeon_app::MAX_NAME_INPUT + 1);
    assert!(
        err_text(
            b.backend
                .pair(&link, &code.join(" "), Some(&long_name))
                .await
        )
        .contains("too long")
    );
    assert!(err_text(a.backend.create_invite(Some(&long_name))).contains("too long"));
    assert!(err_text(a.backend.remove_folder(&long_name)).contains("too long"));
    assert!(err_text(a.backend.rename(&long_name)).contains("too long"));
    let a_id = a.backend.id().to_string();
    let long_path = "p/".repeat(pigeon_app::MAX_PATH_INPUT);
    assert!(err_text(b.backend.list(&a_id, &long_path).await).contains("too long"));
    assert!(err_text(b.backend.get(&a_id, &long_path)).contains("too long"));
    // NUL bytes too.
    assert!(err_text(b.backend.list(&a_id, "a\0b").await).contains("NUL"));
    // None of that cost a try or paired anyone.
    let (_, _, left) = waiting(a.backend.status().unwrap().invite.unwrap());
    assert_eq!(left, 3);
    assert!(a.backend.status().unwrap().devices.is_empty());

    stop(a).await;
    stop(b).await;
}

/// A failed transfer cannot be resumed while another transfer is writing the
/// same `.part` file (two writers on one `.part` corrupt it), and a stale
/// `.part` never turns into a wrong file.
#[tokio::test(flavor = "multi_thread")]
async fn resume_never_races_an_active_transfer_on_the_same_file() {
    let dir = tempfile::tempdir().unwrap();
    let a = node(dir.path(), "Sharer").await;
    let b = node(dir.path(), "Getter").await;
    let share = dir.path().join("Big");
    std::fs::create_dir_all(&share).unwrap();
    let src = share.join("big.bin");
    write_file(&src, 48 * 1024 * 1024);
    a.backend.add_folder(&share).unwrap();
    pair(&a, &b, "Getter", "Sharer").await;
    let a_id = a.backend.id().to_string();

    // First attempt fails: the folder is not shared at that moment.
    a.backend.remove_folder("Big").unwrap();
    let first = b.backend.get(&a_id, "Big/big.bin").unwrap();
    let t1 = wait_done(&b, first).await;
    assert_eq!(t1.state, TransferState::Failed);

    // Shared again; a new get of the same file starts on the same name.
    a.backend.add_folder(&share).unwrap();
    let second = b.backend.get(&a_id, "Big/big.bin").unwrap();
    let resumed = b.backend.resume(first);
    let t2 = wait_done(&b, second).await;
    if resumed.is_ok() {
        let t1 = wait_done(&b, first).await;
        eprintln!(
            "resume raced: t1={:?}/{:?} t2={:?}/{:?}",
            t1.state, t1.error, t2.state, t2.error
        );
    }
    assert!(
        resumed.is_err(),
        "resume must refuse while another transfer writes {}",
        t2.file_name
    );
    assert_eq!(t2.state, TransferState::Done, "{:?}", t2.error);
    let want = pigeon_core::hex(&sha256_file(src.clone()).await.unwrap());
    let got = b.backend.finished_file(second).unwrap();
    assert_eq!(pigeon_core::hex(&sha256_file(got).await.unwrap()), want);

    stop(a).await;
    stop(b).await;
}
