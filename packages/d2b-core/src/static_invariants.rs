//! Static security/policy invariants over rendered manifest + bundle artifacts.
//!
//! These were historically enforced by the `tests/static-invariant-*.sh` bash
//! gates as `jq` filters over synthetic positive/negative fixtures (plus, for
//! two of them, a grep over the real rendered `vms.json`). They are re-homed
//! here as typed validators, with the original positive/negative gate cases
//! preserved as in-module unit tests (see the `tests` module below).
//!
//! Validators are pure and return the list of offending locations (empty ==
//! invariant holds), so callers (contract tests, and potentially the broker)
//! can assert and report precisely.

use std::borrow::Cow;

use serde_json::Value;

/// Linux capabilities considered "broad" - granting one to a long-lived
/// profile requires an explicit ADR/plan carve-out reference.
pub const BROAD_CAPABILITIES: &[&str] = &["CAP_SYS_ADMIN", "CAP_NET_ADMIN"];

/// Field-name keys that are public-safe in the world-readable manifest
/// (`vms.json`). Mirrors the allowlist from
/// `tests/static-invariant-world-readable-leak.sh`.
pub const PUBLIC_MANIFEST_FIELDS: &[&str] = &[
    "_manifest",
    "_observability",
    "name",
    "env",
    "index",
    "hostName",
    "hostname",
    "sshHost",
    "sshPort",
    "sshUser",
    "ipv4",
    "ip",
    "mac",
    "tap",
    "bridge",
    "netVm",
    "isNetVm",
    "routerVm",
    "isRouter",
    "autostart",
    "graphics",
    "tpm",
    "audio",
    "usbip",
    "usbipYubikey",
    "usbipdHostIp",
    "securityKey",
    "observability",
    // `observability.enabled` (public-safe boolean) is nested under the per-VM
    // `observability` object in the current manifest; the bash allowlist
    // predated this field. The path-bearing key/secret invariant separately
    // guards the observability block against host-path leaks.
    "enabled",
    "enable",
    "vsockCid",
    "vsockHostSocket",
    "agentSocket",
    "state",
    "status",
    "pendingRestart",
    "closure",
    "current",
    "booted",
    "runner",
    "store",
    "stateDir",
    "apiSocket",
    "gpuSocket",
    "tpmSocket",
    "timeoutSeconds",
    "audioStateFile",
    "audioService",
    "staticIp",
    "runtime",
    "shell",
    "defaultName",
    "maxAttached",
    "maxSessions",
    "autostartPolicy",
    "kind",
    "provider",
    "driver",
    "id",
    "type",
    "capabilities",
    "operationCapabilities",
    "services",
    "role",
    "optional",
    "configSync",
    "display",
    "displayWindow",
    "exec",
    "guest",
    "gracefulShutdown",
    "hostPrepare",
    "inGuestObservability",
    "keys",
    "lifecycle",
    "media",
    "qemuMedia",
    "removableMedia",
    "restart",
    "ssh",
    "start",
    "stop",
    "storage",
    "storeSync",
    "switch",
    "usbHotplug",
    "video",
    "virtiofs",
    "volumes",
    "waylandProxy",
];

/// Collect `(dotted_path, scalar_value)` for every scalar leaf in `value`,
/// mirroring jq's `paths(scalars)`. Array indices appear as numeric path
/// segments (stringified), matching the bash gates.
fn scalar_paths(value: &Value) -> Vec<(Vec<String>, &Value)> {
    let mut out = Vec::new();
    walk_scalars(value, &mut Vec::new(), &mut out);
    out
}

fn walk_scalars<'a>(
    value: &'a Value,
    prefix: &mut Vec<String>,
    out: &mut Vec<(Vec<String>, &'a Value)>,
) {
    match value {
        Value::Object(map) => {
            for (k, v) in map {
                prefix.push(k.clone());
                walk_scalars(v, prefix, out);
                prefix.pop();
            }
        }
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                prefix.push(i.to_string());
                walk_scalars(v, prefix, out);
                prefix.pop();
            }
        }
        // Scalar leaf (incl. null).
        scalar => out.push((prefix.clone(), scalar)),
    }
}

