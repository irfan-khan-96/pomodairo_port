//! Device sync inside the app: runs the sync node and discovery, keeps paired
//! devices up to date in the background, and offers commands for pairing.
//!
//! Nothing listens on the network until the user starts pairing, or devices are
//! already paired. Syncs run after every local change (a short debounce), when a
//! paired device appears on the network, and every 30 seconds as a safety net.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::{Notify, oneshot};

use crate::error::{Error, Result};
use crate::net::discovery::{Discovery, Seen};
use crate::net::identity::{self, OsSecretStore};
use crate::net::node::{BoxFuture, CONFIRM_TIMEOUT, Node, PairingUi, SyncStore};
use crate::sync::engine::{self, Event};
use crate::sync::peers::{self, Peer};
use crate::task_service::TaskService;

pub const STATUS_EVENT: &str = "sync://status";
pub const PAIRING_EVENT: &str = "sync://pairing";
const PERIODIC: Duration = Duration::from_secs(30);
const DEBOUNCE: Duration = Duration::from_millis(400);

/// The app's database, as the sync node sees it.
pub struct AppStore {
    app: AppHandle,
}

impl AppStore {
    fn tasks(&self) -> State<'_, TaskService> {
        self.app.state::<TaskService>()
    }
}

impl SyncStore for AppStore {
    fn version_vector(&self) -> Result<std::collections::BTreeMap<String, i64>> {
        self.tasks().read(engine::version_vector)
    }
    fn events_missing(
        &self,
        theirs: &std::collections::BTreeMap<String, i64>,
    ) -> Result<Vec<Event>> {
        self.tasks().read(|c| engine::events_missing(c, theirs))
    }
    fn ingest(&self, events: &[Event]) -> Result<usize> {
        let new = self.tasks().ingest_remote(&self.app, events)?;
        if new > 0 {
            // Pass the news on to other paired devices.
            if let Some(svc) = self.app.try_state::<SyncService>() {
                svc.wake.notify_one();
            }
        }
        Ok(new)
    }
    fn peer_by_fingerprint(&self, fp: &str) -> Result<Option<Peer>> {
        self.tasks().read(|c| peers::by_fingerprint(c, fp))
    }
    fn peer_by_device(&self, device: &str) -> Result<Option<Peer>> {
        self.tasks().read(|c| peers::by_device(c, device))
    }
    fn save_peer(&self, device: &str, name: &str, fp: &str) -> Result<()> {
        self.tasks()
            .read(|c| peers::save(c, device, name, fp, chrono::Utc::now()))
    }
    fn mark_synced(&self, device: &str, addr: Option<&str>, acked: i64) -> Result<()> {
        self.tasks()
            .read(|c| peers::mark_synced(c, device, addr, acked, chrono::Utc::now()))?;
        publish(&self.app);
        Ok(())
    }
}

/// Asks the user (through the UI) whether the pairing codes match.
struct AppPairingUi {
    app: AppHandle,
    pending: Mutex<Option<oneshot::Sender<bool>>>,
}

#[derive(Clone, Serialize)]
struct PairingPrompt {
    code: String,
    peer_name: String,
    timeout_secs: u64,
}

impl PairingUi for AppPairingUi {
    fn confirm(&self, code: String, peer_name: String) -> BoxFuture<bool> {
        let (tx, rx) = oneshot::channel();
        *self.pending.lock().unwrap() = Some(tx);
        let _ = self.app.emit(
            PAIRING_EVENT,
            PairingPrompt {
                code,
                peer_name,
                timeout_secs: CONFIRM_TIMEOUT.as_secs(),
            },
        );
        Box::pin(async move { rx.await.unwrap_or(false) })
    }
}

/// Devices seen on the network: their addresses, and their name while pairing.
type SeenMap = HashMap<String, (Vec<SocketAddr>, Option<String>)>;

struct Running {
    discovery: Option<Discovery>,
}

pub struct SyncService {
    app: AppHandle,
    node: Arc<Node<AppStore>>,
    ui: Arc<AppPairingUi>,
    running: tokio::sync::Mutex<Option<Running>>,
    /// Devices seen on the network: addresses, and a name while they are pairing.
    seen: Arc<Mutex<SeenMap>>,
    wake: Arc<Notify>,
    last_error: Mutex<Option<String>>,
}

/// This computer's name, as shown on other devices.
fn device_name() -> String {
    if cfg!(debug_assertions)
        && let Ok(name) = std::env::var("POMODAIRO_NAME")
    {
        return name;
    }
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Pomodairo device".into())
}

