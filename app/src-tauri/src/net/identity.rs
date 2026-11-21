//! This device's TLS identity: a self-signed certificate and its private key.
//!
//! The certificate is public and kept in the database; peers pin its SHA-256
//! fingerprint when pairing. The private key never leaves this device: it is
//! stored in the Windows Credential Manager / macOS Keychain, or on Linux in a
//! file only the user can read.

use std::path::PathBuf;

use rusqlite::Connection;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

pub struct Identity {
    pub device_id: String,
    pub cert: CertificateDer<'static>,
    key: Vec<u8>, // PKCS#8 DER
    pub fingerprint: String,
}

impl Identity {
    pub fn key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(self.key.clone()))
    }

    /// A fresh identity (ECDSA P-256).
    pub fn generate(device_id: &str) -> Result<Identity> {
        let ck = rcgen::generate_simple_self_signed(vec!["pomodairo.local".to_string()])
            .map_err(|e| Error::Invalid(format!("cannot create device certificate: {e}")))?;
        let cert = ck.cert.der().clone();
        Ok(Identity {
            device_id: device_id.to_string(),
            fingerprint: fingerprint(&cert),
            cert,
            key: ck.signing_key.serialize_der(),
        })
    }
}

/// Hex SHA-256 of a DER certificate.
pub fn fingerprint(cert_der: &[u8]) -> String {
    hex(&Sha256::digest(cert_der))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// Where private keys are kept.
pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Option<Vec<u8>>;
    fn set(&self, name: &str, secret: &[u8]) -> Result<()>;
}

/// The platform's secure storage.
pub struct OsSecretStore {
    /// Used on Linux (and as a fallback elsewhere): the app config directory.
    pub fallback_dir: PathBuf,
}

const SERVICE: &str = "com.pomodairo.app";

impl SecretStore for OsSecretStore {
    fn get(&self, name: &str) -> Option<Vec<u8>> {
        #[cfg(any(windows, target_os = "macos"))]
        if let Ok(secret) = keyring::Entry::new(SERVICE, name).and_then(|e| e.get_secret()) {
            return Some(secret);
        }
        std::fs::read(self.fallback_dir.join(format!("{name}.key"))).ok()
    }

    fn set(&self, name: &str, secret: &[u8]) -> Result<()> {
        #[cfg(any(windows, target_os = "macos"))]
        {
            let stored = keyring::Entry::new(SERVICE, name).and_then(|e| e.set_secret(secret));
            match stored {
                Ok(()) => return Ok(()),
                Err(e) => eprintln!("sync: keychain unavailable ({e}), using a private file"),
            }
        }
        write_private_file(&self.fallback_dir.join(format!("{name}.key")), secret)
    }
}

/// Write a file only the current user can read.
fn write_private_file(path: &std::path::Path, data: &[u8]) -> Result<()> {
    let to_err = |e: std::io::Error| Error::Invalid(format!("cannot store device key: {e}"));
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(to_err)?;
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(to_err)?;
        f.write_all(data).map_err(to_err)
    }
    #[cfg(not(unix))]
    {
        // The app config directory is already private to the user on Windows.
        std::fs::write(path, data).map_err(to_err)
    }
}

/// Load this device's identity, creating it on first use. If the stored key is
/// missing (e.g. the database was copied from another computer), a new identity
/// is created and devices need to pair again.
pub fn load_or_create(conn: &Connection, secrets: &dyn SecretStore) -> Result<Identity> {
    let device_id = crate::sync::engine::device_id(conn)?;
    let key_name = format!("device-key-{device_id}");
    let cert: Option<String> = conn.query_row("SELECT cert FROM sync_state", [], |r| r.get(0))?;
    if let (Some(cert), Some(key)) = (cert.as_deref().and_then(unhex), secrets.get(&key_name)) {
        let cert = CertificateDer::from(cert);
        return Ok(Identity {
            device_id,
            fingerprint: fingerprint(&cert),
            cert,
            key,
        });
    }
    let identity = Identity::generate(&device_id)?;
    secrets.set(&key_name, &identity.key)?;
    conn.execute("UPDATE sync_state SET cert = ?1", [hex(&identity.cert)])?;
    Ok(identity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct MemSecrets(Mutex<HashMap<String, Vec<u8>>>);

    impl SecretStore for MemSecrets {
        fn get(&self, name: &str) -> Option<Vec<u8>> {
            self.0.lock().unwrap().get(name).cloned()
        }
        fn set(&self, name: &str, secret: &[u8]) -> Result<()> {
            self.0.lock().unwrap().insert(name.into(), secret.to_vec());
            Ok(())
        }
    }

    #[test]
    fn identity_is_created_once_and_reloaded() {
        let conn = crate::db::open_in_memory().unwrap();
        let secrets = MemSecrets::default();
        let first = load_or_create(&conn, &secrets).unwrap();
        let again = load_or_create(&conn, &secrets).unwrap();
        assert_eq!(first.fingerprint, again.fingerprint);
        assert_eq!(first.fingerprint.len(), 64);
        let stored: String = conn
            .query_row("SELECT cert FROM sync_state", [], |r| r.get(0))
            .unwrap();
        assert!(!stored.is_empty(), "certificate is stored, the key is not");
        let in_db: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sync_state WHERE cert LIKE '%' || ?1 || '%'",
                [hex(&first.key)],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(in_db, 0, "private key never goes into the database");
    }

    #[test]
    fn a_lost_key_means_a_new_identity() {
        let conn = crate::db::open_in_memory().unwrap();
        let first = load_or_create(&conn, &MemSecrets::default()).unwrap();
        let second = load_or_create(&conn, &MemSecrets::default()).unwrap();
        assert_ne!(first.fingerprint, second.fingerprint);
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(unhex(&hex(&[0, 1, 254, 255])), Some(vec![0, 1, 254, 255]));
        assert_eq!(unhex("abc"), None);
        assert_eq!(unhex("zz"), None);
    }
}