/// **world-readable-leak**: the public `vms.json` must expose only
/// allowlisted, public-safe scalar fields. Returns the dotted paths of any
/// scalar whose terminal key is not in [`PUBLIC_MANIFEST_FIELDS`] and is not
/// nested under the `_manifest.`/`_observability.` reserved blocks.
pub fn world_readable_field_leaks(manifest: &Value) -> Vec<String> {
    let mut leaks = Vec::new();
    for (path, _) in scalar_paths(manifest) {
        let Some(last) = path.last() else { continue };
        if PUBLIC_MANIFEST_FIELDS.contains(&last.as_str()) {
            continue;
        }
        let dotted = path.join(".");
        if dotted.starts_with("_manifest.") || dotted.starts_with("_observability.") {
            continue;
        }
        leaks.push(dotted);
    }
    leaks
}

/// Terminal-key suffixes that denote a key/secret *path* (a host filesystem
/// location), which must never appear in world-readable artifacts.
const PATH_BEARING_KEY_SUFFIXES: &[&str] = &[
    "keypath",
    "privatekeypath",
    "secret_path",
    "secretpath",
    "tokenpath",
    "credentialpath",
];

/// **opaque-key-ids**: manifest/bundle artifacts must reference keys and
/// secrets by opaque IDs, never by host path. Returns `path=value` for every
/// scalar whose terminal key matches a path-bearing key suffix
/// (case-insensitive) and whose value looks like a path (contains `/`).
pub fn path_bearing_key_violations(manifest: &Value) -> Vec<String> {
    let mut violations = Vec::new();
    for (path, value) in scalar_paths(manifest) {
        let Some(last) = path.last() else { continue };
        let lower = last.to_ascii_lowercase();
        if !PATH_BEARING_KEY_SUFFIXES
            .iter()
            .any(|suffix| lower.ends_with(suffix))
        {
            continue;
        }
        let rendered = match value {
            Value::String(s) => Cow::Borrowed(s.as_str()),
            other => Cow::Owned(other.to_string()),
        };
        if rendered.contains('/') {
            violations.push(format!("{}={}", path.join("."), rendered));
        }
    }
    violations
}

/// **broad-caps**: a process retaining a broad capability
/// ([`BROAD_CAPABILITIES`]) must carry a non-empty `adr_carve_out` reference.
/// Returns `true` when `caps` requests a broad capability without an ADR
/// carve-out. Field-based (not tied to a specific profile DTO) so it applies
/// equally to `processes.json` `RoleProfile`s and `SandboxProfile`s.
pub fn is_broad_cap_violation(caps: &[String], adr_carve_out: Option<&str>) -> bool {
    let requests_broad = caps
        .iter()
        .any(|cap| BROAD_CAPABILITIES.contains(&cap.as_str()));
    if !requests_broad {
        return false;
    }
    let has_carve_out = adr_carve_out.is_some_and(|s| !s.trim().is_empty());
    !has_carve_out
}

