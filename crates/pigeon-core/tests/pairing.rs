//! Pairing end to end (docs/security/PAIRING.md section 7). Agent and
//! clients in one process over loopback iroh, nothing persisted.
//! P-T9 (keystore), P-T10 (transcript binding) and P-T11 (parsers) are unit /
//! property tests in `keystore.rs`, `pair.rs` and `props.rs`.

mod common;

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use common::*;
use iroh::Endpoint;
use pigeon_core::{
    PAIR_ALPN,
    agent::{self, Agent, AgentStorage, Offer, ServeOptions},
    client,
    devices::{Device, DeviceList},
    pair::{
        self, AgentMsg, DEFAULT_TTL, Invite, InviteLink, MAX_PAIR_FRAME, PAIR_VERSION, PairConfirm,
        PairStart, PairingCode,
    },
    proto::{Request, read_frame_max, write_frame},
};
use spake2::{Ed25519Group, Identity, Password, Spake2};

/// An agent with an empty allow-list (deny by default) and an active invite.
async fn with_invite() -> (Setup, InviteLink, PairingCode) {
    let s = setup_with(ServeOptions::default(), false).await;
    let (invite, code) =
        Invite::new(DEFAULT_TTL, pair::now_secs(), Some("Home PC".into())).unwrap();
    let link = invite.link(s.addr.clone());
    s.agent.offer_invite(invite);
    assert!(s.agent.invite_active());
    (s, link, code)
}

/// A wrong code that is still made of valid words.
fn wrong(code: &PairingCode) -> PairingCode {
    let words = pair::wordlist();
    let first = code.reveal().split('-').next().unwrap();
    let other = if first == words[0] {
        words[1]
    } else {
        words[0]
    };
    let rest: Vec<&str> = code.reveal().split('-').skip(1).collect();
    PairingCode::parse(&format!("{other}-{}", rest.join("-"))).unwrap()
}

async fn new_client() -> Endpoint {
    local_endpoint(vec![]).await
}

/// Pairs `s.client`, retrying while the agent finishes an earlier handshake.
async fn pair_eventually(s: &Setup, link: &InviteLink, code: &PairingCode) {
    let mut last = None;
    for _ in 0..30 {
        match within(pair::pair(&s.client, link, code)).await {
            Ok(_) => return,
            Err(e) => last = Some(e),
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("pairing never worked: {:#}", last.unwrap());
}

/// P-T1: the right code pairs; the device is listed; `ls`/`get` then work
/// with nothing but the device key (no token on the wire, there is none).
#[tokio::test(flavor = "multi_thread")]
async fn pairing_with_the_right_code_lists_the_device_and_get_works() {
    let (s, link, code) = with_invite().await;
    std::fs::write(s.root.join("report.txt"), b"quarterly numbers").unwrap();

    // Before pairing: refused.
    assert!(within(client::list(&s.client, &s.addr, "")).await.is_err());

    let started = Instant::now();
    let paired = within(pair::pair(&s.client, &link, &code)).await.unwrap();
    println!("pairing took {:?}", started.elapsed());
    assert_eq!(paired.agent, s.agent.endpoint().id());
    let devices = s.agent.devices();
    assert_eq!(devices.devices.len(), 1);
    assert_eq!(devices.devices[0].id, s.client.id());
    assert_eq!(devices.devices[0].name, "Home PC");
    assert!(!s.agent.invite_active(), "the invite is single use");

    let entries = within(client::list(&s.client, &s.addr, "")).await.unwrap();
    assert!(entries.iter().any(|e| e.name == "report.txt"));
    let out = s.work.join("report.txt");
    let stats = within(client::get(&s.client, &s.addr, "report.txt", &out))
        .await
        .unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"quarterly numbers");
    assert_eq!(stats.size, 17);
    shutdown(s).await;
}

/// P-T3: a wrong code fails, costs one attempt, leaves no device behind, and
/// the invite still works with the right code.
#[tokio::test(flavor = "multi_thread")]
async fn wrong_code_costs_an_attempt_and_leaves_no_partial_state() {
    let (s, link, code) = with_invite().await;
    let err = within(pair::pair(&s.client, &link, &wrong(&code)))
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("attempts left: 2"), "{err:#}");
    assert!(s.agent.devices().devices.is_empty(), "no partial pairing");
    assert!(s.agent.invite_active());
    assert!(within(client::list(&s.client, &s.addr, "")).await.is_err());

    within(pair::pair(&s.client, &link, &code)).await.unwrap();
    assert!(s.agent.devices().contains(&s.client.id()));
    shutdown(s).await;
}