impl SyncService {
    pub fn new(app: &AppHandle) -> Result<Self> {
        let fallback_dir = app
            .path()
            .app_config_dir()
            .map_err(|e| Error::Invalid(e.to_string()))?;
        let secrets = OsSecretStore { fallback_dir };
        let identity = app
            .state::<TaskService>()
            .read(|c| identity::load_or_create(c, &secrets))?;
        let ui = Arc::new(AppPairingUi {
            app: app.clone(),
            pending: Mutex::new(None),
        });
        let node = Node::new(
            Arc::new(AppStore { app: app.clone() }),
            identity,
            &device_name(),
            ui.clone(),
        )?;
        Ok(SyncService {
            app: app.clone(),
            node,
            ui,
            running: tokio::sync::Mutex::new(None),
            seen: Arc::new(Mutex::new(HashMap::new())),
            wake: Arc::new(Notify::new()),
            last_error: Mutex::new(None),
        })
    }

    fn has_peers(&self) -> bool {
        self.app
            .state::<TaskService>()
            .read(peers::list)
            .is_ok_and(|p| !p.is_empty())
    }

    /// Start listening, discovery and the background loop (once).
    async fn ensure_running(&self) -> Result<()> {
        let mut running = self.running.lock().await;
        if running.is_some() {
            return Ok(());
        }
        let port = self
            .node
            .listen("0.0.0.0:0".parse().expect("valid"))
            .await?
            .port();
        let seen = Arc::clone(&self.seen);
        let wake = Arc::clone(&self.wake);
        let app = self.app.clone();
        let discovery = match Discovery::start(self.node.device_id(), port, move |event| {
            {
                let mut seen = seen.lock().unwrap();
                match event {
                    Seen::Found {
                        device,
                        addrs,
                        pairing_name,
                    } => {
                        seen.insert(device, (addrs, pairing_name));
                    }
                    Seen::Lost { device } => {
                        seen.remove(&device);
                    }
                }
            }
            wake.notify_one();
            publish(&app);
        }) {
            Ok(d) => Some(d),
            Err(e) => {
                // Sync can still work with remembered addresses.
                self.set_error(Some(e.to_string()));
                None
            }
        };
        *running = Some(Running { discovery });
        drop(running);

        let app = self.app.clone();
        tauri::async_runtime::spawn(async move {
            let svc = app.state::<SyncService>();
            loop {
                tokio::select! {
                    _ = svc.wake.notified() => tokio::time::sleep(DEBOUNCE).await,
                    _ = tokio::time::sleep(PERIODIC) => {}
                }
                svc.sync_all().await;
            }
        });
        Ok(())
    }

    fn set_error(&self, error: Option<String>) {
        *self.last_error.lock().unwrap() = error;
    }

    /// Where a paired device can be reached: what discovery saw, then the last address that worked.
    fn addresses(&self, peer: &Peer) -> Vec<SocketAddr> {
        let mut addrs = self
            .seen
            .lock()
            .unwrap()
            .get(&peer.device)
            .map(|(a, _)| a.clone())
            .unwrap_or_default();
        if let Some(last) = peer.last_addr.as_deref().and_then(|a| a.parse().ok())
            && !addrs.contains(&last)
        {
            addrs.push(last);
        }
        addrs
    }

    async fn sync_all(&self) {
        let peers = self
            .app
            .state::<TaskService>()
            .read(peers::list)
            .unwrap_or_default();
        let mut error = None;
        for peer in peers {
            let mut synced = false;
            for addr in self.addresses(&peer) {
                match self.node.sync_with(addr, &peer.device).await {
                    Ok(_) => {
                        synced = true;
                        break;
                    }
                    Err(e) => error = Some(format!("{}: {e}", peer.name)),
                }
            }
            if synced {
                error = None;
            }
        }
        self.set_error(error);
        publish(&self.app);
    }

    fn status(&self) -> Result<SyncStatus> {
        let peers = self.app.state::<TaskService>().read(peers::list)?;
        let seen = self.seen.lock().unwrap().clone();
        let paired: Vec<PeerStatus> = peers
            .iter()
            .map(|p| PeerStatus {
                device: p.device.clone(),
                name: p.name.clone(),
                last_sync: p.last_sync.clone(),
                online: seen.contains_key(&p.device),
                pending: self
                    .app
                    .state::<TaskService>()
                    .read(|c| peers::pending(c, self.node.device_id(), p))
                    .unwrap_or(0),
            })
            .collect();
        let pending_total = paired.iter().map(|p| p.pending).sum();
        let nearby = seen
            .into_iter()
            .filter(|(d, _)| !peers.iter().any(|p| &p.device == d))
            .filter_map(|(device, (_, name))| name.map(|name| Nearby { device, name }))
            .collect();
        Ok(SyncStatus {
            device_id: self.node.device_id().to_string(),
            name: self.node.name().to_string(),
            pairing_open: self.node.pairing_open(),
            pending_total,
            peers: paired,
            nearby,
            last_error: self.last_error.lock().unwrap().clone(),
        })
    }
}

/// Start sync at app launch if devices are already paired.
pub fn init(app: &AppHandle) -> Result<()> {
    let svc = SyncService::new(app)?;
    let start = svc.has_peers();
    app.manage(svc);
    publish(app);
    if start {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = app.state::<SyncService>().ensure_running().await {
                eprintln!("sync: cannot start: {e}");
            }
        });
    }
    Ok(())
}

