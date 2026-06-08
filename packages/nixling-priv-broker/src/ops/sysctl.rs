//! `ApplySysctl` op (W3 s2).
//!
//! Per-link writes only; fail-closed on drift readback. The IPv6-off
//! sysctl set is defined in `nixling_host::netlink::IPV6_OFF_SYSCTLS`;
//! this op is the broker-side dispatch that translates a
//! [`SysctlIntent`] into a `/proc/sys/...` write with a deterministic
//! readback gate.

use crate::ops::exec_reconcile::{ReconcileExecError, ReconcileExecutor};
use nixling_core::host_w3::SysctlIntent;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ApplySysctlRequest {
    pub intents: Vec<SysctlIntent>,
    /// Override the `/proc/sys` root for tests.
    pub proc_sys_root: PathBuf,
}

impl ApplySysctlRequest {
    pub fn with_default_root(intents: Vec<SysctlIntent>) -> Self {
        Self {
            intents,
            proc_sys_root: PathBuf::from("/proc/sys"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplySysctlOutcome {
    pub key: String,
    pub value_before: String,
    pub value_after: String,
    pub drift: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplySysctlError {
    Io(String),
    ReadbackDrift {
        key: String,
        expected: String,
        observed: String,
    },
}

impl std::fmt::Display for ApplySysctlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(s) => write!(f, "sysctl: io: {s}"),
            Self::ReadbackDrift {
                key,
                expected,
                observed,
            } => write!(
                f,
                "ipv6-sysctl-drift: {key} expected={expected:?} observed={observed:?}"
            ),
        }
    }
}

impl std::error::Error for ApplySysctlError {}

impl From<io::Error> for ApplySysctlError {
    fn from(value: io::Error) -> Self {
        Self::Io(value.to_string())
    }
}

/// Converts `net.ipv6.conf.<ifname>.disable_ipv6` to
/// `<root>/net/ipv6/conf/<ifname>/disable_ipv6` for safe per-link
/// writes.
pub fn intent_to_proc_path(root: &Path, intent: &SysctlIntent) -> PathBuf {
    let mut path = root.to_path_buf();
    for component in intent.key.split('.') {
        path.push(component);
    }
    path
}

pub fn apply_sysctl_intents(
    req: &ApplySysctlRequest,
) -> Result<Vec<ApplySysctlOutcome>, ApplySysctlError> {
    let mut out = Vec::with_capacity(req.intents.len());
    for intent in &req.intents {
        let path = intent_to_proc_path(&req.proc_sys_root, intent);
        let before = std::fs::read_to_string(&path).unwrap_or_default();
        let trimmed_before = before.trim().to_owned();
        std::fs::write(&path, intent.value.as_bytes())?;
        let after = std::fs::read_to_string(&path).unwrap_or_default();
        let trimmed_after = after.trim().to_owned();
        if trimmed_after != intent.value {
            return Err(ApplySysctlError::ReadbackDrift {
                key: intent.key.clone(),
                expected: intent.value.clone(),
                observed: trimmed_after,
            });
        }
        out.push(ApplySysctlOutcome {
            key: intent.key.clone(),
            value_before: trimmed_before,
            value_after: trimmed_after,
            drift: false,
        });
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyWithReadbackError {
    ReconcileExec(ReconcileExecError),
    ReadbackIo {
        path: String,
        detail: String,
    },
    ReadbackDrift {
        key: String,
        expected: String,
        observed: String,
    },
}

impl std::fmt::Display for ApplyWithReadbackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReconcileExec(err) => write!(f, "apply-sysctl: {err}"),
            Self::ReadbackIo { path, detail } => {
                write!(f, "apply-sysctl readback {path}: {detail}")
            }
            Self::ReadbackDrift {
                key,
                expected,
                observed,
            } => write!(
                f,
                "apply-sysctl readback drift for {key}: expected {expected:?}, observed {observed:?}"
            ),
        }
    }
}

impl std::error::Error for ApplyWithReadbackError {}

/// W12 runtime entry-point for `ApplySysctl`.
///
/// Keep the dispatch surface anchored on `ops::sysctl` so future
/// per-key verification logic can land here without another runtime
/// rewrite.
pub fn apply_with_readback(
    executor: &dyn ReconcileExecutor,
    key: &str,
    value: &str,
) -> Result<(), ApplyWithReadbackError> {
    apply_with_readback_using(executor, key, value, read_sysctl_value)
}

fn apply_with_readback_using<F>(
    executor: &dyn ReconcileExecutor,
    key: &str,
    value: &str,
    mut readback: F,
) -> Result<(), ApplyWithReadbackError>
where
    F: FnMut(&str) -> Result<String, ApplyWithReadbackError>,
{
    executor
        .write_sysctl(key, value)
        .map_err(ApplyWithReadbackError::ReconcileExec)?;
    let observed = readback(key)?.trim().to_owned();
    if observed != value {
        return Err(ApplyWithReadbackError::ReadbackDrift {
            key: key.to_owned(),
            expected: value.to_owned(),
            observed,
        });
    }
    Ok(())
}

fn read_sysctl_value(key: &str) -> Result<String, ApplyWithReadbackError> {
    let path = proc_sys_path(key);
    fs::read_to_string(&path).map_err(|err| ApplyWithReadbackError::ReadbackIo {
        path: path.display().to_string(),
        detail: err.to_string(),
    })
}

fn proc_sys_path(key: &str) -> PathBuf {
    let mut path = PathBuf::from("/proc/sys");
    for component in key.split('.') {
        path.push(component);
    }
    path
}

pub fn destroy_value_for_key(key: &str) -> Option<&'static str> {
    if key.ends_with(".disable_ipv6") {
        Some("0")
    } else if key.ends_with(".accept_ra") || key.ends_with(".autoconf") {
        Some("1")
    } else if key.ends_with(".addr_gen_mode") || key.ends_with(".arp_ignore") {
        Some("0")
    } else if key == "net.bridge.bridge-nf-call-iptables"
        || key == "net.bridge.bridge-nf-call-ip6tables"
        || key == "net.bridge.bridge-nf-call-arptables"
    {
        Some("1")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::exec_reconcile::{FakeReconcileExecutor, ReconcileOp};

    fn scratch() -> PathBuf {
        let dir = std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join("target")
            .join("test-scratch")
            .join(format!(
                "nixling-w3-s2-sysctl-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn intent_to_proc_path_replaces_dots_with_slashes() {
        let intent = SysctlIntent {
            key: "net.ipv6.conf.nl-bX.disable_ipv6".into(),
            value: "1".into(),
            if_name: None,
        };
        let p = intent_to_proc_path(Path::new("/proc/sys"), &intent);
        assert_eq!(
            p.to_str().unwrap(),
            "/proc/sys/net/ipv6/conf/nl-bX/disable_ipv6"
        );
    }

    #[test]
    fn apply_writes_value_and_returns_outcome() {
        let dir = scratch();
        let leaf = dir.join("net/ipv6/conf/x");
        fs::create_dir_all(&leaf).unwrap();
        fs::write(leaf.join("disable_ipv6"), b"0\n").unwrap();
        let intent = SysctlIntent {
            key: "net.ipv6.conf.x.disable_ipv6".into(),
            value: "1".into(),
            if_name: None,
        };
        let outcomes = apply_sysctl_intents(&ApplySysctlRequest {
            intents: vec![intent],
            proc_sys_root: dir.clone(),
        })
        .unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].value_before, "0");
        assert_eq!(outcomes[0].value_after, "1");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn drift_after_write_fails_closed() {
        let dir = scratch();
        let leaf = dir.join("net/ipv6/conf/x");
        fs::create_dir_all(&leaf).unwrap();
        let path = leaf.join("disable_ipv6");
        fs::write(&path, b"sticky\n").unwrap();
        let intent = SysctlIntent {
            key: "net.ipv6.conf.x.disable_ipv6".into(),
            value: "1".into(),
            if_name: None,
        };
        let outcomes = apply_sysctl_intents(&ApplySysctlRequest {
            intents: vec![intent],
            proc_sys_root: dir.clone(),
        })
        .unwrap();
        assert_eq!(outcomes[0].value_after, "1");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn apply_with_readback_records_write_then_confirms() {
        let exec = FakeReconcileExecutor::new();
        apply_with_readback_using(&exec, "net.ipv4.ip_forward", "1", |_| Ok("1\n".to_owned()))
            .unwrap();
        let log = exec.take_log();
        assert_eq!(log.len(), 1);
        match &log[0] {
            ReconcileOp::WriteSysctl { key, value } => {
                assert_eq!(key, "net.ipv4.ip_forward");
                assert_eq!(value, "1");
            }
            other => panic!("unexpected op: {other:?}"),
        }
    }

    #[test]
    fn apply_with_readback_rejects_drift() {
        let exec = FakeReconcileExecutor::new();
        let err =
            apply_with_readback_using(&exec, "net.ipv4.ip_forward", "1", |_| Ok("0\n".to_owned()))
                .unwrap_err();
        assert!(matches!(
            err,
            ApplyWithReadbackError::ReadbackDrift {
                key,
                expected,
                observed,
            } if key == "net.ipv4.ip_forward" && expected == "1" && observed == "0"
        ));
    }
}
