//! End-to-end: an agent and a client in one process, talking over iroh on
//! 127.0.0.1 with relays and address lookup disabled (works offline).

use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{Connection, presets},
};
use pigeon_core::proto::{Hello, PROTOCOL_VERSION, Request, Response, read_frame, write_frame};
use pigeon_core::{
    ALPN, Ticket,
    agent::{Limits, ServeOptions, Share, serve, sha256_file},
    client,
};

async fn local_endpoint(alpns: Vec<Vec<u8>>) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0")
        .unwrap()
        .alpns(alpns)
        .bind()
        .await
        .unwrap()
}

struct Setup {
    _dir: tempfile::TempDir,
    root: PathBuf,
    work: PathBuf,
    agent: Endpoint,
    client: Endpoint,
    ticket: Ticket,
}

async fn setup() -> Setup {
    setup_with(ServeOptions::default()).await
}

async fn setup_with(opts: ServeOptions) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("share");
    let work = dir.path().join("work");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(dir.path().join("secret.txt"), b"outside the share").unwrap();

    let agent = local_endpoint(vec![ALPN.to_vec()]).await;
    let client = local_endpoint(vec![]).await;

    let token = pigeon_core::new_token().unwrap();
    let addr = agent
        .bound_sockets()
        .into_iter()
        .fold(EndpointAddr::new(agent.id()), |a, s| a.with_ip_addr(s));
    // Round-trip through the text form, like a user pasting it.
    let ticket: Ticket = Ticket { addr, token }.to_string().parse().unwrap();

    tokio::spawn(serve(
        agent.clone(),
        Share {
            root: root.clone(),
            token,
        },
        opts,
    ));
    Setup {
        _dir: dir,
        root,
        work,
        agent,
        client,
        ticket,
    }
}

/// Writes `len` bytes of deterministic pseudo-random data.
fn write_random_file(path: &Path, len: usize) {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut buf = vec![0u8; 1024 * 1024];
    let mut left = len;
    while left > 0 {
        for chunk in buf.chunks_mut(8) {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            chunk.copy_from_slice(&x.to_le_bytes()[..chunk.len()]);
        }
        let n = left.min(buf.len());
        f.write_all(&buf[..n]).unwrap();
        left -= n;
    }
    f.flush().unwrap();
}

async fn shutdown(s: Setup) {
    s.client.close().await;
    s.agent.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn transfer_100mb_and_hash_matches() {
    let s = setup().await;
    let size = 100 * 1024 * 1024;
    let src = s.root.join("sub").join("big.bin");
    write_random_file(&src, size);

    let entries = client::list(&s.client, &s.ticket, "sub").await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "big.bin");
    assert_eq!(entries[0].size, size as u64);
    assert!(!entries[0].is_dir);

    let out = s.work.join("big.bin");
    let stats = client::get(&s.client, &s.ticket, "sub/big.bin", &out)
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
    let stats = client::get(&s.client, &s.ticket, "resume.bin", &out)
        .await
        .unwrap();
    assert_eq!(stats.resumed_from, 1_000_000);
    assert_eq!(stats.received, data.len() as u64 - 1_000_000);
    assert_eq!(std::fs::read(&out).unwrap(), data);
    shutdown(s).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_token_is_rejected() {
    let s = setup().await;
    std::fs::write(s.root.join("a.txt"), b"hello").unwrap();
    let mut bad = s.ticket.clone();
    bad.token[0] ^= 1;

    let err = client::list(&s.client, &bad, "").await.unwrap_err();
    assert!(format!("{err:#}").contains("unauthorized"), "{err:#}");
    let out = s.work.join("a.txt");
    let err = client::get(&s.client, &bad, "a.txt", &out)
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("unauthorized"), "{err:#}");
    assert!(!out.exists());

    // The right token still works.
    let entries = client::list(&s.client, &s.ticket, "").await.unwrap();
    assert!(entries.iter().any(|e| e.name == "a.txt"));
    shutdown(s).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn path_escape_is_rejected() {
    let s = setup().await;
    let out = s.work.join("stolen.txt");
    for path in ["../secret.txt", "sub/../../secret.txt", "..\\secret.txt"] {
        let err = client::get(&s.client, &s.ticket, path, &out)
            .await
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("not allowed"),
            "{path}: {err:#}"
        );
        assert!(!out.exists());
    }
    let err = client::list(&s.client, &s.ticket, "..").await.unwrap_err();
    assert!(format!("{err:#}").contains("not allowed"), "{err:#}");
    shutdown(s).await;
}

// ---- Added by Proof (verifier) ----

