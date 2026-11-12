//! Whether this network's DNS sends YouTube's hosts somewhere that isn't Google.
//!
//! Some DNS services and filters answer `www.youtube.com` and `youtubei.googleapis.com` with
//! servers of their own that pass the traffic on, while `music.youtube.com` resolves
//! normally. YouTube then sees those servers' country, refuses the songs not licensed there,
//! and ties stream URLs to their address. Pear's web player talks to `music.youtube.com`
//! only, so it doesn't notice.
//!
//! An answer outside Google's ranges isn't wrong by itself (a Google cache inside an ISP can
//! answer so), so what `check` finds goes to the log and Copy diagnostics, and the user hears
//! of it only when YouTube refuses songs too (the refused songs' toast). `check` asks the
//! system resolver once, off the UI thread. Nothing is overridden: requests still go where
//! the system's answers say.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::sync::OnceLock;

/// The hosts featherplay's player and API requests go to, which such a rule redirects.
const CHECKED: [&str; 2] = ["www.youtube.com", "youtubei.googleapis.com"];
/// The host the same rules leave alone, whose answer the others are compared with.
const REFERENCE: &str = "music.youtube.com";

/// Google's own front-end prefixes, from `https://www.gstatic.com/ipranges/goog.json`
/// (creationTime 2026-11-12). Only the blocks Google's sites answer from; Google Cloud's
/// customer ranges (34.x, 35.x and the like) are left out, as anyone can run a host there.
const GOOGLE_V4: &[(Ipv4Addr, u8)] = &[
    (Ipv4Addr::new(8, 8, 4, 0), 24),
    (Ipv4Addr::new(8, 8, 8, 0), 24),
    (Ipv4Addr::new(64, 233, 160, 0), 19),
    (Ipv4Addr::new(66, 102, 0, 0), 20),
    (Ipv4Addr::new(66, 249, 64, 0), 19),
    (Ipv4Addr::new(70, 32, 128, 0), 19),
    (Ipv4Addr::new(72, 14, 192, 0), 18),
    (Ipv4Addr::new(74, 125, 0, 0), 16),
    (Ipv4Addr::new(108, 170, 192, 0), 18),
    (Ipv4Addr::new(108, 177, 0, 0), 17),
    (Ipv4Addr::new(142, 250, 0, 0), 15),
    (Ipv4Addr::new(172, 217, 0, 0), 16),
    (Ipv4Addr::new(172, 253, 0, 0), 16),
    (Ipv4Addr::new(173, 194, 0, 0), 16),
    (Ipv4Addr::new(192, 178, 0, 0), 15),
    (Ipv4Addr::new(207, 223, 160, 0), 20),
    (Ipv4Addr::new(208, 65, 152, 0), 22),
    (Ipv4Addr::new(208, 117, 224, 0), 19),
    (Ipv4Addr::new(209, 85, 128, 0), 17),
    (Ipv4Addr::new(216, 58, 192, 0), 19),
    (Ipv4Addr::new(216, 239, 32, 0), 19),
];
const GOOGLE_V6: &[(Ipv6Addr, u8)] = &[
    (Ipv6Addr::new(0x2001, 0x4860, 0, 0, 0, 0, 0, 0), 32),
    (Ipv6Addr::new(0x2404, 0x6800, 0, 0, 0, 0, 0, 0), 32),
    (Ipv6Addr::new(0x2607, 0xf8b0, 0, 0, 0, 0, 0, 0), 32),
    (Ipv6Addr::new(0x2800, 0x03f0, 0, 0, 0, 0, 0, 0), 32),
    (Ipv6Addr::new(0x2a00, 0x1450, 0, 0, 0, 0, 0, 0), 32),
    (Ipv6Addr::new(0x2c0f, 0xfb50, 0, 0, 0, 0, 0, 0), 32),
];

/// Two addresses this close are taken as the same operator's: a /16 for IPv4, a /32 for
/// IPv6 (Google answers different names from neighbouring blocks).
const NEAR_V4: u8 = 16;
const NEAR_V6: u8 = 32;

/// What `check` found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dns {
    /// YouTube's hosts resolve as `music.youtube.com` does, or to Google.
    Normal,
    /// A proxy is set: it looks the names up, not this PC.
    Proxied,
    /// No lookup answered (offline, or no DNS).
    Failed,
    /// These hosts resolve to addresses that aren't Google's, each with its answer, and
    /// `music.youtube.com` to `reference` (its first address, if it resolved).
    Redirected {
        hosts: Vec<(&'static str, Vec<IpAddr>)>,
        reference: Option<IpAddr>,
    },
}

