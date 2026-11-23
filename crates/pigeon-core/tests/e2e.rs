//! End-to-end: an agent and a client in one process, talking over iroh on
//! 127.0.0.1 with relays and address lookup disabled (works offline). The
//! client is on the agent's allow-list; pairing is tested in `pairing.rs`.

mod common;

use std::time::Duration;

use common::*;
use pigeon_core::{
    ALPN,
    agent::{Limits, ServeOptions, close, sha256_file},
    client,
    proto::{Request, Response, read_frame},
};

#[tokio::test(flavor = "multi_thread")]
async fn transfer_100mb_and_hash_matches() {
    let s = setup().await;
    let size = 100 * 1024 * 1024;
    let src = s.root.join("sub").join("big.bin");
    write_random_file(&src, size);

    let entries = client::list(&s.client, &s.addr, "sub").await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "big.bin");
    assert_eq!(entries[0].size, size as u64);
    assert!(!entries[0].is_dir);

    let out = s.work.join("big.bin");
    let stats = client::get(&s.client, &s.addr, "sub/big.bin", &out)
        .await
        .unwrap();
    let expected = pigeon_core::hex(&sha256_file(src).await.unwrap());
    let actual = pigeon_core::hex(&sha256_file(out.clone()).await.unwrap());
    println!(
        "e2e: {} bytes in {:.3}s = {:.1} MB/s, sha256 src={} dst={} match={}",
        stats.size,
        stats.elapsed.as_secs_f64(),
        stats.mb_per_sec(),
        expected,
        actual,
        expected == actual
    );
    assert_eq!(stats.size, size as u64);
    assert_eq!(expected, actual);
    assert_eq!(stats.sha256, expected);
    // Loopback with relays disabled: the only possible path is direct.
    assert_eq!(stats.route, client::Route::Direct);
    assert!(!client::part_path(&out).exists());
    // SEC-14: the fetched copy is private to the home user.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&out).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "fetched file mode {mode:o}");
    }
    shutdown(s).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn resume_from_part_file() {
    let s = setup().await;
    let src = s.root.join("resume.bin");
    write_random_file(&src, 3 * 1024 * 1024 + 123);
    let data = std::fs::read(&src).unwrap();

    let out = s.work.join("resume.bin");
    std::fs::write(client::part_path(&out), &data[..1_000_000]).unwrap();
    let stats = client::get(&s.client, &s.addr, "resume.bin", &out)
        .await
        .unwrap();
    assert_eq!(stats.resumed_from, 1_000_000);
    assert_eq!(stats.received, data.len() as u64 - 1_000_000);
    assert_eq!(std::fs::read(&out).unwrap(), data);
    shutdown(s).await;
}

/// SEC-08 / P-T2 (replaces Day-1 `wrong_token_is_rejected`): a device whose
/// key is not on the allow-list is closed with "not paired" before the agent
/// accepts any stream; nothing is listed or downloaded.
#[tokio::test(flavor = "multi_thread")]
async fn unpaired_device_is_refused_before_any_stream() {
    let s = setup().await;
    std::fs::write(s.root.join("a.txt"), b"hello").unwrap();
    let stranger = local_endpoint(vec![]).await;

    let err = within(client::list(&stranger, &s.addr, ""))
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("not paired"), "{err:#}");
    let out = s.work.join("a.txt");
    let err = within(client::get(&stranger, &s.addr, "a.txt", &out))
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("not paired"), "{err:#}");
    assert!(!out.exists() && !client::part_path(&out).exists());

    // At the QUIC level: the agent closes with NOT_PAIRED without reading.
    let conn = within(stranger.connect(s.addr.clone(), ALPN))
        .await
        .unwrap();
    if let Ok((mut send, mut recv)) = conn.open_bi().await {
        let req = Request::List {
            path: String::new(),
        };
        let _ = pigeon_core::proto::write_frame(&mut send, &req).await;
        let _ = send.finish();
        let read = within(read_frame::<_, Response>(&mut recv)).await;
        assert!(read.is_err(), "an unpaired device got a response: {read:?}");
    }
    let reason = within(conn.closed()).await;
    assert!(
        format!("{reason}").contains(&format!("{}", close::NOT_PAIRED)),
        "{reason}"
    );

    // The paired client still works.
    let entries = within(client::list(&s.client, &s.addr, "")).await.unwrap();
    assert!(entries.iter().any(|e| e.name == "a.txt"));
    stranger.close().await;
    shutdown(s).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn path_escape_is_rejected() {
    let s = setup().await;
    let out = s.work.join("stolen.txt");
    for path in ["../secret.txt", "sub/../../secret.txt", "..\\secret.txt"] {
        let err = client::get(&s.client, &s.addr, path, &out)
            .await
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("not allowed"),
            "{path}: {err:#}"
        );
        assert!(!out.exists());
    }
    let err = client::list(&s.client, &s.addr, "..").await.unwrap_err();
    assert!(format!("{err:#}").contains("not allowed"), "{err:#}");
    shutdown(s).await;
}

