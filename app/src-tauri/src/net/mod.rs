//! Device-to-device sync over the local network: identities, TLS, the wire
//! protocol and the node that pairs and syncs with other devices.

pub mod discovery;
pub mod identity;
pub mod node;
pub mod protocol;
pub mod tls;

#[cfg(test)]
mod tests;