impl Dns {
    pub fn redirected(&self) -> bool {
        matches!(self, Dns::Redirected { .. })
    }
}

/// `normal`, `not checked (a proxy resolves names)`, `lookups failed`, or
/// `www.youtube.com resolves to 203.0.113.7, outside Google's own ranges and unlike
/// music.youtube.com (142.250.180.14)`.
impl fmt::Display for Dns {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Dns::Normal => f.write_str("normal"),
            Dns::Proxied => f.write_str("not checked (a proxy resolves names)"),
            Dns::Failed => f.write_str("lookups failed"),
            Dns::Redirected { hosts, reference } => {
                for (n, (host, addresses)) in hosts.iter().enumerate() {
                    let list = addresses
                        .iter()
                        .map(IpAddr::to_string)
                        .collect::<Vec<_>>()
                        .join(", ");
                    match n {
                        0 => write!(f, "{host} resolves to {list}")?,
                        _ => write!(f, " and {host} to {list}")?,
                    }
                }
                f.write_str(", outside Google's own ranges")?;
                match reference {
                    Some(r) => write!(f, " and unlike {REFERENCE} ({r})"),
                    None => Ok(()),
                }
            }
        }
    }
}

/// The decision. `proxied`: a proxy is set for YouTube. `reference`: the answer for
/// `music.youtube.com`; `hosts`: the answers for the hosts checked (empty when a lookup
/// failed). A host is redirected when it has an answer, none of its addresses is in Google's
/// prefixes, and none is near one of the reference's.
pub fn judge(proxied: bool, reference: &[IpAddr], hosts: &[(&'static str, Vec<IpAddr>)]) -> Dns {
    if proxied {
        return Dns::Proxied;
    }
    if reference.is_empty() && hosts.iter().all(|(_, a)| a.is_empty()) {
        return Dns::Failed;
    }
    let redirected: Vec<_> = hosts
        .iter()
        .filter(|(_, addresses)| {
            !addresses.is_empty()
                && addresses
                    .iter()
                    .all(|a| !is_google(a) && !reference.iter().any(|r| near(a, r)))
        })
        .cloned()
        .collect();
    if redirected.is_empty() {
        Dns::Normal
    } else {
        Dns::Redirected {
            hosts: redirected,
            reference: reference.first().copied(),
        }
    }
}

fn is_google(address: &IpAddr) -> bool {
    match address {
        IpAddr::V4(a) => GOOGLE_V4.iter().any(|(net, len)| within_v4(*a, *net, *len)),
        IpAddr::V6(a) => GOOGLE_V6.iter().any(|(net, len)| within_v6(*a, *net, *len)),
    }
}

fn near(a: &IpAddr, b: &IpAddr) -> bool {
    match (a, b) {
        (IpAddr::V4(a), IpAddr::V4(b)) => within_v4(*a, *b, NEAR_V4),
        (IpAddr::V6(a), IpAddr::V6(b)) => within_v6(*a, *b, NEAR_V6),
        _ => false,
    }
}

fn within_v4(a: Ipv4Addr, net: Ipv4Addr, len: u8) -> bool {
    let mask = u32::MAX.checked_shl(32 - u32::from(len)).unwrap_or(0);
    u32::from(a) & mask == u32::from(net) & mask
}

fn within_v6(a: Ipv6Addr, net: Ipv6Addr, len: u8) -> bool {
    let mask = u128::MAX.checked_shl(128 - u32::from(len)).unwrap_or(0);
    u128::from(a) & mask == u128::from(net) & mask
}

static FOUND: OnceLock<Dns> = OnceLock::new();

/// Looks the hosts up with the system resolver and keeps the verdict for the session; a
/// second call returns the first one's. Blocks for the lookups: run it off the UI thread.
/// Skipped when a proxy is set for YouTube, since the proxy resolves the names.
pub fn check() -> &'static Dns {
    FOUND.get_or_init(|| {
        let proxied = crate::for_url("https://www.youtube.com/").is_some();
        let dns = if proxied {
            Dns::Proxied
        } else {
            let hosts: Vec<_> = CHECKED.iter().map(|h| (*h, resolve(h))).collect();
            judge(false, &resolve(REFERENCE), &hosts)
        };
        log::info!("dns: {dns}");
        dns
    })
}

/// What `check` found, once it has run.
pub fn found() -> Option<&'static Dns> {
    FOUND.get()
}

