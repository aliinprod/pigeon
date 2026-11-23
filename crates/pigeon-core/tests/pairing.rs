//! Pairing end to end (docs/security/PAIRING.md section 7). Agent and
//! clients in one process over loopback iroh, nothing persisted.
//! P-T9 (keystore), P-T10 (transcript binding) and P-T11 (parsers) are unit /
//! property tests in `keystore.rs`, `pair.rs` and `props.rs`.

mod common;

use std::time::{Duration, Instant};

use common::*;
use iroh::Endpoint;
use pigeon_core::{
    PAIR_ALPN,
    agent::ServeOptions,
    client,
    pair::{self, DEFAULT_TTL, Invite, InviteLink, PairingCode},
    proto::Request,
};

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

/// P-T5: while one pairing handshake is running, a second is refused at once
/// (no parallel guessing) and costs no attempt.
#[tokio::test(flavor = "multi_thread")]
async fn second_concurrent_pairing_is_refused() {
    let (s, link, code) = with_invite().await;
    // Peer A starts a pairing connection and stalls inside it.
    let a = new_client().await;
    let held = within(a.connect(s.addr.clone(), PAIR_ALPN)).await.unwrap();
    let (_send, _recv) = held.open_bi().await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

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
    assert!(s.agent.devices().devices.is_empty());

    // Once A goes away, the invite (all 3 attempts) is still there.
    held.close(0u32.into(), b"bye");
    let mut paired = false;
    for _ in 0..20 {
        if within(pair::pair(&s.client, &link, &code)).await.is_ok() {
            paired = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(paired, "pairing did not work after the stalled peer left");
    a.close().await;
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
