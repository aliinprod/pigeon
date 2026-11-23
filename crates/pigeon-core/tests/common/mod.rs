//! Shared setup for the end-to-end tests: an agent and clients in one
//! process, talking over iroh on 127.0.0.1 with relays and address lookup
//! disabled (works offline).
#![allow(dead_code)]

use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{Connection, RecvStream, presets},
};
use pigeon_core::{
    ALPN,
    agent::{Agent, NoStorage, ServeOptions},
    devices::{Device, DeviceList},
    proto::{Request, write_frame},
};

pub async fn local_endpoint(alpns: Vec<Vec<u8>>) -> Endpoint {
    Endpoint::builder(presets::Minimal)
        .clear_ip_transports()
        .bind_addr("127.0.0.1:0")
        .unwrap()
        .alpns(alpns)
        .bind()
        .await
        .unwrap()
}

pub struct Setup {
    pub _dir: tempfile::TempDir,
    pub root: PathBuf,
    pub work: PathBuf,
    pub agent: Agent,
    /// A client that is on the agent's allow-list (unless `paired` was false).
    pub client: Endpoint,
    /// How the client dials the agent (endpoint ID + loopback address).
    pub addr: EndpointAddr,
}

/// An agent serving a temp share. `paired`: put `client` on the allow-list
/// directly (pairing itself is tested in `pairing.rs`).
pub async fn setup_with(opts: ServeOptions, paired: bool) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("share");
    let work = dir.path().join("work");
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(dir.path().join("secret.txt"), b"outside the share").unwrap();

    let agent_ep = local_endpoint(vec![ALPN.to_vec()]).await;
    let client = local_endpoint(vec![]).await;
    let addr = addr_of(&agent_ep);
    let mut devices = DeviceList::default();
    if paired {
        devices.upsert(Device {
            id: client.id(),
            name: "test client".into(),
            paired_at: 0,
        });
    }
    let agent = Agent::new(agent_ep, root.clone(), devices, opts, Box::new(NoStorage));
    let serving = agent.clone();
    tokio::spawn(async move { serving.serve().await });
    Setup {
        _dir: dir,
        root,
        work,
        agent,
        client,
        addr,
    }
}

pub async fn setup() -> Setup {
    setup_with(ServeOptions::default(), true).await
}

/// The agent's endpoint ID plus its loopback sockets.
pub fn addr_of(ep: &Endpoint) -> EndpointAddr {
    ep.bound_sockets()
        .into_iter()
        .fold(EndpointAddr::new(ep.id()), |a, s| a.with_ip_addr(s))
}

pub async fn shutdown(s: Setup) {
    s.client.close().await;
    s.agent.endpoint().close().await;
}

/// Fails the test instead of hanging forever.
pub async fn within<T>(fut: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(60), fut)
        .await
        .expect("operation hung")
}

/// Writes `len` bytes of deterministic pseudo-random data.
pub fn write_random_file(path: &Path, len: usize) {
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

/// Sends `req` from `client` on a fresh connection and returns it open.
pub async fn raw_request(
    client: &Endpoint,
    addr: &EndpointAddr,
    req: &Request,
) -> (Connection, RecvStream) {
    let conn = within(client.connect(addr.clone(), ALPN)).await.unwrap();
    let (mut send, recv) = conn.open_bi().await.unwrap();
    write_frame(&mut send, req).await.unwrap();
    send.finish().unwrap();
    (conn, recv)
}

/// Retries `list` until it works (the agent frees slots asynchronously).
pub async fn list_eventually(s: &Setup) -> Vec<pigeon_core::proto::Entry> {
    within(async {
        loop {
            if let Ok(entries) = pigeon_core::client::list(&s.client, &s.addr, "").await {
                return entries;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
}
