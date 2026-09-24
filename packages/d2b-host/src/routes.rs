//! Host-prepare routes module.
//!
//! Retained surface: the `/etc/hosts` managed-block render/extract pair
//! consumed by the broker's hosts op (`d2b-broker::ops::hosts`). The
//! route-preflight predicate set and host-LAN-CIDR derivation were
//! retired with ADR 0015; the broker does its own `ip`-based preflight.

use d2b_core::host_w3::HostsEntry;

/// Marker lines delimiting the d2b-managed block in `/etc/hosts`.
pub const HOSTS_MANAGED_BEGIN: &str = "# d2b-managed begin";
pub const HOSTS_MANAGED_END: &str = "# d2b-managed end";

/// Renders the managed block exactly as the broker would write it.
/// One entry per line, address + hostname + aliases space-joined.
pub fn render_hosts_block(entries: &[HostsEntry]) -> String {
    let mut out = String::new();
    out.push_str(HOSTS_MANAGED_BEGIN);
    out.push('\n');
    for entry in entries {
        out.push_str(&entry.address);
        out.push(' ');
        out.push_str(&entry.hostname);
        for alias in &entry.aliases {
            out.push(' ');
            out.push_str(alias);
        }
        out.push('\n');
    }
    out.push_str(HOSTS_MANAGED_END);
    out.push('\n');
    out
}

/// Extracts the managed block from a `/etc/hosts` body. Foreign lines
/// outside the markers are not inspected.
pub fn extract_managed_block(hosts_body: &str) -> Option<String> {
    let begin = hosts_body.find(HOSTS_MANAGED_BEGIN)?;
    let end = hosts_body[begin..].find(HOSTS_MANAGED_END)?;
    let end_full = begin + end + HOSTS_MANAGED_END.len();
    let tail = &hosts_body[end_full..];
    let after_marker_nl = tail.find('\n').map(|i| i + 1).unwrap_or(tail.len());
    Some(hosts_body[begin..end_full + after_marker_nl].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_block_render_round_trip() {
        let entries = vec![
            HostsEntry {
                address: "10.0.0.10".into(),
                hostname: "vm-a".into(),
                aliases: vec!["a".into()],
            },
            HostsEntry {
                address: "10.0.0.11".into(),
                hostname: "vm-b".into(),
                aliases: vec![],
            },
        ];
        let rendered = render_hosts_block(&entries);
        let body = format!("127.0.0.1 localhost\n{rendered}# foreign\n");
        let extracted = extract_managed_block(&body).unwrap();
        assert_eq!(extracted, rendered);
    }
}