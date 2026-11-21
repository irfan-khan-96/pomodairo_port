//! A sync node: accepts connections from paired devices, connects to them, and
//! runs pairing. Who a device is is decided by its certificate fingerprint right
//! after the TLS handshake; a connection from an unknown device is closed
//! without sending it anything (unless this device is in pairing mode).
//!
//! A sync session, after both sides said hello:
//!   1. each sends its version vector (what it already has);
//!   2. each sends the events the other lacks (concurrently, so large transfers
//!      can't deadlock), and applies what it receives.

use std::collections::BTreeMap;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio_rustls::{TlsAcceptor, TlsConnector};

use super::identity::{Identity, fingerprint, hex, unhex};
use super::protocol::{
    Mode, Msg, PROTOCOL, commitment, net_err, pairing_code, read_msg, write_msg,
};
use super::tls;
use crate::error::{Error, Result};
use crate::sync::engine::Event;
use crate::sync::hlc::is_device_id;
use crate::sync::peers::Peer;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const STEP_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the user has to compare and confirm the pairing code.
pub const CONFIRM_TIMEOUT: Duration = Duration::from_secs(120);
/// At most this many inbound connections are handled at once.
const MAX_INBOUND: usize = 4;

/// What the node needs from the database.
pub trait SyncStore: Send + Sync + 'static {
    fn version_vector(&self) -> Result<BTreeMap<String, i64>>;
    fn events_missing(&self, theirs: &BTreeMap<String, i64>) -> Result<Vec<Event>>;
    /// Apply received events; returns how many were new.
    fn ingest(&self, events: &[Event]) -> Result<usize>;
    fn peer_by_fingerprint(&self, fingerprint: &str) -> Result<Option<Peer>>;
    fn peer_by_device(&self, device: &str) -> Result<Option<Peer>>;
    fn save_peer(&self, device: &str, name: &str, fingerprint: &str) -> Result<()>;
    /// A sync completed; the peer confirmed having our changes up to `acked_seq`.
    fn mark_synced(&self, device: &str, addr: Option<&str>, acked_seq: i64) -> Result<()>;
}

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Shows the pairing code and asks the user to confirm it matches the other screen.
pub trait PairingUi: Send + Sync + 'static {
    fn confirm(&self, code: String, peer_name: String) -> BoxFuture<bool>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub device: String,
    pub sent: usize,
    pub received: usize,
}

pub struct Node<S: SyncStore> {
    store: Arc<S>,
    identity: Arc<Identity>,
    name: String,
    acceptor: TlsAcceptor,
    connector: TlsConnector,
    ui: Arc<dyn PairingUi>,
    pairing_until: Mutex<Option<Instant>>,
    inbound: Arc<Semaphore>,
}

/// Device names come from other machines: keep them short and printable.
fn clean_name(name: &str) -> String {
    let cleaned: String = name.chars().filter(|c| !c.is_control()).take(64).collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        "Unnamed device".into()
    } else {
        cleaned.to_string()
    }
}

fn random_nonce() -> Result<[u8; 32]> {
    let mut nonce = [0u8; 32];
    tls::provider()
        .secure_random
        .fill(&mut nonce)
        .map_err(|_| Error::Invalid("no secure random source".into()))?;
    Ok(nonce)
}

fn peer_fingerprint(conn: &rustls::CommonState) -> Result<String> {
    let certs = conn
        .peer_certificates()
        .ok_or_else(|| Error::Invalid("peer presented no certificate".into()))?;
    let leaf = certs
        .first()
        .ok_or_else(|| Error::Invalid("peer presented no certificate".into()))?;
    Ok(fingerprint(leaf))
}

impl<S: SyncStore> Node<S> {
    pub fn new(
        store: Arc<S>,
        identity: Identity,
        name: &str,
        ui: Arc<dyn PairingUi>,
    ) -> Result<Arc<Self>> {
        Ok(Arc::new(Node {
            acceptor: TlsAcceptor::from(tls::server_config(&identity)?),
            connector: TlsConnector::from(tls::client_config(&identity)?),
            store,
            identity: Arc::new(identity),
            name: clean_name(name),
            ui,
            pairing_until: Mutex::new(None),
            inbound: Arc::new(Semaphore::new(MAX_INBOUND)),
        }))
    }