// ---- Added by Proof (verifier) ----

#[tokio::test(flavor = "multi_thread")]
async fn stale_part_from_changed_source_is_dropped_not_kept() {
    let s = setup().await;
    let src = s.root.join("changed.bin");
    write_random_file(&src, 2 * 1024 * 1024);
    let data = std::fs::read(&src).unwrap();

    // A .part left over from an older version of the file: same length prefix,
    // different bytes.
    let out = s.work.join("changed.bin");
    let part = client::part_path(&out);
    std::fs::write(&part, vec![0xAAu8; 500_000]).unwrap();

    let err = within(client::get(&s.client, &s.addr, "changed.bin", &out))
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("SHA-256 mismatch"), "{err:#}");
    assert!(!out.exists(), "corrupt data must never land at OUT");
    assert!(!part.exists(), "a corrupt .part must be dropped");

    // Running again downloads cleanly.
    within(client::get(&s.client, &s.addr, "changed.bin", &out))
        .await
        .unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), data);
    shutdown(s).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn part_larger_than_source_restarts_from_zero() {
    let s = setup().await;
    std::fs::write(s.root.join("small.txt"), b"tiny").unwrap();
    let out = s.work.join("small.txt");
    let part = client::part_path(&out);
    std::fs::write(&part, vec![1u8; 10_000]).unwrap();

    let stats = within(client::get(&s.client, &s.addr, "small.txt", &out))
        .await
        .unwrap();
    assert_eq!(
        stats.resumed_from, 0,
        "a stale, too-long .part is not resumed"
    );
    assert_eq!(stats.received, 4);
    assert_eq!(std::fs::read(&out).unwrap(), b"tiny");
    assert!(!part.exists());
    shutdown(s).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn unicode_and_spaces_in_names_roundtrip() {
    let s = setup().await;
    // NFC "é" and NFD "e\u{301}" are different names on Windows/Linux but the
    // same one on APFS; use only one form per file so the test is portable.
    let name = "caf\u{e9} \u{65e5}\u{672c} [1] #x.txt";
    std::fs::write(s.root.join(name), b"unicode ok").unwrap();
    let entries = within(client::list(&s.client, &s.addr, "")).await.unwrap();
    assert!(entries.iter().any(|e| e.name == name), "{entries:?}");
    let out = s.work.join(name);
    within(client::get(&s.client, &s.addr, name, &out))
        .await
        .unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"unicode ok");
    shutdown(s).await;
}

/// SEC-01 / SEC-05 (replaces the Day-1 oversized-hello test): a request
/// frame claiming 16 MiB is refused from its length prefix alone, from a
/// stranger (closed before any stream) and from a paired device.
#[tokio::test(flavor = "multi_thread")]
async fn oversized_request_is_refused_and_agent_keeps_serving() {
    use tokio::io::AsyncWriteExt;
    let s = setup().await;
    std::fs::write(s.root.join("a.txt"), b"hello").unwrap();
    let stranger = local_endpoint(vec![]).await;

    for ep in [&stranger, &s.client] {
        let conn = within(ep.connect(s.addr.clone(), ALPN)).await.unwrap();
        // The stranger may already be closed (not paired) before it can open
        // a stream; that is the point.
        let Ok((mut send, mut recv)) = conn.open_bi().await else {
            continue;
        };
        let _ = send
            .write_all(&pigeon_core::proto::MAX_FRAME_LEN.to_be_bytes())
            .await;
        let _ = send.flush().await;
        // The agent drops it at once; reading must end (error or EOF)
        // quickly, not wait for a body that never comes.
        let mut buf = [0u8; 16];
        let _ = tokio::time::timeout(Duration::from_secs(5), recv.read(&mut buf))
            .await
            .expect("agent kept waiting for an oversized request");
        conn.close(0u32.into(), b"bye");
    }

    let entries = within(client::list(&s.client, &s.addr, "")).await.unwrap();
    assert!(entries.iter().any(|e| e.name == "a.txt"));
    stranger.close().await;
    shutdown(s).await;
}

