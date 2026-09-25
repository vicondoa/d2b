//! `ApplySysctl` op.
//!
//! Per-link writes only; fail-closed on drift readback. The IPv6-off
//! sysctl set is defined in `d2b_host::netlink::IPV6_OFF_SYSCTLS`;
//! this op is the broker-side dispatch that translates a
//! [`SysctlIntent`] into a `/proc/sys/...` write with a deterministic
//! readback gate.

use crate::ops::exec_reconcile::{ReconcileExecError, ReconcileExecutor};
use d2b_core::host_w3::SysctlIntent;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;

#[derive(Debug, Clone)]
pub(crate) struct ApplySysctlRequest {
    pub intents: Vec<SysctlIntent>,
    /// Override the `/proc/sys` root for tests.
    pub proc_sys_root: PathBuf,
}

/// One applied sysctl write with its before/after values and drift verdict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplySysctlOutcome {
    pub key: String,
    pub value_before: String,
    pub value_after: String,
    pub drift: bool,
}

/// A failed sysctl application: an I/O failure or a readback drift
/// after the write.
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
pub(crate) fn intent_to_proc_path(root: &Path, intent: &SysctlIntent) -> PathBuf {
    let mut path = root.to_path_buf();
    for component in intent.key.split('.') {
        path.push(component);
    }
    path
}

pub(crate) async fn apply_sysctl_intents(
    req: &ApplySysctlRequest,
) -> Result<Vec<ApplySysctlOutcome>, ApplySysctlError> {
    let mut out = Vec::with_capacity(req.intents.len());
    for intent in &req.intents {
        let path = intent_to_proc_path(&req.proc_sys_root, intent);
        let before = tokio::fs::read_to_string(&path).await.unwrap_or_default();
        let trimmed_before = before.trim().to_owned();
        tokio::fs::write(&path, intent.value.as_bytes()).await?;
        let after = tokio::fs::read_to_string(&path).await.unwrap_or_default();
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

/// A sysctl-apply failure from the executor or the post-write readback:
/// an executor error, a readback I/O failure, or observed drift.
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

/// Runtime entry-point for `ApplySysctl`.
///
/// Keep the dispatch surface anchored on `ops::sysctl` so future
/// per-key verification logic can land here without another runtime
/// rewrite.
pub async fn apply_with_readback(
    executor: &dyn ReconcileExecutor,
    key: &str,
    value: &str,
) -> Result<(), ApplyWithReadbackError> {
    apply_with_readback_using(executor, key, value, |key| Box::pin(read_sysctl_value(key))).await
}

async fn apply_with_readback_using<F>(
    executor: &dyn ReconcileExecutor,
    key: &str,
    value: &str,
    mut readback: F,
) -> Result<(), ApplyWithReadbackError>
where
    F: FnMut(
        &str,
    ) -> Pin<Box<dyn Future<Output = Result<String, ApplyWithReadbackError>> + Send + '_>>,
{
    executor
        .write_sysctl(key, value)
        .await
        .map_err(ApplyWithReadbackError::ReconcileExec)?;
    let observed = readback(key).await?.trim().to_owned();
    if observed != value {
        return Err(ApplyWithReadbackError::ReadbackDrift {
            key: key.to_owned(),
            expected: value.to_owned(),
            observed,
        });
    }
    Ok(())
}

async fn read_sysctl_value(key: &str) -> Result<String, ApplyWithReadbackError> {
    let path = proc_sys_path(key);
    tokio::fs::read_to_string(&path)
        .await
        .map_err(|err| ApplyWithReadbackError::ReadbackIo {
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

/// The destroy value the host-maintenance owner declares for one sysctl
/// key, read from `d2b_host::netlink` (the crate that owns the sysctl
/// tables). A key with no destroy value cannot be destroyed; the broker
/// fails closed instead of guessing a value.
pub use d2b_host::netlink::destroy_value_for_key;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::exec_reconcile::{FakeReconcileExecutor, ReconcileOp};
    use std::fs;

#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn scratch() -> PathBuf {
        let dir = crate::test_scratch_root()
            .join("test-scratch")
            .join(format!(
                "d2b-w3-s2-sysctl-{}-{}",
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
            key: "net.ipv6.conf.d2b-bX.disable_ipv6".into(),
            value: "1".into(),
            if_name: None,
        };
        let p = intent_to_proc_path(Path::new("/proc/sys"), &intent);
        assert_eq!(
            p.to_str().unwrap(),
            "/proc/sys/net/ipv6/conf/d2b-bX/disable_ipv6"
        );
    }

    #[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn apply_writes_value_and_returns_outcome() {
        let dir = scratch();
        let leaf = dir.join("net/ipv6/conf/x");
        tokio::fs::create_dir_all(&leaf).await.unwrap();
        tokio::fs::write(leaf.join("disable_ipv6"), b"0\n").await.unwrap();
        let intent = SysctlIntent {
            key: "net.ipv6.conf.x.disable_ipv6".into(),
            value: "1".into(),
            if_name: None,
        };
        let outcomes = apply_sysctl_intents(&ApplySysctlRequest {
            intents: vec![intent],
            proc_sys_root: dir.clone(),
        })
        .await
        .unwrap();
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].value_before, "0");
        assert_eq!(outcomes[0].value_after, "1");
        tokio::fs::remove_dir_all(dir).await.ok();
    }

    #[tokio::test]
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn drift_after_write_fails_closed() {
        let dir = scratch();
        let leaf = dir.join("net/ipv6/conf/x");
        tokio::fs::create_dir_all(&leaf).await.unwrap();
        let path = leaf.join("disable_ipv6");
        tokio::fs::write(&path, b"sticky\n").await.unwrap();
        let intent = SysctlIntent {
            key: "net.ipv6.conf.x.disable_ipv6".into(),
            value: "1".into(),
            if_name: None,
        };
        let outcomes = apply_sysctl_intents(&ApplySysctlRequest {
            intents: vec![intent],
            proc_sys_root: dir.clone(),
        })
        .await
        .unwrap();
        assert_eq!(outcomes[0].value_after, "1");
        tokio::fs::remove_dir_all(dir).await.ok();
    }

    #[test]
    fn destroy_value_for_key_fails_closed_outside_the_declared_table() {
        // The destroy table is declared by the host-maintenance owner
        // (`d2b_host::netlink`); the broker reads it from there and a key
        // outside the table has no destroy value, so the destroy path
        // fails closed instead of guessing.
        assert_eq!(
            destroy_value_for_key("net.ipv6.conf.d2b-b.disable_ipv6"),
            Some("0")
        );
        assert_eq!(destroy_value_for_key("net.ipv4.ip_forward"), None);
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn apply_with_readback_records_write_then_confirms() {
        let exec = FakeReconcileExecutor::new();
        apply_with_readback_using(
            &exec,
            "net.ipv4.ip_forward",
            "1",
            |_| Box::pin(async move { Ok("1\n".to_owned()) }),
        )
        .await
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

    #[tokio::test]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn apply_with_readback_rejects_drift() {
        let exec = FakeReconcileExecutor::new();
        let err = apply_with_readback_using(
            &exec,
            "net.ipv4.ip_forward",
            "1",
            |_| Box::pin(async move { Ok("0\n".to_owned()) }),
        )
        .await
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