/// The system resolver's addresses for `host`, without repeats; none when it fails.
fn resolve(host: &str) -> Vec<IpAddr> {
    let mut out: Vec<IpAddr> = Vec::new();
    match (host, 443).to_socket_addrs() {
        Ok(found) => {
            for a in found.map(|s| s.ip()) {
                if !out.contains(&a) {
                    out.push(a);
                }
            }
        }
        Err(e) => log::info!("dns: looking up {host} failed: {e}"),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ips(list: &[&str]) -> Vec<IpAddr> {
        list.iter().map(|s| s.parse().unwrap()).collect()
    }

    /// What a normal network answers for `music.youtube.com`.
    fn music() -> Vec<IpAddr> {
        ips(&["142.250.180.14", "2a00:1450:4002:411::200e"])
    }

    fn hosts(youtube: &[&str], youtubei: &[&str]) -> Vec<(&'static str, Vec<IpAddr>)> {
        vec![
            ("www.youtube.com", ips(youtube)),
            ("youtubei.googleapis.com", ips(youtubei)),
        ]
    }

    #[test]
    fn hosts_sent_elsewhere_are_redirected() {
        let dns = judge(
            false,
            &music(),
            &hosts(&["203.0.113.7", "198.51.100.20"], &["203.0.113.7"]),
        );
        assert!(dns.redirected());
        assert_eq!(
            dns.to_string(),
            "www.youtube.com resolves to 203.0.113.7, 198.51.100.20 and \
             youtubei.googleapis.com to 203.0.113.7, outside Google's own ranges and unlike \
             music.youtube.com (142.250.180.14)"
        );
        let one = judge(
            false,
            &music(),
            &hosts(&["203.0.113.7"], &["142.250.184.10"]),
        );
        assert_eq!(
            one.to_string(),
            "www.youtube.com resolves to 203.0.113.7, outside Google's own ranges and unlike \
             music.youtube.com (142.250.180.14)"
        );
        // music.youtube.com didn't resolve: only Google's ranges to go by.
        let alone = judge(false, &[], &hosts(&["203.0.113.7"], &[]));
        assert_eq!(
            alone.to_string(),
            "www.youtube.com resolves to 203.0.113.7, outside Google's own ranges"
        );
    }

    #[test]
    fn google_answers_are_normal() {
        // The same addresses as music.youtube.com.
        let same = judge(
            false,
            &music(),
            &hosts(&["142.250.180.14"], &["142.250.180.14"]),
        );
        assert_eq!(same, Dns::Normal);
        // Other Google blocks, as answers differ per name on some networks.
        let other = judge(
            false,
            &music(),
            &hosts(&["216.58.204.142", "172.217.16.14"], &["74.125.71.95"]),
        );
        assert_eq!(other, Dns::Normal);
    }

    #[test]
    fn an_address_near_music_youtube_coms_is_normal() {
        // Not in the embedded list (a Google cache in an ISP, say), but in the same /16.
        let reference = ips(&["203.0.113.40"]);
        let dns = judge(
            false,
            &reference,
            &hosts(&["203.0.200.1"], &["203.0.113.41"]),
        );
        assert_eq!(dns, Dns::Normal);
    }

    #[test]
    fn ipv6_answers_are_judged_too() {
        let google = judge(
            false,
            &music(),
            &hosts(&["2a00:1450:4002:402::200e"], &["2607:f8b0:4004:c1b::5f"]),
        );
        assert_eq!(google, Dns::Normal);
        let elsewhere = judge(false, &music(), &hosts(&["2001:db8::7"], &["2001:db8::7"]));
        assert!(elsewhere.redirected(), "{elsewhere:?}");
        // An IPv6 answer near music.youtube.com's (same /32).
        let reference = ips(&["2001:db8:1::1"]);
        let near = judge(false, &reference, &hosts(&["2001:db8:2::1"], &[]));
        assert_eq!(near, Dns::Normal);
    }

    #[test]
    fn a_proxy_or_failed_lookups_decide_nothing() {
        let redirected = hosts(&["203.0.113.7"], &["203.0.113.7"]);
        assert_eq!(judge(true, &music(), &redirected), Dns::Proxied);
        assert_eq!(judge(false, &[], &hosts(&[], &[])), Dns::Failed);
        // A host that didn't resolve isn't redirected.
        assert_eq!(
            judge(false, &music(), &hosts(&[], &["142.250.184.10"])),
            Dns::Normal
        );
    }
}
