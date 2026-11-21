//! Hybrid logical clock: wall-clock milliseconds plus a counter, tie-broken by
//! device id. Timestamps only move forward, even if a device's clock is behind
//! one it has received events from, so "newest edit wins" is well defined.
//!
//! Text form `MMMMMMMMMMMMMMM.CCCCC.<device>` (fixed width) sorts the same as
//! the timestamps themselves, so SQL `MAX(hlc)` and `ORDER BY hlc` just work.

use std::fmt;

const MAX_COUNTER: u32 = 99_999;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hlc {
    pub ms: u64,
    pub counter: u32,
    pub device: String,
}

impl Hlc {
    /// The next timestamp for `device`, after `last` (the newest one seen) and
    /// no earlier than the wall clock `now_ms`.
    pub fn tick(last: Option<&Hlc>, now_ms: u64, device: &str) -> Hlc {
        let (ms, counter) = match last {
            Some(l) if l.ms >= now_ms => {
                if l.counter >= MAX_COUNTER {
                    (l.ms + 1, 0)
                } else {
                    (l.ms, l.counter + 1)
                }
            }
            _ => (now_ms, 0),
        };
        Hlc {
            ms,
            counter,
            device: device.to_string(),
        }
    }

    pub fn parse(s: &str) -> Option<Hlc> {
        let mut parts = s.splitn(3, '.');
        let (ms, counter, device) = (parts.next()?, parts.next()?, parts.next()?);
        if ms.len() != 15 || counter.len() != 5 || !is_device_id(device) {
            return None;
        }
        Some(Hlc {
            ms: ms.parse().ok()?,
            counter: counter.parse().ok().filter(|c| *c <= MAX_COUNTER)?,
            device: device.to_string(),
        })
    }
}

impl fmt::Display for Hlc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:015}.{:05}.{}", self.ms, self.counter, self.device)
    }
}

/// Device ids and global ids are 32 lowercase hex characters.
pub fn is_device_id(s: &str) -> bool {
    s.len() == 32
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn follows_wall_clock_when_it_moves_forward() {
        let t = Hlc::tick(None, 1000, A);
        assert_eq!((t.ms, t.counter), (1000, 0));
        let t2 = Hlc::tick(Some(&t), 2000, A);
        assert_eq!((t2.ms, t2.counter), (2000, 0));
    }

    #[test]
    fn never_goes_backwards() {
        let seen = Hlc {
            ms: 5000,
            counter: 7,
            device: B.into(),
        };
        // Our clock is behind the newest timestamp we have seen.
        let t = Hlc::tick(Some(&seen), 1000, A);
        assert!(t > seen);
        assert_eq!((t.ms, t.counter), (5000, 8));
        let full = Hlc {
            ms: 5000,
            counter: MAX_COUNTER,
            device: B.into(),
        };
        assert_eq!(Hlc::tick(Some(&full), 1000, A).ms, 5001);
    }

    #[test]
    fn text_form_round_trips_and_sorts_like_the_value() {
        let a = Hlc {
            ms: 999,
            counter: 12,
            device: A.into(),
        };
        let b = Hlc {
            ms: 1000,
            counter: 0,
            device: A.into(),
        };
        let c = Hlc {
            ms: 1000,
            counter: 0,
            device: B.into(),
        };
        assert_eq!(Hlc::parse(&a.to_string()), Some(a.clone()));
        assert!(a < b && b < c);
        assert!(a.to_string() < b.to_string() && b.to_string() < c.to_string());
    }

    #[test]
    fn rejects_malformed_text() {
        for bad in [
            "",
            "1.2.3",
            "000000000001000.00000.XYZ",
            "00000000000100.00000.aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ] {
            assert_eq!(Hlc::parse(bad), None, "{bad}");
        }
    }
}