/// P-T4: three wrong codes burn the invite; a 4th try with the right code
/// fails, and the pairing ALPN is gone.
#[tokio::test(flavor = "multi_thread")]
async fn three_wrong_codes_burn_the_invite() {
    let (s, link, code) = with_invite().await;
    let bad = wrong(&code);
    for left in [2, 1] {
        let err = within(pair::pair(&s.client, &link, &bad))
            .await
            .unwrap_err();
        assert!(
            format!("{err:#}").contains(&format!("attempts left: {left}")),
            "{err:#}"
        );
    }
    let err = within(pair::pair(&s.client, &link, &bad))
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("used up"), "{err:#}");
    assert!(!s.agent.invite_active());
    let err = within(pair::pair(&s.client, &link, &code))
        .await
        .unwrap_err();
    println!("4th try with the right code: {err:#}");
    assert!(s.agent.devices().devices.is_empty());
    shutdown(s).await;
}

/// P-T5: while one pairing handshake is running (a peer with the link has
/// started it and stalls), a second is refused at once (no parallel
/// guessing) and costs no attempt.
#[tokio::test(flavor = "multi_thread")]
async fn second_concurrent_pairing_is_refused() {
    let (s, link, code) = with_invite().await;
    // Peer A sends a valid PairStart and then stalls before confirming.
    let a = new_client().await;
    let (held, _send, _recv) = start_raw_handshake(&a, &s, link.invite_id).await;

    let started = Instant::now();
    let err = within(pair::pair(&s.client, &link, &code))
        .await
        .unwrap_err();
    println!(
        "concurrent pairing refused after {:?}: {err:#}",
        started.elapsed()
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "not refused at once"
    );
    assert!(format!("{err:#}").contains("another pairing"), "{err:#}");
    assert!(s.agent.devices().devices.is_empty());

    // Once A goes away the invite still works: A used one attempt, the
    // refused pairing none.
    held.close(0u32.into(), b"bye");
    pair_eventually(&s, &link, &code).await;
    a.close().await;
    shutdown(s).await;
}

/// W-2 (Warden, inverted): a stranger who knows only the agent ID connects
/// on the pairing ALPN and sends nothing. The owner's pairing must still go
/// through at once; the stranger never gets the pairing lock.
#[tokio::test(flavor = "multi_thread")]
async fn stranger_without_invite_id_does_not_block_pairing() {
    let (s, link, code) = with_invite().await;
    let stranger = new_client().await;
    let held = within(stranger.connect(s.addr.clone(), PAIR_ALPN))
        .await
        .unwrap();
    let (_send, _recv) = held.open_bi().await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    let t = Instant::now();
    within(pair::pair(&s.client, &link, &code)).await.unwrap();
    println!(
        "owner paired after {:?} with a silent stranger connected",
        t.elapsed()
    );
    assert!(t.elapsed() < Duration::from_secs(2), "owner was held up");
    assert!(s.agent.devices().contains(&s.client.id()));
    // The silent stranger is dropped after the start timeout.
    tokio::time::timeout(pair::START_TIMEOUT + Duration::from_secs(3), held.closed())
        .await
        .expect("a silent pairing peer was kept open");
    stranger.close().await;
    shutdown(s).await;
}

