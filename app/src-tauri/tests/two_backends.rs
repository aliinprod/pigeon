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
    assert_eq!(got, want);
    assert_eq!(t.done, t.size);
    assert_eq!(
        saved,
        dir.path().join("Home PC-downloads").join("report.bin")
    );

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
    assert!(b.backend.list(&a_id, "").await.is_err());
    assert_eq!(b.backend.status().unwrap().computers[0].online, Some(false));
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
    assert!(
        matches!(gone, PairOutcome::Gone { .. } | PairOutcome::Expired),
        "{gone:?}"
    );

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
    assert!(again.is_err());

    stop(a).await;
    stop(b).await;
}