    pub fn device_id(&self) -> &str {
        &self.identity.device_id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Accept one pairing request during the next `duration`.
    pub fn open_pairing(&self, duration: Duration) {
        *self.pairing_until.lock().unwrap() = Some(Instant::now() + duration);
    }

    pub fn close_pairing(&self) {
        *self.pairing_until.lock().unwrap() = None;
    }

    pub fn pairing_open(&self) -> bool {
        self.pairing_until
            .lock()
            .unwrap()
            .is_some_and(|t| Instant::now() < t)
    }

    /// Take the pairing window for one attempt.
    fn take_pairing_slot(&self) -> bool {
        let mut slot = self.pairing_until.lock().unwrap();
        let open = slot.is_some_and(|t| Instant::now() < t);
        *slot = None;
        open
    }

    fn hello(&self, mode: Mode) -> Msg {
        Msg::Hello {
            protocol: PROTOCOL,
            device: self.identity.device_id.clone(),
            name: self.name.clone(),
            mode,
        }
    }

    /// Start accepting connections on `addr`; returns the bound address.
    pub async fn listen(self: &Arc<Self>, addr: SocketAddr) -> Result<SocketAddr> {
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| net_err("cannot listen", e))?;
        let local = listener
            .local_addr()
            .map_err(|e| net_err("cannot listen", e))?;
        let node = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                let Ok((tcp, remote)) = listener.accept().await else {
                    continue;
                };
                let Ok(permit) = Arc::clone(&node.inbound).try_acquire_owned() else {
                    continue; // busy: drop the connection
                };
                let node = Arc::clone(&node);
                tokio::spawn(async move {
                    let _permit = permit;
                    let limit = CONFIRM_TIMEOUT + Duration::from_secs(60);
                    match tokio::time::timeout(limit, node.handle_inbound(tcp)).await {
                        Ok(Err(e)) => eprintln!("sync: connection from {remote} ended: {e}"),
                        Err(_) => eprintln!("sync: connection from {remote} timed out"),
                        Ok(Ok(())) => {}
                    }
                });
            }
        });
        Ok(local)
    }

    async fn handle_inbound(&self, tcp: TcpStream) -> Result<()> {
        let mut tls = tokio::time::timeout(STEP_TIMEOUT, self.acceptor.accept(tcp))
            .await
            .map_err(|_| Error::Invalid("handshake timed out".into()))?
            .map_err(|e| net_err("handshake failed", e))?;
        let fp = peer_fingerprint(tls.get_ref().1)?;
        let known = self.store.peer_by_fingerprint(&fp)?;
        if known.is_none() && !self.pairing_open() {
            // Unknown device and not pairing: say nothing at all.
            let _ = tls.shutdown().await;
            return Err(Error::Invalid("unknown device".into()));
        }
        let Msg::Hello {
            protocol,
            device,
            name,
            mode,
        } = read_msg(&mut tls, STEP_TIMEOUT).await?
        else {
            return Err(Error::Invalid("expected hello".into()));
        };
        if protocol != PROTOCOL {
            let reason = format!("protocol {protocol} not supported");
            write_msg(
                &mut tls,
                &Msg::Reject {
                    reason: reason.clone(),
                },
            )
            .await?;
            return Err(Error::Invalid(reason));
        }
        match mode {
            Mode::Sync => {
                let Some(peer) = known.filter(|p| p.device == device) else {
                    let _ = tls.shutdown().await;
                    return Err(Error::Invalid(
                        "device does not match its certificate".into(),
                    ));
                };
                write_msg(&mut tls, &self.hello(Mode::Sync)).await?;
                let done = exchange(&mut tls, self.store.as_ref(), self.device_id()).await?;
                self.store.mark_synced(&peer.device, None, done.acked)?;
                Ok(())
            }
            Mode::Pair => {
                if !is_device_id(&device)
                    || device == self.identity.device_id
                    || !self.take_pairing_slot()
                {
                    write_msg(
                        &mut tls,
                        &Msg::Reject {
                            reason: "not accepting pairing".into(),
                        },
                    )
                    .await?;
                    return Err(Error::Invalid("unexpected pairing request".into()));
                }
                write_msg(&mut tls, &self.hello(Mode::Pair)).await?;
                // Responder: receive the commitment, send our nonce, check the reveal.
                let Msg::Commit { hash } = read_msg(&mut tls, STEP_TIMEOUT).await? else {
                    return Err(Error::Invalid("expected commitment".into()));
                };
                let ours = random_nonce()?;
                write_msg(&mut tls, &Msg::Nonce { nonce: hex(&ours) }).await?;
                let Msg::Reveal { nonce } = read_msg(&mut tls, STEP_TIMEOUT).await? else {
                    return Err(Error::Invalid("expected reveal".into()));
                };
                let theirs = unhex(&nonce).filter(|n| n.len() == 32 && commitment(n) == hash);
                let Some(theirs) = theirs else {
                    return Err(Error::Invalid("pairing commitment did not match".into()));
                };
                let code = pairing_code(&fp, &self.identity.fingerprint, &theirs, &ours);
                let name = clean_name(&name);
                self.confirm_both(&mut tls, code, name.clone()).await?;
                self.store.save_peer(&device, &name, &fp)?;
                let done = exchange(&mut tls, self.store.as_ref(), self.device_id()).await?;
                self.store.mark_synced(&device, None, done.acked)?;
                Ok(())
            }
        }
    }

    /// Both users confirm the code; either side declining cancels pairing.
    async fn confirm_both<IO: AsyncRead + AsyncWrite + Unpin>(
        &self,
        io: &mut IO,
        code: String,
        peer_name: String,
    ) -> Result<()> {
        let local = tokio::time::timeout(CONFIRM_TIMEOUT, self.ui.confirm(code, peer_name))
            .await
            .unwrap_or(false);
        write_msg(io, &Msg::Confirm { accept: local }).await?;
        let remote = match read_msg(io, CONFIRM_TIMEOUT + Duration::from_secs(10)).await? {
            Msg::Confirm { accept } => accept,
            _ => false,
        };
        if local && remote {
            Ok(())
        } else {
            Err(Error::Invalid("pairing was cancelled".into()))
        }
    }

    async fn connect(
        &self,
        addr: SocketAddr,
    ) -> Result<(tokio_rustls::client::TlsStream<TcpStream>, String)> {
        let tcp = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
            .await
            .map_err(|_| Error::Invalid(format!("{addr} is not reachable")))?
            .map_err(|e| net_err("cannot connect", e))?;
        let name = rustls::pki_types::ServerName::try_from(tls::SERVER_NAME).expect("valid name");
        let tls = tokio::time::timeout(STEP_TIMEOUT, self.connector.connect(name, tcp))
            .await
            .map_err(|_| Error::Invalid("handshake timed out".into()))?
            .map_err(|e| net_err("handshake failed", e))?;
        let fp = peer_fingerprint(tls.get_ref().1)?;
        Ok((tls, fp))
    }

    /// Sync with the paired device `device` at `addr`.
    pub async fn sync_with(&self, addr: SocketAddr, device: &str) -> Result<Outcome> {
        let peer = self
            .store
            .peer_by_device(device)?
            .ok_or_else(|| Error::Invalid("device is not paired".into()))?;
        let (mut tls, fp) = self.connect(addr).await?;
        if fp != peer.fingerprint {
            // Not the device we paired with: send nothing.
            let _ = tls.shutdown().await;
            return Err(Error::Invalid("device identity changed; pair again".into()));
        }
        write_msg(&mut tls, &self.hello(Mode::Sync)).await?;
        match read_msg(&mut tls, STEP_TIMEOUT).await? {
            Msg::Hello {
                device: d,
                mode: Mode::Sync,
                ..
            } if d == peer.device => {}
            Msg::Reject { reason } => return Err(Error::Invalid(reason)),
            _ => return Err(Error::Invalid("unexpected answer".into())),
        }
        let done = exchange(&mut tls, self.store.as_ref(), self.device_id()).await?;
        self.store
            .mark_synced(&peer.device, Some(&addr.to_string()), done.acked)?;
        Ok(Outcome {
            device: peer.device,
            sent: done.sent,
            received: done.received,
        })
    }

    /// Pair with the device at `addr` (which must be in pairing mode), then sync.
    pub async fn pair_with(&self, addr: SocketAddr) -> Result<Peer> {
        let (mut tls, fp) = self.connect(addr).await?;
        write_msg(&mut tls, &self.hello(Mode::Pair)).await?;
        // A device that isn't in pairing mode hangs up without a word.
        let not_pairing = || {
            Error::Invalid(
                "the other device isn't accepting pairing; choose \"Pair a new device\" on it first".into(),
            )
        };
        let answer = read_msg(&mut tls, STEP_TIMEOUT)
            .await
            .map_err(|_| not_pairing())?;
        let (device, name) = match answer {
            Msg::Hello {
                device,
                name,
                mode: Mode::Pair,
                protocol: PROTOCOL,
            } => (device, clean_name(&name)),
            Msg::Reject { .. } => return Err(not_pairing()),
            _ => return Err(Error::Invalid("unexpected answer".into())),
        };
        if !is_device_id(&device) || device == self.identity.device_id {
            return Err(Error::Invalid("invalid device".into()));
        }
        // Initiator: commit to our nonce before seeing theirs.
        let ours = random_nonce()?;
        write_msg(
            &mut tls,
            &Msg::Commit {
                hash: commitment(&ours),
            },
        )
        .await?;
        let Msg::Nonce { nonce } = read_msg(&mut tls, STEP_TIMEOUT).await? else {
            return Err(Error::Invalid("expected nonce".into()));
        };
        let theirs = unhex(&nonce)
            .filter(|n| n.len() == 32)
            .ok_or_else(|| Error::Invalid("invalid nonce".into()))?;
        write_msg(&mut tls, &Msg::Reveal { nonce: hex(&ours) }).await?;
        let code = pairing_code(&self.identity.fingerprint, &fp, &ours, &theirs);
        self.confirm_both(&mut tls, code, name.clone()).await?;
        self.store.save_peer(&device, &name, &fp)?;
        let done = exchange(&mut tls, self.store.as_ref(), self.device_id()).await?;
        self.store
            .mark_synced(&device, Some(&addr.to_string()), done.acked)?;
        self.store
            .peer_by_device(&device)?
            .ok_or_else(|| Error::Invalid("pairing was not saved".into()))
    }
}