/// W-2: a PairStart for an unknown invite ID is refused without using an
/// attempt or the lock, and a second pairing connection from the same peer
/// is refused (per-peer cap).
#[tokio::test(flavor = "multi_thread")]
async fn wrong_invite_id_and_second_connection_per_peer_are_refused() {
    let (s, link, code) = with_invite().await;
    let stranger = new_client().await;
    let conn = within(stranger.connect(s.addr.clone(), PAIR_ALPN))
        .await
        .unwrap();
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    let start = PairStart {
        version: PAIR_VERSION,
        invite_id: [7; 16],
        msg_a: vec![0; 33],
    };
    write_frame(&mut send, &start).await.unwrap();
    let reply: AgentMsg = within(read_frame_max(&mut recv, MAX_PAIR_FRAME))
        .await
        .unwrap();
    assert!(
        matches!(&reply, AgentMsg::Refused(why) if why.contains("no active invite")),
        "{reply:?}"
    );
    conn.close(0u32.into(), b"bye");

    // Per-peer cap: while A has one pairing connection open, a second one
    // from A is closed as busy.
    let a = new_client().await;
    let first = within(a.connect(s.addr.clone(), PAIR_ALPN)).await.unwrap();
    let (_s1, _r1) = first.open_bi().await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let second = within(a.connect(s.addr.clone(), PAIR_ALPN)).await.unwrap();
    let reason = tokio::time::timeout(Duration::from_secs(5), second.closed())
        .await
        .expect("second pairing connection from one peer was kept");
    assert!(format!("{reason}").contains("busy"), "{reason}");
    first.close(0u32.into(), b"bye");

    // None of that cost an attempt: three wrong codes are still allowed
    // after it, so the first wrong one reports 2 left.
    let err = within(pair::pair(&s.client, &link, &wrong(&code)))
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("attempts left: 2"), "{err:#}");
    a.close().await;
    stranger.close().await;
    shutdown(s).await;
}

/// P-T6: after a successful pairing the same link + code is useless, also
/// for a different device, and a stale copy of the invite is not revived.
#[tokio::test(flavor = "multi_thread")]
async fn invite_cannot_be_reused() {
    let s = setup_with(ServeOptions::default(), false).await;
    let (invite, code) = Invite::new(DEFAULT_TTL, pair::now_secs(), None).unwrap();
    let link = invite.link(s.addr.clone());
    s.agent.offer_invite(invite.clone());
    within(pair::pair(&s.client, &link, &code)).await.unwrap();

    let thief = new_client().await;
    assert!(within(pair::pair(&thief, &link, &code)).await.is_err());
    // E.g. the keystore copy could not be deleted: offering it again is ignored.
    s.agent.offer_invite(invite);
    assert!(!s.agent.invite_active());
    assert!(within(pair::pair(&thief, &link, &code)).await.is_err());
    let devices = s.agent.devices();
    assert_eq!(devices.devices.len(), 1);
    assert!(!devices.contains(&thief.id()));
    assert!(within(client::list(&thief, &s.addr, "")).await.is_err());
    thief.close().await;
    shutdown(s).await;
}

/// P-T7: an expired invite fails, whether it expired before it was offered
/// or while it was waiting (the invite takes the clock as an argument).
#[tokio::test(flavor = "multi_thread")]
async fn expired_invite_is_refused() {
    let s = setup_with(ServeOptions::default(), false).await;
    let now = pair::now_secs();
    let (old, code) = Invite::new(Duration::from_secs(60), now - 3600, None).unwrap();
    let link = old.link(s.addr.clone());
    s.agent.offer_invite(old);
    assert!(
        !s.agent.invite_active(),
        "an expired invite must not activate"
    );
    assert!(within(pair::pair(&s.client, &link, &code)).await.is_err());

    // Valid when offered, expired by the time it is used.
    let (short, code) = Invite::new(Duration::from_secs(2), now, None).unwrap();
    let link = short.link(s.addr.clone());
    s.agent.offer_invite(short);
    assert!(s.agent.invite_active());
    tokio::time::sleep(Duration::from_millis(3100)).await;
    let err = within(pair::pair(&s.client, &link, &code))
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("expired"), "{err:#}");
    assert!(!s.agent.invite_active());
    assert!(s.agent.devices().devices.is_empty());
    shutdown(s).await;
}

