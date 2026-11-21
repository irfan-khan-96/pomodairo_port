//! Finding other Pomodairo devices on the local network with mDNS / DNS-SD.
//!
//! A device advertises `_pomodairo._tcp` under its random device id. Its name is
//! only included while it is in pairing mode (so the other screen can show who
//! is waiting to pair); otherwise the advertisement reveals nothing personal.
//! Discovery is only a hint: whoever answers still has to prove its identity
//! with the pinned certificate.

use std::net::{IpAddr, SocketAddr};
use std::thread;

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

use crate::error::{Error, Result};
use crate::sync::hlc::is_device_id;

pub const SERVICE_TYPE: &str = "_pomodairo._tcp.local.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    Found {
        device: String,
        addrs: Vec<SocketAddr>,
        /// Present only while the device is in pairing mode.
        pairing_name: Option<String>,
    },
    Lost {
        device: String,
    },
}

pub struct Discovery {
    daemon: ServiceDaemon,
    device: String,
    port: u16,
}

fn mdns_err(e: mdns_sd::Error) -> Error {
    Error::Invalid(format!("local network discovery failed: {e}"))
}

impl Discovery {
    /// Advertise this device on `port` and report other devices to `on_seen`.
    pub fn start(device: &str, port: u16, on_seen: impl Fn(Seen) + Send + 'static) -> Result<Self> {
        let daemon = ServiceDaemon::new().map_err(mdns_err)?;
        let discovery = Discovery {
            daemon,
            device: device.to_string(),
            port,
        };
        discovery.advertise(None)?;
        let events = discovery.daemon.browse(SERVICE_TYPE).map_err(mdns_err)?;
        let me = device.to_string();
        thread::Builder::new()
            .name("mdns-browse".into())
            .spawn(move || {
                while let Ok(event) = events.recv() {
                    match event {
                        ServiceEvent::ServiceResolved(info) => {
                            let device = instance(&info.fullname);
                            if !is_device_id(&device) || device == me {
                                continue;
                            }
                            let mut addrs: Vec<SocketAddr> = info
                                .get_addresses_v4()
                                .into_iter()
                                .map(|ip| SocketAddr::new(IpAddr::V4(ip), info.port))
                                .collect();
                            addrs.sort();
                            let pairing_name = info
                                .txt_properties
                                .get_property_val_str("pair")
                                .filter(|v| !v.is_empty())
                                .map(str::to_string);
                            on_seen(Seen::Found {
                                device,
                                addrs,
                                pairing_name,
                            });
                        }
                        ServiceEvent::ServiceRemoved(_, fullname) => {
                            let device = instance(&fullname);
                            if is_device_id(&device) && device != me {
                                on_seen(Seen::Lost { device });
                            }
                        }
                        _ => {}
                    }
                }
            })
            .map_err(|e| Error::Invalid(format!("cannot start discovery: {e}")))?;
        Ok(discovery)
    }

    /// (Re-)advertise; `pairing_name` is shown to others only while pairing.
    pub fn advertise(&self, pairing_name: Option<&str>) -> Result<()> {
        let props = [("v", "1"), ("pair", pairing_name.unwrap_or(""))];
        let info = ServiceInfo::new(
            SERVICE_TYPE,
            &self.device,
            &format!("{}.local.", self.device),
            "",
            self.port,
            &props[..],
        )
        .map_err(mdns_err)?
        .enable_addr_auto();
        self.daemon.register(info).map_err(mdns_err)
    }
}

/// The instance (device id) part of `<instance>._pomodairo._tcp.local.`.
fn instance(fullname: &str) -> String {
    fullname
        .strip_suffix(&format!(".{SERVICE_TYPE}"))
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_name_is_the_device_id() {
        let id = "0123456789abcdef0123456789abcdef";
        assert_eq!(instance(&format!("{id}._pomodairo._tcp.local.")), id);
        assert_eq!(instance("printer._ipp._tcp.local."), "");
    }
}
