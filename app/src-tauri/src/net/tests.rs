//! Real nodes talking TLS over localhost, each with its own in-memory database.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use rusqlite::Connection;

use super::identity::Identity;
use super::node::{BoxFuture, Node, PairingUi, SyncStore};
use crate::db;
use crate::error::Result;
use crate::sync::engine::{self, Event};
use crate::sync::peers::{self, Peer};
use crate::tasks;

struct TestStore(Mutex<Connection>);

impl TestStore {
    fn new() -> Arc<Self> {
        Arc::new(TestStore(Mutex::new(db::open_in_memory().unwrap())))
    }
    fn device(&self) -> String {
        engine::device_id(&self.0.lock().unwrap()).unwrap()
    }
    fn names(&self) -> Vec<String> {
        let mut n: Vec<String> = tasks::list(&self.0.lock().unwrap())
            .unwrap()
            .into_iter()
            .map(|t| t.name)
            .collect();
        n.sort();
        n
    }
    fn create(&self, name: &str) -> i64 {
        tasks::create(&self.0.lock().unwrap(), name, Some(1), Utc::now())
            .unwrap()
            .id
    }
    fn event_count(&self) -> i64 {
        self.0
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM sync_event", [], |r| r.get(0))
            .unwrap()
    }
    fn peers(&self) -> Vec<Peer> {
        peers::list(&self.0.lock().unwrap()).unwrap()
    }
}

impl SyncStore for TestStore {
    fn version_vector(&self) -> Result<BTreeMap<String, i64>> {
        engine::version_vector(&self.0.lock().unwrap())
    }
    fn events_missing(&self, theirs: &BTreeMap<String, i64>) -> Result<Vec<Event>> {
        engine::events_missing(&self.0.lock().unwrap(), theirs)
    }
    fn ingest(&self, events: &[Event]) -> Result<usize> {
        engine::ingest(&self.0.lock().unwrap(), events)
    }
    fn peer_by_fingerprint(&self, fp: &str) -> Result<Option<Peer>> {
        peers::by_fingerprint(&self.0.lock().unwrap(), fp)
    }
    fn peer_by_device(&self, device: &str) -> Result<Option<Peer>> {
        peers::by_device(&self.0.lock().unwrap(), device)
    }
    fn save_peer(&self, device: &str, name: &str, fp: &str) -> Result<()> {
        peers::save(&self.0.lock().unwrap(), device, name, fp, Utc::now())
    }
    fn mark_synced(&self, device: &str, addr: Option<&str>) -> Result<()> {
        peers::mark_synced(&self.0.lock().unwrap(), device, addr, Utc::now())
    }
}

/// Answers the pairing prompt automatically and remembers the codes it saw.
struct AutoUi {
    accept: bool,
    seen: Mutex<Vec<String>>,
}

impl AutoUi {
    fn new(accept: bool) -> Arc<Self> {
        Arc::new(AutoUi {
            accept,
            seen: Mutex::new(Vec::new()),
        })
    }
}

impl PairingUi for AutoUi {
    fn confirm(&self, code: String, _peer: String) -> BoxFuture<bool> {
        self.seen.lock().unwrap().push(code);
        let accept = self.accept;
        Box::pin(async move { accept })
    }
}

struct Device {
    store: Arc<TestStore>,
    node: Arc<Node<TestStore>>,
    ui: Arc<AutoUi>,
    addr: SocketAddr,
}

async fn device(name: &str, accept: bool) -> Device {
    device_with(TestStore::new(), name, accept, None).await
}

async fn device_with(
    store: Arc<TestStore>,
    name: &str,
    accept: bool,
    identity: Option<Identity>,
) -> Device {
    let identity = identity.unwrap_or_else(|| Identity::generate(&store.device()).unwrap());
    let ui = AutoUi::new(accept);
    let node = Node::new(Arc::clone(&store), identity, name, ui.clone()).unwrap();
    let addr = node.listen("127.0.0.1:0".parse().unwrap()).await.unwrap();
    Device {
        store,
        node,
        ui,
        addr,
    }
}

async fn paired(a_name: &str, b_name: &str) -> (Device, Device) {
    let (a, b) = (device(a_name, true).await, device(b_name, true).await);
    b.node.open_pairing(std::time::Duration::from_secs(30));
    a.node.pair_with(b.addr).await.unwrap();
    (a, b)
}

#[tokio::test]
async fn pairing_shows_the_same_code_and_syncs_both_lists() {
    let (a, b) = (device("Desktop", true).await, device("Laptop", true).await);
    a.store.create("From desktop");
    b.store.create("From laptop");
    b.node.open_pairing(std::time::Duration::from_secs(30));
    let peer = a.node.pair_with(b.addr).await.unwrap();

    assert_eq!(peer.name, "Laptop");
    let (ca, cb) = (
        a.ui.seen.lock().unwrap().clone(),
        b.ui.seen.lock().unwrap().clone(),
    );
    assert_eq!(ca.len(), 1);
    assert_eq!(ca, cb, "both screens show the same code");
    assert_eq!(a.store.peers()[0].device, b.store.device());
    assert_eq!(b.store.peers()[0].device, a.store.device());
    assert_eq!(a.store.names(), ["From desktop", "From laptop"]);
    assert_eq!(b.store.names(), a.store.names());
}