/// P-T8: revoking a device closes its live transfer within 1 s and blocks
/// reconnects.
#[tokio::test(flavor = "multi_thread")]
async fn revoke_closes_a_live_transfer_within_a_second() {
    let (s, link, code) = with_invite().await;
    within(pair::pair(&s.client, &link, &code)).await.unwrap();
    write_random_file(&s.root.join("big.bin"), 48 * 1024 * 1024);

    let req = Request::Get {
        path: "big.bin".into(),
        offset: 0,
    };
    let (conn, mut recv) = raw_request(&s.client, &s.addr, &req).await;
    // Make sure the transfer is really running.
    let mut buf = vec![0u8; 64 * 1024];
    let mut got = 0;
    while got < 1024 * 1024 {
        got += within(recv.read(&mut buf))
            .await
            .unwrap()
            .expect("stream ended early");
    }

    let started = Instant::now();
    s.agent.revoke(&s.client.id());
    within(conn.closed()).await;
    let took = started.elapsed();
    println!("revoke closed the live transfer after {took:?} ({got} bytes in)");
    assert!(took < Duration::from_secs(1), "revoke took {took:?}");

    let err = within(client::list(&s.client, &s.addr, ""))
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("not paired"), "{err:#}");
    shutdown(s).await;
}

/// P-T12: without an active invite the agent does not even offer the
/// pairing ALPN, so the TLS handshake fails.
#[tokio::test(flavor = "multi_thread")]
async fn pairing_alpn_is_refused_without_an_invite() {
    let s = setup_with(ServeOptions::default(), false).await;
    let res = within(s.client.connect(s.addr.clone(), PAIR_ALPN)).await;
    assert!(res.is_err(), "pairing ALPN accepted with no invite");

    // With an invite it is offered, and gone again after use.
    let (invite, code) = Invite::new(DEFAULT_TTL, pair::now_secs(), None).unwrap();
    let link = invite.link(s.addr.clone());
    s.agent.offer_invite(invite);
    within(pair::pair(&s.client, &link, &code)).await.unwrap();
    let other = new_client().await;
    let res = within(other.connect(s.addr.clone(), PAIR_ALPN)).await;
    assert!(
        res.is_err(),
        "pairing ALPN still offered after the invite was used"
    );
    other.close().await;
    shutdown(s).await;
}

/// A new `pair` replaces the old invite: the old link stops working.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_invite_replaces_the_old_one() {
    let (s, old_link, old_code) = with_invite().await;
    let (invite, code) = Invite::new(DEFAULT_TTL, pair::now_secs(), None).unwrap();
    let link = invite.link(s.addr.clone());
    s.agent.offer_invite(invite);
    let err = within(pair::pair(&s.client, &old_link, &old_code))
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("no active invite"), "{err:#}");
    within(pair::pair(&s.client, &link, &code)).await.unwrap();
    shutdown(s).await;
}

/// Starts a raw handshake for `invite_id` from `ep` and returns the streams
/// right after the agent's `Reply` (one attempt used, lock held).
async fn start_raw_handshake(
    ep: &Endpoint,
    s: &Setup,
    invite_id: [u8; 16],
) -> (
    iroh::endpoint::Connection,
    iroh::endpoint::SendStream,
    iroh::endpoint::RecvStream,
) {
    let conn = within(ep.connect(s.addr.clone(), PAIR_ALPN)).await.unwrap();
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    let agent_id = s.agent.endpoint().id();
    let (_st, msg_a) = Spake2::<Ed25519Group>::start_a(
        &Password::new(b"acid-acid-acid-acid"),
        &Identity::new(ep.id().as_bytes()),
        &Identity::new(agent_id.as_bytes()),
    );
    let start = PairStart {
        version: PAIR_VERSION,
        invite_id,
        msg_a,
    };
    write_frame(&mut send, &start).await.unwrap();
    let reply: AgentMsg = within(read_frame_max(&mut recv, MAX_PAIR_FRAME))
        .await
        .unwrap();
    assert!(matches!(reply, AgentMsg::Reply { .. }), "{reply:?}");
    (conn, send, recv)
}