/// Fails the test instead of hanging forever.
async fn within<T>(fut: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(60), fut)
        .await
        .expect("operation hung")
}

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

    let err = within(client::get(&s.client, &s.ticket, "changed.bin", &out))
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("SHA-256 mismatch"), "{err:#}");
    assert!(!out.exists(), "corrupt data must never land at OUT");
    assert!(!part.exists(), "a corrupt .part must be dropped");

    // Running again downloads cleanly.
    within(client::get(&s.client, &s.ticket, "changed.bin", &out))
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

    let stats = within(client::get(&s.client, &s.ticket, "small.txt", &out))
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
    let entries = within(client::list(&s.client, &s.ticket, ""))
        .await
        .unwrap();
    assert!(entries.iter().any(|e| e.name == name), "{entries:?}");
    let out = s.work.join(name);
    within(client::get(&s.client, &s.ticket, name, &out))
        .await
        .unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"unicode ok");
    shutdown(s).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn oversized_hello_from_stranger_is_refused_and_agent_keeps_serving() {
    use tokio::io::AsyncWriteExt;
    let s = setup().await;
    std::fs::write(s.root.join("a.txt"), b"hello").unwrap();

    // A peer that only knows the address (no token) claims a 16 MiB hello and
    // never sends it. The agent must refuse from the length prefix alone.
    let conn = within(s.client.connect(s.ticket.addr.clone(), ALPN))
        .await
        .unwrap();
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(&pigeon_core::proto::MAX_FRAME_LEN.to_be_bytes())
        .await
        .unwrap();
    send.flush().await.unwrap();
    // The agent drops the connection at once; reading must end (error or EOF)
    // quickly, not wait for a body that never comes.
    let mut buf = [0u8; 16];
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), recv.read(&mut buf))
        .await
        .expect("agent kept waiting for an oversized unauthenticated hello");
    conn.close(0u32.into(), b"bye");

    let entries = within(client::list(&s.client, &s.ticket, ""))
        .await
        .unwrap();
    assert!(entries.iter().any(|e| e.name == "a.txt"));
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

/// Sends hello + `req` on a fresh connection and returns it without closing.
async fn raw_request(s: &Setup, req: &Request) -> (Connection, iroh::endpoint::RecvStream) {
    let conn = within(s.client.connect(s.ticket.addr.clone(), ALPN))
        .await
        .unwrap();
    let (mut send, recv) = conn.open_bi().await.unwrap();
    let hello = Hello {
        version: PROTOCOL_VERSION,
        token: s.ticket.token,
    };
    write_frame(&mut send, &hello).await.unwrap();
    write_frame(&mut send, req).await.unwrap();
    send.finish().unwrap();
    (conn, recv)
}

/// Retries `list` until the agent has freed a slot (it notices closed
/// connections asynchronously).
async fn list_eventually(s: &Setup) -> Vec<pigeon_core::proto::Entry> {
    within(async {
        loop {
            if let Ok(entries) = client::list(&s.client, &s.ticket, "").await {
                return entries;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn per_peer_connection_limit_closes_extra_connections() {
    let s = setup_with(short_limits(32, 2)).await;
    std::fs::write(s.root.join("a.txt"), b"hello").unwrap();
    // Two connections that send nothing yet and just sit there.
    let held1 = within(s.client.connect(s.ticket.addr.clone(), ALPN))
        .await
        .unwrap();
    let held2 = within(s.client.connect(s.ticket.addr.clone(), ALPN))
        .await
        .unwrap();
    // The third one from the same peer is closed by the agent as "busy".
    let extra = within(s.client.connect(s.ticket.addr.clone(), ALPN))
        .await
        .unwrap();
    let reason = tokio::time::timeout(Duration::from_secs(5), extra.closed())
        .await
        .expect("agent kept a connection over the per-peer limit");
    assert!(format!("{reason}").contains("busy"), "{reason}");
    // Freeing a slot lets the peer in again.
    held1.close(0u32.into(), b"bye");
    let entries = list_eventually(&s).await;
    assert!(entries.iter().any(|e| e.name == "a.txt"));
    held2.close(0u32.into(), b"bye");
    shutdown(s).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn global_connection_limit_refuses_extra_connections() {
    let s = setup_with(short_limits(2, 8)).await;
    std::fs::write(s.root.join("a.txt"), b"hello").unwrap();
    let held1 = within(s.client.connect(s.ticket.addr.clone(), ALPN))
        .await
        .unwrap();
    let held2 = within(s.client.connect(s.ticket.addr.clone(), ALPN))
        .await
        .unwrap();
    // A third connection (from any peer) is refused before the handshake.
    let other = local_endpoint(vec![]).await;
    let res = tokio::time::timeout(
        Duration::from_secs(10),
        other.connect(s.ticket.addr.clone(), ALPN),
    )
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
    let s = setup_with(short_limits(32, 4)).await;
    std::fs::write(s.root.join("a.txt"), b"hello").unwrap();
    let (conn, mut recv) = raw_request(
        &s,
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
    let s = setup_with(short_limits(32, 4)).await;
    write_random_file(&s.root.join("big.bin"), 48 * 1024 * 1024);
    let req = Request::Get {
        path: "big.bin".into(),
        offset: 0,
    };
    // Ask for 48 MiB and never read a byte: flow control stalls the agent's
    // writes, and the idle timeout must cut the connection.
    let (conn, _recv) = raw_request(&s, &req).await;
    tokio::time::timeout(Duration::from_secs(20), conn.closed())
        .await
        .expect("agent kept writing to a peer that never reads");
    // The agent keeps serving others.
    std::fs::write(s.root.join("a.txt"), b"hello").unwrap();
    let entries = list_eventually(&s).await;
    assert!(entries.iter().any(|e| e.name == "a.txt"));
    shutdown(s).await;
}