/// A local change happened: show it as not yet handed off, and sync soon.
pub fn changed(app: &AppHandle) {
    if let Some(svc) = app.try_state::<SyncService>() {
        svc.wake.notify_one();
        publish(app);
    }
}

/// Send the current sync status to the UI and the tray.
fn publish(app: &AppHandle) {
    let Some(svc) = app.try_state::<SyncService>() else {
        return;
    };
    let Ok(status) = svc.status() else { return };
    crate::tray::set_sync(app, tray_note(&status));
    let _ = app.emit(STATUS_EVENT, &status);
}

/// The tray tooltip line: nothing until a device is paired.
fn tray_note(status: &SyncStatus) -> Option<(String, bool)> {
    if status.peers.is_empty() {
        return None;
    }
    let waiting: Vec<&str> = status
        .peers
        .iter()
        .filter(|p| p.pending > 0)
        .map(|p| p.name.as_str())
        .collect();
    Some(if waiting.is_empty() {
        ("All changes synced".to_string(), false)
    } else {
        let n = status.pending_total;
        let s = if n == 1 { "" } else { "s" };
        (
            format!("{n} change{s} not yet on {}", waiting.join(", ")),
            true,
        )
    })
}

#[derive(Serialize)]
pub struct PeerStatus {
    device: String,
    name: String,
    last_sync: Option<String>,
    online: bool,
    /// This device's changes the peer hasn't confirmed yet.
    pending: i64,
}

#[derive(Serialize)]
pub struct Nearby {
    device: String,
    name: String,
}

#[derive(Serialize)]
pub struct SyncStatus {
    device_id: String,
    name: String,
    pairing_open: bool,
    /// Changes made here that some paired device doesn't have yet.
    pending_total: i64,
    peers: Vec<PeerStatus>,
    /// Unpaired devices nearby that are waiting to pair.
    nearby: Vec<Nearby>,
    last_error: Option<String>,
}

#[tauri::command]
pub fn sync_status(svc: State<'_, SyncService>) -> Result<SyncStatus> {
    svc.status()
}

/// Let another device pair with this one for the next two minutes.
#[tauri::command]
pub async fn sync_open_pairing(svc: State<'_, SyncService>) -> Result<SyncStatus> {
    svc.ensure_running().await?;
    svc.node.open_pairing(CONFIRM_TIMEOUT);
    if let Some(Running {
        discovery: Some(d), ..
    }) = svc.running.lock().await.as_ref()
    {
        d.advertise(Some(svc.node.name()))?;
    }
    // Stop advertising the name once the window closes.
    let app = svc.app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(CONFIRM_TIMEOUT).await;
        let svc = app.state::<SyncService>();
        if !svc.node.pairing_open()
            && let Some(Running {
                discovery: Some(d), ..
            }) = svc.running.lock().await.as_ref()
        {
            let _ = d.advertise(None);
        }
        publish(&app);
    });
    svc.status()
}

/// Stop waiting for another device to pair.
#[tauri::command]
pub async fn sync_cancel_pairing(svc: State<'_, SyncService>) -> Result<SyncStatus> {
    svc.node.close_pairing();
    sync_confirm_pairing(svc.clone(), false);
    if let Some(Running { discovery: Some(d) }) = svc.running.lock().await.as_ref() {
        d.advertise(None)?;
    }
    svc.status()
}

/// Start looking for nearby devices (so ones waiting to pair can be listed).
#[tauri::command]
pub async fn sync_discover(svc: State<'_, SyncService>) -> Result<SyncStatus> {
    svc.ensure_running().await?;
    svc.status()
}

/// Pair with a nearby device that is waiting to pair.
#[tauri::command]
pub async fn sync_pair_with(svc: State<'_, SyncService>, device: String) -> Result<SyncStatus> {
    svc.ensure_running().await?;
    let addrs = svc
        .seen
        .lock()
        .unwrap()
        .get(&device)
        .map(|(a, _)| a.clone())
        .ok_or_else(|| Error::Invalid("that device is no longer visible on the network".into()))?;
    let mut last = Error::Invalid("that device has no reachable address".into());
    for addr in addrs {
        match svc.node.pair_with(addr).await {
            Ok(_) => {
                svc.wake.notify_one();
                return svc.status();
            }
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Answer the pairing prompt: do the codes on both screens match?
#[tauri::command]
pub fn sync_confirm_pairing(svc: State<'_, SyncService>, accept: bool) {
    if let Some(tx) = svc.ui.pending.lock().unwrap().take() {
        let _ = tx.send(accept);
    }
}

#[tauri::command]
pub fn sync_unpair(svc: State<'_, SyncService>, device: String) -> Result<SyncStatus> {
    svc.app
        .state::<TaskService>()
        .read(|c| peers::remove(c, &device))?;
    publish(&svc.app);
    svc.status()
}

#[tauri::command]
pub fn sync_now(svc: State<'_, SyncService>) {
    svc.wake.notify_one();
}