/// W-1 (Warden): a wrong confirmation for an OLD invite, arriving after the
/// owner made a NEW invite, must not burn the new one.
#[tokio::test(flavor = "multi_thread")]
async fn stale_failed_attempt_does_not_burn_a_newer_invite() {
    let s = setup_with(ServeOptions::default(), false).await;
    let (a, _code_a) = Invite::new(DEFAULT_TTL, pair::now_secs(), None).unwrap();
    let a_id = a.invite_id;
    s.agent.offer_invite(a);

    let attacker = new_client().await;
    let (conn, mut send, mut recv) = start_raw_handshake(&attacker, &s, a_id).await;

    // The owner runs `pigeon-agent pair` again; `share` picks up invite B.
    let (b, code_b) = Invite::new(DEFAULT_TTL, pair::now_secs(), None).unwrap();
    let link_b = b.link(s.addr.clone());
    s.agent.offer_invite(b);
    assert!(s.agent.invite_active());

    // The attacker now sends a bogus confirmation for A.
    write_frame(&mut send, &PairConfirm { mac: [0; 32] })
        .await
        .unwrap();
    let res: AgentMsg = within(read_frame_max(&mut recv, MAX_PAIR_FRAME))
        .await
        .unwrap();
    assert!(matches!(res, AgentMsg::Failed { .. }), "{res:?}");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        s.agent.invite_active(),
        "a failed attempt on the old invite burned the new invite"
    );
    // And the owner's pairing with B works once the attacker is gone.
    conn.close(0u32.into(), b"bye");
    pair_eventually(&s, &link_b, &code_b).await;
    attacker.close().await;
    shutdown(s).await;
}

/// Proof: an "agent" that does not know the code (here: the right key, but a
/// SPAKE2 run with another password and a junk confirmation MAC) must not be
/// accepted by the client. Without the client's MAC check this would pair.
#[tokio::test(flavor = "multi_thread")]
async fn client_refuses_an_agent_that_cannot_prove_the_code() {
    use pigeon_core::pair::{AgentMsg, MAX_PAIR_FRAME, PairConfirm, PairStart};
    use pigeon_core::proto::{read_frame_max, write_frame};
    use spake2::{Ed25519Group, Identity, Password, Spake2};

    let fake = local_endpoint(vec![PAIR_ALPN.to_vec()]).await;
    let addr = addr_of(&fake);
    let (invite, code) = Invite::new(DEFAULT_TTL, pair::now_secs(), None).unwrap();
    let link = invite.link(addr);
    let server = tokio::spawn(async move {
        let conn = fake.accept().await.unwrap().await.unwrap();
        let (mut send, mut recv) = conn.accept_bi().await.unwrap();
        let _start: PairStart = read_frame_max(&mut recv, MAX_PAIR_FRAME).await.unwrap();
        let (_state, msg_b) = Spake2::<Ed25519Group>::start_b(
            &Password::new(b"not-the-code"),
            &Identity::new(b"a"),
            &Identity::new(b"b"),
        );
        write_frame(&mut send, &AgentMsg::Reply { msg_b })
            .await
            .unwrap();
        let _c: PairConfirm = read_frame_max(&mut recv, MAX_PAIR_FRAME).await.unwrap();
        write_frame(&mut send, &AgentMsg::Ok { mac: [0u8; 32] })
            .await
            .unwrap();
        send.finish().unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(5), conn.closed()).await;
    });
    let client = new_client().await;
    let err = within(pair::pair(&client, &link, &code))
        .await
        .expect_err("client accepted an agent that does not know the code");
    assert!(
        format!("{err:#}").contains("could not prove"),
        "unexpected error: {err:#}"
    );
    server.abort();
}

/// M1 (Proof): after `pigeon-agent reset` the next invite is made for the
/// new key. A `share` still running with the old key must ignore it (and
/// say so) instead of advertising an invite nobody can use.
#[tokio::test(flavor = "multi_thread")]
async fn invite_for_another_agent_key_is_ignored() {
    let s = setup_with(ServeOptions::default(), false).await;
    let other_key = iroh::SecretKey::from_bytes(&[42; 32]).public();
    let (invite, _) = Invite::new(DEFAULT_TTL, pair::now_secs(), None).unwrap();
    assert_eq!(
        s.agent.offer_invite(invite.for_agent(other_key)),
        Offer::OtherAgent
    );
    assert!(!s.agent.invite_active());
    // An invite made for this agent's key is taken.
    let (invite, _) = Invite::new(DEFAULT_TTL, pair::now_secs(), None).unwrap();
    let mine = invite.for_agent(s.agent.endpoint().id());
    assert_eq!(s.agent.offer_invite(mine), Offer::Activated);
    shutdown(s).await;
}