struct Exchanged {
    sent: usize,
    received: usize,
    /// Our own sequence number the peer has confirmed applying.
    acked: i64,
}

/// Swap version vectors, send and receive missing events concurrently, apply
/// them, then confirm to each other that they were applied.
async fn exchange<IO, S>(io: &mut IO, store: &S, me: &str) -> Result<Exchanged>
where
    IO: AsyncRead + AsyncWrite + Unpin,
    S: SyncStore + ?Sized,
{
    let ours_vv = store.version_vector()?;
    write_msg(
        io,
        &Msg::Vector {
            vv: ours_vv.clone(),
        },
    )
    .await?;
    let Msg::Vector { vv: theirs } = read_msg(io, STEP_TIMEOUT).await? else {
        return Err(Error::Invalid("expected version vector".into()));
    };
    let outgoing = Msg::Events {
        events: store.events_missing(&theirs)?,
    };
    let sent = match &outgoing {
        Msg::Events { events } => events.len(),
        _ => 0,
    };
    let received = {
        let (mut r, mut w) = tokio::io::split(&mut *io);
        let (sent_res, received) = tokio::join!(
            write_msg(&mut w, &outgoing),
            read_msg(&mut r, Duration::from_secs(120)),
        );
        sent_res?;
        let Msg::Events { events } = received? else {
            return Err(Error::Invalid("expected events".into()));
        };
        store.ingest(&events)?
    };
    // Both sides applied what they received: now it counts as delivered.
    write_msg(io, &Msg::Applied).await?;
    let Msg::Applied = read_msg(io, STEP_TIMEOUT).await? else {
        return Err(Error::Invalid("peer did not confirm the sync".into()));
    };
    Ok(Exchanged {
        sent,
        received,
        acked: ours_vv.get(me).copied().unwrap_or(0),
    })
}