#[tokio::test]
async fn paired_devices_keep_syncing_changes_both_ways() {
    let (a, b) = paired("Desktop", "Laptop").await;
    let id = a.store.create("Write report");
    let out = a.node.sync_with(b.addr, &b.store.device()).await.unwrap();
    assert_eq!((out.sent, out.received), (1, 0));
    assert_eq!(b.store.names(), ["Write report"]);

    // Edit on the laptop, sync from the laptop's side this time.
    let b_id = b
        .store
        .0
        .lock()
        .unwrap()
        .query_row("SELECT id FROM task WHERE name = 'Write report'", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap();
    tasks::update(
        &b.store.0.lock().unwrap(),
        b_id,
        "Write final report",
        Some(2),
        Utc::now(),
    )
    .unwrap();
    b.node.sync_with(a.addr, &a.store.device()).await.unwrap();
    assert_eq!(
        tasks::get(&a.store.0.lock().unwrap(), id).unwrap().name,
        "Write final report"
    );
    assert!(a.store.peers()[0].last_sync.is_some());
}

#[tokio::test]
async fn pairing_is_refused_unless_the_other_device_is_in_pairing_mode() {
    let (a, b) = (device("Desktop", true).await, device("Laptop", true).await);
    let err = a.node.pair_with(b.addr).await.unwrap_err();
    assert!(err.to_string().contains("isn't accepting pairing"), "{err}");
    assert!(a.store.peers().is_empty() && b.store.peers().is_empty());

    // The window allows exactly one attempt.
    b.node.open_pairing(std::time::Duration::from_secs(30));
    a.node.pair_with(b.addr).await.unwrap();
    let c = device("Stranger", true).await;
    assert!(c.node.pair_with(b.addr).await.is_err());
}

#[tokio::test]
async fn declining_on_either_screen_cancels_pairing() {
    for (accept_a, accept_b) in [(false, true), (true, false)] {
        let (a, b) = (
            device("Desktop", accept_a).await,
            device("Laptop", accept_b).await,
        );
        a.store.create("Private task");
        b.node.open_pairing(std::time::Duration::from_secs(30));
        assert!(a.node.pair_with(b.addr).await.is_err());
        assert!(a.store.peers().is_empty() && b.store.peers().is_empty());
        assert!(
            b.store.names().is_empty(),
            "nothing is shared before both confirm"
        );
    }
}

#[tokio::test]
async fn an_unpaired_device_gets_nothing() {
    let (a, b) = paired("Desktop", "Laptop").await;
    b.store.create("Secret plans");
    // A stranger that knows the laptop's public certificate and device id (both
    // visible to anyone during a handshake) records it as a "peer"...
    let stranger = device("Stranger", true).await;
    let b_fp = b.store.peers()[0].fingerprint.clone(); // any fingerprint string works for the lookup
    let b_cert_fp = a.store.peers()[0].fingerprint.clone();
    assert_ne!(b_fp, b_cert_fp);
    stranger
        .store
        .save_peer(&b.store.device(), "Laptop", &b_cert_fp)
        .unwrap();
    // ...but the laptop doesn't know the stranger's certificate, so it hangs up.
    assert!(
        stranger
            .node
            .sync_with(b.addr, &b.store.device())
            .await
            .is_err()
    );
    assert!(stranger.store.names().is_empty(), "no data leaked");
    let _ = a;
}

#[tokio::test]
async fn a_device_pretending_to_be_a_paired_one_is_refused() {
    let (a, b) = paired("Desktop", "Laptop").await;
    b.store.create("Secret plans");
    // An impostor with the desktop's device id but its own key pair.
    let impostor_store = TestStore::new();
    let fake = Identity::generate(&a.store.device()).unwrap();
    let impostor = device_with(impostor_store, "Desktop", true, Some(fake)).await;
    let b_cert_fp = a.store.peers()[0].fingerprint.clone();
    impostor
        .store
        .save_peer(&b.store.device(), "Laptop", &b_cert_fp)
        .unwrap();
    assert!(
        impostor
            .node
            .sync_with(b.addr, &b.store.device())
            .await
            .is_err()
    );
    assert!(impostor.store.names().is_empty());
}

#[tokio::test]
async fn a_peer_whose_certificate_changed_is_not_trusted() {
    let (a, b) = paired("Desktop", "Laptop").await;
    a.store.create("Do not send to an impostor");
    let events_before = b.store.event_count();
    // Something answers at the laptop's id with a different key (e.g. an impostor
    // on the network, or the laptop was reset): the desktop refuses to talk.
    let imposter = device_with(Arc::clone(&b.store), "Laptop", true, None).await;
    let err = a
        .node
        .sync_with(imposter.addr, &b.store.device())
        .await
        .unwrap_err();
    assert!(err.to_string().contains("identity changed"), "{err}");
    assert_eq!(b.store.event_count(), events_before, "nothing was sent");
}

#[tokio::test]
async fn unpairing_stops_sync() {
    let (a, b) = paired("Desktop", "Laptop").await;
    peers::remove(&b.store.0.lock().unwrap(), &a.store.device()).unwrap();
    a.store.create("After unpairing");
    assert!(a.node.sync_with(b.addr, &b.store.device()).await.is_err());
    assert!(b.store.names().is_empty());
}

#[tokio::test]
async fn three_devices_relay_changes_through_each_other() {
    let (a, b) = paired("Desktop", "Laptop").await;
    let c = device("Tablet", true).await;
    c.node.open_pairing(std::time::Duration::from_secs(30));
    b.node.pair_with(c.addr).await.unwrap();
    // The desktop and tablet never talk directly.
    a.store.create("Made on desktop");
    a.node.sync_with(b.addr, &b.store.device()).await.unwrap();
    b.node.sync_with(c.addr, &c.store.device()).await.unwrap();
    assert_eq!(c.store.names(), ["Made on desktop"]);
}
