//! One key per computer (PAIRING.md D9), end to end over loopback iroh.
//!
//! Two computers in the state the owner's Mac and PC were in after Day 5:
//! each has an agent key and a client key; each agent allow-lists the other
//! computer's *client* key; each client remembers the other computer's
//! *agent*. After the migration both directions must keep working, and
//! pairing again must switch to the device key and drop the old client key.

mod common;

use std::path::{Path, PathBuf};

use common::*;
use iroh::{Endpoint, EndpointAddr, SecretKey};
use pigeon_core::{
    ALPN, PAIR_ALPN,
    agent::{Agent, NoStorage, ServeOptions},
    client,
    devices::{AGENTS_FILE, AgentList, DEVICES_FILE, Device, DeviceList, PairedAgent},
    identity::{self, LEGACY_AGENT_KEY, LEGACY_CLIENT_KEY, Origin},
    keystore::{MemoryStore, read_key, write_key},
    pair::{self, DEFAULT_TTL, Invite},
};

/// One computer with a pre-one-key install.
struct Computer {
    _dir: tempfile::TempDir,
    home: PathBuf,
    share: PathBuf,
    store: MemoryStore,
    agent_key: SecretKey,
    client_key: SecretKey,
}

fn old_install(file: &str, body: &[u8]) -> Computer {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let share = dir.path().join("share");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&share).unwrap();
    std::fs::write(share.join(file), body).unwrap();
    let store = MemoryStore::default();
    let agent_key = SecretKey::generate();
    let client_key = SecretKey::generate();
    write_key(&store, LEGACY_AGENT_KEY, &agent_key).unwrap();
    write_key(&store, LEGACY_CLIENT_KEY, &client_key).unwrap();
    Computer {
        _dir: dir,
        home,
        share,
        store,
        agent_key,
        client_key,
    }
}

/// `pigeon-agent pair` + `pigeon pair` as the old build did: the agent's
/// allow-list gets the client key, the client remembers the agent key.
fn old_pairing(agent: &Computer, client: &Computer, agent_name: &str) {
    DeviceList::update(&agent.home.join(DEVICES_FILE), |l| {
        l.upsert(Device {
            id: client.client_key.public(),
            name: "old client".into(),
            paired_at: 1,
        });
        Ok(())
    })
    .unwrap();
    AgentList::update(&client.home.join(AGENTS_FILE), |l| {
        l.upsert(PairedAgent {
            id: agent.agent_key.public(),
            name: agent_name.into(),
            relay_url: None,
            paired_at: 1,
            legacy_key: None,
        });
        Ok(())
    })
    .unwrap();
}

/// Starts `computer`'s agent with its (migrated) device key and its allow-list.
async fn start_agent(computer: &Computer) -> (Agent, EndpointAddr) {
    let me = identity::load(&computer.home, &computer.store).unwrap();
    let ep = local_endpoint_with_key(me.key.clone(), vec![ALPN.to_vec()]).await;
    let addr = addr_of(&ep);
    let devices = DeviceList::load(&computer.home.join(DEVICES_FILE)).unwrap();
    let agent = Agent::new(
        ep,
        computer.share.clone(),
        devices,
        ServeOptions::default(),
        Box::new(NoStorage),
    );
    let serving = agent.clone();
    tokio::spawn(async move { serving.serve().await });
    (agent, addr)
}

/// What `pigeon get` does: pick the agent, choose the key for it, dial.
async fn fetch(client: &Computer, agent_addr: &EndpointAddr, file: &str, out: &Path) -> String {
    let me = identity::load(&client.home, &client.store).unwrap();
    let target = AgentList::load(&client.home.join(AGENTS_FILE))
        .unwrap()
        .pick(None)
        .unwrap()
        .clone();
    assert_eq!(target.id, agent_addr.id);
    let key = me.key_for(&client.store, &target).unwrap();
    let ep = local_endpoint_with_key(key, vec![]).await;
    let stats = within(client::get(&ep, agent_addr, file, out))
        .await
        .unwrap();
    ep.close().await;
    stats.sha256
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    pigeon_core::hex(&Sha256::digest(bytes))
}