/// Shared "disk" for the restart test below.
#[derive(Default, Clone)]
struct RememberingStorage {
    spent: Arc<Mutex<HashSet<[u8; 16]>>>,
}

impl AgentStorage for RememberingStorage {
    fn device_paired(&self, _: &Device) -> anyhow::Result<()> {
        Ok(())
    }
    fn invite_changed(&self, _: &[u8; 16], _: Option<&Invite>) -> anyhow::Result<()> {
        // Simulates a keystore delete that keeps failing (locked Keychain).
        anyhow::bail!("keystore locked")
    }
    fn invite_spent(&self, id: &[u8; 16]) -> anyhow::Result<()> {
        self.spent.lock().unwrap().insert(*id);
        Ok(())
    }
    fn is_spent(&self, id: &[u8; 16]) -> bool {
        self.spent.lock().unwrap().contains(id)
    }
}

/// W-5 (Warden): a used invite whose keystore copy could not be deleted must
/// not come back after the agent restarts.
#[tokio::test(flavor = "multi_thread")]
async fn used_invite_does_not_come_back_after_a_restart() {
    let disk = RememberingStorage::default();
    let dir = tempfile::tempdir().unwrap();
    let start_agent = |ep: Endpoint| {
        let agent = Agent::new(
            ep,
            dir.path().to_path_buf(),
            DeviceList::default(),
            ServeOptions::default(),
            Box::new(disk.clone()),
        );
        let serving = agent.clone();
        tokio::spawn(async move { serving.serve().await });
        agent
    };
    let ep = local_endpoint(vec![]).await;
    let addr = addr_of(&ep);
    let agent = start_agent(ep);
    let (invite, code) = Invite::new(DEFAULT_TTL, pair::now_secs(), None).unwrap();
    let link = invite.link(addr);
    agent.offer_invite(invite.clone());
    let client = new_client().await;
    within(pair::pair(&client, &link, &code)).await.unwrap();
    agent.endpoint().close().await;

    // "Restart": a new agent on the same storage; the keystore still holds
    // the invite (its delete failed), and `share` offers it again.
    let restarted = start_agent(local_endpoint(vec![]).await);
    assert_eq!(restarted.offer_invite(invite), Offer::Spent);
    assert!(!restarted.invite_active());
    client.close().await;
    restarted.endpoint().close().await;
}

/// W-8 / SEC-08: the path users really take. `pigeon-agent revoke` edits
/// devices.json; the running agent's `watch` loop picks it up and closes the
/// device's live transfer within 1.5 s.
#[tokio::test(flavor = "multi_thread")]
async fn revoke_through_the_devices_file_closes_a_live_transfer() {
    let s = setup().await;
    let devices_path = s.work.join("devices.json");
    s.agent.devices().save(&devices_path).unwrap();
    let watcher = tokio::spawn(agent::watch(s.agent.clone(), devices_path.clone(), || {
        Ok(None)
    }));
    write_random_file(&s.root.join("big.bin"), 48 * 1024 * 1024);
    let req = Request::Get {
        path: "big.bin".into(),
        offset: 0,
    };
    let (conn, mut recv) = raw_request(&s.client, &s.addr, &req).await;
    let mut buf = vec![0u8; 64 * 1024];
    let mut got = 0;
    while got < 1024 * 1024 {
        got += within(recv.read(&mut buf))
            .await
            .unwrap()
            .expect("stream ended early");
    }
    // What `pigeon-agent revoke "test client"` does:
    let started = Instant::now();
    DeviceList::update(&devices_path, |list| list.remove("test client").map(|_| ())).unwrap();
    tokio::time::timeout(Duration::from_millis(1500), conn.closed())
        .await
        .expect("revoke through the file did not close the transfer within 1.5 s");
    println!(
        "file revoke closed the transfer after {:?}",
        started.elapsed()
    );
    let err = within(client::list(&s.client, &s.addr, ""))
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("not paired"), "{err:#}");
    watcher.abort();
    shutdown(s).await;
}