// ---- Connection limits and idle timeouts (SEC-18, TH-08) ----

fn short_limits(max_connections: usize, max_per_peer: usize) -> ServeOptions {
    ServeOptions {
        limits: Limits {
            max_connections,
            max_per_peer,
            handshake_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(1),
        },
        verbose: true,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn per_peer_connection_limit_closes_extra_connections() {
    let s = setup_with(short_limits(32, 2), true).await;
    std::fs::write(s.root.join("a.txt"), b"hello").unwrap();
    // Three connections from one peer that send nothing and just sit there.
    // The agent finishes handshakes in its own order, so we do not assume
    // which one it closes (W-8): exactly one must be closed as busy.
    let mut conns = Vec::new();
    for _ in 0..3 {
        conns.push(
            within(s.client.connect(s.addr.clone(), ALPN))
                .await
                .unwrap(),
        );
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let closed = loop {
        let closed: Vec<_> = conns
            .iter()
            .filter(|c| c.close_reason().is_some())
            .collect();
        if !closed.is_empty() || std::time::Instant::now() > deadline {
            break closed.len();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(closed, 1, "agent kept a connection over the per-peer limit");
    // Give a wrongly closed second one a moment to show up.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let (busy, open): (Vec<_>, Vec<_>) =
        conns.into_iter().partition(|c| c.close_reason().is_some());
    assert_eq!((busy.len(), open.len()), (1, 2));
    let reason = busy[0].close_reason().unwrap();
    assert!(format!("{reason}").contains("busy"), "{reason}");
    // Freeing a slot lets the peer in again.
    open[0].close(0u32.into(), b"bye");
    let entries = list_eventually(&s).await;
    assert!(entries.iter().any(|e| e.name == "a.txt"));
    open[1].close(0u32.into(), b"bye");
    shutdown(s).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn global_connection_limit_refuses_extra_connections() {
    let s = setup_with(short_limits(2, 8), true).await;
    std::fs::write(s.root.join("a.txt"), b"hello").unwrap();
    let held1 = within(s.client.connect(s.addr.clone(), ALPN))
        .await
        .unwrap();
    let held2 = within(s.client.connect(s.addr.clone(), ALPN))
        .await
        .unwrap();
    // A third connection (from any peer) is refused before the handshake.
    let other = local_endpoint(vec![]).await;
    let res = tokio::time::timeout(Duration::from_secs(10), other.connect(s.addr.clone(), ALPN))
        .await
        .expect("refused connection attempt hung");
    assert!(
        res.is_err(),
        "connection over the global limit was accepted"
    );
    held1.close(0u32.into(), b"bye");
    let entries = list_eventually(&s).await;
    assert!(entries.iter().any(|e| e.name == "a.txt"));
    held2.close(0u32.into(), b"bye");
    other.close().await;
    shutdown(s).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn idle_client_is_disconnected_after_its_request() {
    let s = setup_with(short_limits(32, 4), true).await;
    std::fs::write(s.root.join("a.txt"), b"hello").unwrap();
    let (conn, mut recv) = raw_request(
        &s.client,
        &s.addr,
        &Request::List {
            path: String::new(),
        },
    )
    .await;
    let response: Response = within(read_frame(&mut recv)).await.unwrap();
    assert!(matches!(response, Response::List(_)), "{response:?}");
    // We never close. The agent must not wait on us forever.
    let started = std::time::Instant::now();
    tokio::time::timeout(Duration::from_secs(5), conn.closed())
        .await
        .expect("agent kept an idle authenticated connection open");
    println!(
        "idle connection closed by the agent after {:?}",
        started.elapsed()
    );
    shutdown(s).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stalled_reader_is_disconnected() {
    let s = setup_with(short_limits(32, 4), true).await;
    write_random_file(&s.root.join("big.bin"), 48 * 1024 * 1024);
    let req = Request::Get {
        path: "big.bin".into(),
        offset: 0,
    };
    // Ask for 48 MiB and never read a byte: flow control stalls the agent's
    // writes, and the idle timeout must cut the connection.
    let (conn, _recv) = raw_request(&s.client, &s.addr, &req).await;
    tokio::time::timeout(Duration::from_secs(20), conn.closed())
        .await
        .expect("agent kept writing to a peer that never reads");
    // The agent keeps serving others.
    std::fs::write(s.root.join("a.txt"), b"hello").unwrap();
    let entries = list_eventually(&s).await;
    assert!(entries.iter().any(|e| e.name == "a.txt"));
    shutdown(s).await;
}