#[tokio::test(flavor = "multi_thread")]
async fn migration_keeps_pairings_working_in_both_directions() {
    let mac_body = vec![7u8; 300_000];
    let pc_body = b"notes from home".to_vec();
    let mac = old_install("report.bin", &mac_body);
    let pc = old_install("notes.txt", &pc_body);
    old_pairing(&mac, &pc, "Office Mac");
    old_pairing(&pc, &mac, "Home PC");

    // First run of the new build on each computer.
    let mac_me = identity::load(&mac.home, &mac.store).unwrap();
    let pc_me = identity::load(&pc.home, &pc.store).unwrap();
    for (me, c) in [(&mac_me, &mac), (&pc_me, &pc)] {
        assert_eq!(me.origin, Origin::FromAgentKey);
        assert_eq!(me.id(), c.agent_key.public(), "device key = old agent key");
        assert_eq!(me.legacy_client, Some(c.client_key.public()));
    }

    let (mac_agent, mac_addr) = start_agent(&mac).await;
    let (pc_agent, pc_addr) = start_agent(&pc).await;
    let out = tempfile::tempdir().unwrap();

    // PC -> Mac and Mac -> PC still work, through the old client keys.
    let got = fetch(&pc, &mac_addr, "report.bin", &out.path().join("a")).await;
    assert_eq!(got, sha256(&mac_body));
    let got = fetch(&mac, &pc_addr, "notes.txt", &out.path().join("b")).await;
    assert_eq!(got, sha256(&pc_body));

    // Without the fallback (dialing with the device key) the Mac refuses
    // the PC: the fallback is what keeps the old pairing alive.
    let device_ep = local_endpoint_with_key(pc_me.key.clone(), vec![]).await;
    assert!(
        within(client::list(&device_ep, &mac_addr, ""))
            .await
            .is_err()
    );

    // Pair the PC with the Mac again (the real P1 handshake, device key).
    let (invite, code) =
        Invite::new(DEFAULT_TTL, pair::now_secs(), Some("Home PC".into())).unwrap();
    let link = invite.link(mac_addr.clone());
    mac_agent.offer_invite(invite);
    let paired = within(pair::pair(&device_ep, &link, &code)).await.unwrap();
    assert_eq!(paired.agent, mac_me.id());
    assert!(mac_agent.devices().contains(&pc_me.id()));
    // What `pigeon pair` stores: the same agent, now with the device key.
    AgentList::update(&pc.home.join(AGENTS_FILE), |l| {
        let mut a = l.agents[0].clone();
        a.legacy_key = None;
        l.upsert(a);
        Ok(())
    })
    .unwrap();
    let pc_after = identity::load(&pc.home, &pc.store).unwrap();
    assert_eq!(pc_after.legacy_client, None);
    assert!(
        read_key(&pc.store, LEGACY_CLIENT_KEY).unwrap().is_none(),
        "the old client key is dropped once nothing needs it"
    );
    assert!(
        pc_after
            .notes
            .iter()
            .any(|n| n.contains("removed the old client key"))
    );
    let got = fetch(&pc, &mac_addr, "report.bin", &out.path().join("c")).await;
    assert_eq!(got, sha256(&mac_body));
    // The Mac can now drop the old entry (`pigeon-agent revoke <old fp>`).
    mac_agent.revoke(&pc.client_key.public());
    let got = fetch(&pc, &mac_addr, "report.bin", &out.path().join("d")).await;
    assert_eq!(got, sha256(&mac_body));

    // The Mac has not paired again: it still uses its old client key for the PC.
    let got = fetch(&mac, &pc_addr, "notes.txt", &out.path().join("e")).await;
    assert_eq!(got, sha256(&pc_body));
    assert!(read_key(&mac.store, LEGACY_CLIENT_KEY).unwrap().is_some());

    device_ep.close().await;
    mac_agent.endpoint().close().await;
    pc_agent.endpoint().close().await;
}

/// One key for both roles, at the same time: this computer's agent serves
/// another computer while this computer's client (same key, a second
/// endpoint, as when `pigeon get` runs next to `pigeon-agent share`)
/// fetches from that other computer. Over direct paths both work.
/// (Through a relay the newest connection with a key wins; see PAIRING.md D9.)
#[tokio::test(flavor = "multi_thread")]
async fn both_roles_with_one_key_at_once_over_direct_paths() {
    let a = old_install("a.txt", b"from a");
    let b = old_install("b.txt", b"from b");
    let a_me = identity::load(&a.home, &a.store).unwrap();
    let b_me = identity::load(&b.home, &b.store).unwrap();
    // New-style pairing: allow-lists hold device keys.
    for (agent, client) in [(&a, &b_me), (&b, &a_me)] {
        DeviceList::update(&agent.home.join(DEVICES_FILE), |l| {
            l.upsert(Device {
                id: client.id(),
                name: "peer".into(),
                paired_at: 1,
            });
            Ok(())
        })
        .unwrap();
    }
    let (a_agent, a_addr) = start_agent(&a).await;
    let (b_agent, b_addr) = start_agent(&b).await;
    let a_client: Endpoint = local_endpoint_with_key(a_me.key.clone(), vec![]).await;
    let b_client: Endpoint = local_endpoint_with_key(b_me.key.clone(), vec![]).await;
    assert_eq!(a_client.id(), a_agent.endpoint().id(), "one key, two roles");
    let out = tempfile::tempdir().unwrap();
    let (out_x, out_y) = (out.path().join("x"), out.path().join("y"));
    let (x, y) = tokio::join!(
        within(client::get(&a_client, &b_addr, "b.txt", &out_x)),
        within(client::get(&b_client, &a_addr, "a.txt", &out_y)),
    );
    assert_eq!(x.unwrap().sha256, sha256(b"from b"));
    assert_eq!(y.unwrap().sha256, sha256(b"from a"));
    // The pairing ALPN is not offered without an invite, whatever the key.
    assert!(
        within(a_client.connect(b_addr.clone(), PAIR_ALPN))
            .await
            .is_err()
    );
    a_client.close().await;
    b_client.close().await;
    a_agent.endpoint().close().await;
    b_agent.endpoint().close().await;
}