/// **writable-paths**: every writable path a process declares must be declared
/// by the bundle. Returns the used paths absent from `declared` (the set
/// difference `used - declared`).
pub fn undeclared_writable_paths<'a>(
    declared: impl IntoIterator<Item = &'a str>,
    used: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let declared: std::collections::BTreeSet<&str> = declared.into_iter().collect();
    let mut undeclared: Vec<String> = used
        .into_iter()
        .filter(|p| !declared.contains(p))
        .map(|p| p.to_owned())
        .collect();
    undeclared.sort();
    undeclared.dedup();
    undeclared
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Negative fixture from `tests/static-invariant-world-readable-leak.sh`:
    /// a non-allowlisted field must be reported by dotted path, so the owning
    /// object segment is part of the reported location.
    #[test]
    fn world_readable_leak_rejects_non_allowlisted_field() {
        let manifest = json!({
            "corp-vm": {"name": "corp-vm", "privateKeyPath": "/var/lib/nixling/vms/corp-vm/id_ed25519"}
        });
        assert_eq!(
            world_readable_field_leaks(&manifest),
            vec!["corp-vm.privateKeyPath".to_owned()]
        );
    }

    /// Negative fixture from `tests/static-invariant-opaque-key-ids.sh`:
    /// path-bearing key suffixes with host-path values must be reported as
    /// `dotted.path=value`.
    #[test]
    fn path_bearing_key_rejects_host_paths() {
        let manifest = json!({
            "keys": {"ssh": {"privateKeyPath": "/var/lib/nixling/vms/corp-vm/id_ed25519"}},
            "secret_path": "/run/secrets/token"
        });
        assert_eq!(
            path_bearing_key_violations(&manifest),
            vec![
                "keys.ssh.privateKeyPath=/var/lib/nixling/vms/corp-vm/id_ed25519".to_owned(),
                "secret_path=/run/secrets/token".to_owned(),
            ]
        );
    }

    #[test]
    fn world_readable_field_leaks_reports_only_unallowlisted_scalars_outside_reserved_blocks() {
        let manifest = serde_json::json!({
            "name": "dev",
            "secretField": "s3cr3t",
            "_manifest": { "hostName": "hidden" },
            "_observability": { "metricsToken": "opaque" },
        });
        let mut leaked = world_readable_field_leaks(&manifest);
        leaked.sort();
        assert_eq!(leaked, vec!["secretField".to_owned()]);
    }

    #[test]
    fn world_readable_field_leaks_accepts_allowlisted_and_reserved_scalars() {
        let manifest = serde_json::json!({
            "name": "dev",
            "state": "Ready",
            "netVm": { "ip": "10.0.0.1", "mac": "aa:bb" },
            "_manifest": { "generation": 3 },
            "_observability": { "enabled": true },
        });
        assert!(world_readable_field_leaks(&manifest).is_empty());
    }

    #[test]
    fn path_bearing_key_violations_names_only_path_shaped_values() {
        let manifest = serde_json::json!({
            "keyPath": "/run/keys/ed25519",
            "tokenPath": "opaque-id-42",
            "privateKeyPath": 7,
            "credentialPath": "/etc/d2b/creds.json",
            "display": { "tokenPath": "/var/lib/d2b/token" },
        });
        let mut violations = path_bearing_key_violations(&manifest);
        violations.sort_unstable();
        assert_eq!(
            violations,
            vec![
                "credentialPath=/etc/d2b/creds.json".to_owned(),
                "display.tokenPath=/var/lib/d2b/token".to_owned(),
                "keyPath=/run/keys/ed25519".to_owned(),
            ]
        );
    }

    #[test]
    fn broad_cap_requires_a_nonempty_adr_carve_out() {
        assert!(is_broad_cap_violation(
            &["CAP_SYS_ADMIN".to_owned()],
            None,
        ));
        assert!(is_broad_cap_violation(
            &["CAP_NET_ADMIN".to_owned()],
            Some("   "),
        ));
        assert!(!is_broad_cap_violation(
            &["CAP_SYS_ADMIN".to_owned()],
            Some("adr/2026-09-24/broad-caps"),
        ));
        assert!(!is_broad_cap_violation(&["CAP_CHOWN".to_owned()], None));
    }

    #[test]
    fn undeclared_writable_paths_reports_missing_used_paths_sorted_and_deduped() {
        let declared = ["/data", "/runtime"];
        let used = ["/runtime", "/data", "/data", "/etc/passwd", "/tmp/x"];
        assert_eq!(
            undeclared_writable_paths(declared.iter().copied(), used.iter().copied()),
            vec!["/etc/passwd".to_owned(), "/tmp/x".to_owned()]
        );
    }
}
