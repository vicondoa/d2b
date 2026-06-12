//! Daemon-side replacement for the per-VM
//! `nixling-known-hosts-refresh@<vm>.service` systemd
//! oneshot (defined in `nixos-modules/host-known-hosts.nix`).
//!
//! That unit ran a shell script that:
//!
//!   1. Looked up the VM in `share/nixling/vms.json`, skipping net
//!      VMs (no host-accessible sshd) and entries lacking a
//!      `staticIp`.
//!   2. Read the authoritative host pubkey from
//!      `/var/lib/nixling/vms/<vm>/sshd-host-keys/ssh_host_ed25519_key.pub`.
//!   3. Under a flock on `/var/lib/nixling/known_hosts.nixling.lock`,
//!      replaced any line matching the static IP in
//!      `/var/lib/nixling/known_hosts.nixling` with the fresh
//!      `<ip> <pubkey>` line. Idempotent: identical input → no
//!      rewrite.
//!
//! The daemon-only end state moves the side effect into the typed
//! broker `RunRotateKnownHost` op (reachable today via the public
//! `rotateKnownHost` verb — see
//! `dispatch_broker_rotate_known_host` in `lib.rs`). This module
//! splits that work into:
//!
//!   * a **pure** [`build_refresh_intent`] that turns
//!     `(vm, manifest)` into a [`RefreshIntent`] (either a
//!     `Skip{reason}` value or a fully-formed
//!     [`RunRotateKnownHostRequest`]), and
//!   * a **side-effect wrapper** [`refresh_known_hosts`] that takes
//!     a [`RotateKnownHostBroker`] trait object (so the daemon can
//!     plug in the live broker dispatcher and tests can plug in a
//!     fake), runs the intent, and returns a [`RefreshOutcome`].
//!
//! Idempotency contract: same `(vm, manifest)` always yields the
//! same `RefreshIntent`, and re-running `refresh_known_hosts`
//! against a broker that itself is idempotent (the existing
//! `RunRotateKnownHost` handler is — it only rewrites
//! `known_hosts.nixling` when the line actually differs) produces
//! the same `RefreshOutcome` on every call. The
//! `RunRotateKnownHostResponse::removed` field surfaces whether
//! the broker rewrote anything, which the daemon logs but does
//! not treat as a failure signal.

use nixling_core::bundle_resolver::intent_id_rotate_known_host;
use nixling_core::manifest_v04::ManifestV04;
use nixling_ipc::broker_wire::{RunRotateKnownHostRequest, RunRotateKnownHostResponse};
use nixling_ipc::types::BundleOpId;

use crate::typed_error::TypedError;

/// Why the daemon decided not to issue a `RunRotateKnownHost`
/// broker request for a particular VM. Mirrors the early-exit
/// branches in the legacy shell script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// VM name is not present in the loaded manifest. Treated as
    /// "no work to do" rather than an error so a `vm start` for a
    /// VM that disappeared from the bundle between resolve and
    /// post-readiness still reports success for the start; the
    /// audit log captures the missing entry.
    VmNotInManifest,
    /// Manifest entry has `is_net_vm = true`. Net VMs deliberately
    /// have no host-accessible sshd, so there is no key to pin.
    NetVm,
    /// Manifest entry has no `static_ip`. The legacy script
    /// keyed the `known_hosts.nixling` line on the static IP, so
    /// without one there is nothing to write.
    NoStaticIp,
}

impl SkipReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            SkipReason::VmNotInManifest => "vm-not-in-manifest",
            SkipReason::NetVm => "net-vm",
            SkipReason::NoStaticIp => "no-static-ip",
        }
    }
}

/// Pure description of "refresh known_hosts for VM X" derived
/// from the loaded manifest. Two equal inputs always yield two
/// equal `RefreshIntent`s (the broker request's
/// `bundle_rotate_known_host_intent_ref` is deterministic — it
/// hashes only the VM name via [`intent_id_rotate_known_host`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshIntent {
    Skip {
        vm: String,
        reason: SkipReason,
    },
    Rotate {
        vm: String,
        static_ip: String,
        broker_request: RunRotateKnownHostRequest,
    },
}

impl RefreshIntent {
    pub fn vm(&self) -> &str {
        match self {
            RefreshIntent::Skip { vm, .. } | RefreshIntent::Rotate { vm, .. } => vm.as_str(),
        }
    }
}

/// Build the canonical refresh intent for `vm` from `manifest`.
///
/// Pure: no I/O, no clocks, no environment access. Determinism is
/// the contract `tests/static-fast.sh` and the integration test
/// in this module both lean on.
pub fn build_refresh_intent(vm: &str, manifest: &ManifestV04) -> RefreshIntent {
    let Some(entry) = manifest.vms.get(vm) else {
        return RefreshIntent::Skip {
            vm: vm.to_owned(),
            reason: SkipReason::VmNotInManifest,
        };
    };
    if entry.is_net_vm {
        return RefreshIntent::Skip {
            vm: vm.to_owned(),
            reason: SkipReason::NetVm,
        };
    }
    let static_ip = match entry.static_ip.as_ref() {
        Some(ip) if !ip.is_empty() => ip.clone(),
        _ => {
            return RefreshIntent::Skip {
                vm: vm.to_owned(),
                reason: SkipReason::NoStaticIp,
            };
        }
    };
    RefreshIntent::Rotate {
        vm: vm.to_owned(),
        static_ip,
        broker_request: RunRotateKnownHostRequest {
            bundle_rotate_known_host_intent_ref: BundleOpId::new(intent_id_rotate_known_host(vm)),
            vm: vm.to_owned(),
            tracing_span_id: None,
        },
    }
}

/// Side-effect boundary for the post-readiness refresh.
///
/// The daemon's production impl proxies to
/// `dispatch_broker_request(state, BrokerRequest::RunRotateKnownHost(..))`;
/// tests pass in a fake that records every invocation so we can
/// assert idempotency without spinning up a broker.
pub trait RotateKnownHostBroker {
    fn rotate(
        &self,
        request: RunRotateKnownHostRequest,
    ) -> Result<RunRotateKnownHostResponse, TypedError>;
}

/// Outcome of running the refresh once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshOutcome {
    Skipped {
        vm: String,
        reason: SkipReason,
    },
    Rotated {
        vm: String,
        response: RunRotateKnownHostResponse,
    },
    Failed {
        vm: String,
        detail: String,
    },
}

impl RefreshOutcome {
    /// True when the call reached the broker and produced a
    /// response (regardless of whether the broker actually
    /// rewrote `known_hosts.nixling`; that's
    /// [`RunRotateKnownHostResponse::removed`]).
    pub fn rotated(&self) -> bool {
        matches!(self, RefreshOutcome::Rotated { .. })
    }
}

/// Side-effect wrapper. Builds the intent and, if non-skip,
/// dispatches the broker op via `broker`. Failures are surfaced
/// as `RefreshOutcome::Failed` so the caller can decide whether
/// to log-and-continue (current policy, matching the legacy
/// "warn-only" shell script) or escalate.
pub fn refresh_known_hosts<B: RotateKnownHostBroker>(
    vm: &str,
    manifest: &ManifestV04,
    broker: &B,
) -> RefreshOutcome {
    match build_refresh_intent(vm, manifest) {
        RefreshIntent::Skip { vm, reason } => RefreshOutcome::Skipped { vm, reason },
        RefreshIntent::Rotate {
            vm, broker_request, ..
        } => match broker.rotate(broker_request) {
            Ok(response) => RefreshOutcome::Rotated { vm, response },
            Err(err) => RefreshOutcome::Failed {
                vm,
                detail: format!("{err:?}"),
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    use nixling_core::manifest_v04::{
        ManifestMeta, ManifestV04, ObservabilityMeta, VmEntry, VmObservability,
    };

    use super::*;

    fn vm_entry(name: &str, is_net_vm: bool, static_ip: Option<&str>) -> VmEntry {
        VmEntry {
            api_socket: format!("/run/nixling/vms/{name}/api.sock"),
            audio: false,
            audio_service: String::new(),
            audio_state_file: String::new(),
            bridge: Some("br-test".to_owned()),
            env: Some("test".to_owned()),
            mtu: Some(1500),
            mss_clamp: Some(1460),
            lan: None,
            gpu_socket: String::new(),
            graphics: false,
            is_net_vm,
            name: name.to_owned(),
            net_vm: Some("sys-test-net".to_owned()),
            observability: VmObservability {
                agent_socket: format!("/run/nixling/vms/{name}/agent.sock"),
                enabled: false,
                vsock_cid: 42,
                vsock_host_socket: format!("/run/nixling/vms/{name}/agent-host.sock"),
            },
            ssh_user: Some("alice".to_owned()),
            state_dir: format!("/var/lib/nixling/vms/{name}"),
            static_ip: static_ip.map(str::to_owned),
            tap: format!("tap-{name}"),
            tpm: false,
            tpm_socket: String::new(),
            usbip_yubikey: false,
            usbipd_host_ip: None,
        }
    }

    fn manifest_with(vms: Vec<VmEntry>) -> ManifestV04 {
        ManifestV04 {
            manifest: ManifestMeta {
                manifest_version: 4,
            },
            observability: ObservabilityMeta {
                enabled: false,
                obs_vsock_cid: 3,
                obs_vsock_host_socket: "/run/nixling/obs.sock".to_owned(),
                signoz_otlp_grpc_port: 4317,
                signoz_otlp_http_port: 4318,
                signoz_url: "http://127.0.0.1:8080".to_owned(),
                vm_name: "obs".to_owned(),
            },
            vms: vms.into_iter().map(|v| (v.name.clone(), v)).collect(),
        }
    }

    #[test]
    fn skip_when_vm_missing_from_manifest() {
        let manifest = manifest_with(vec![]);
        let intent = build_refresh_intent("ghost", &manifest);
        assert_eq!(
            intent,
            RefreshIntent::Skip {
                vm: "ghost".to_owned(),
                reason: SkipReason::VmNotInManifest,
            }
        );
    }

    #[test]
    fn skip_for_net_vm() {
        let manifest = manifest_with(vec![vm_entry("sys-test-net", true, Some("192.0.2.1"))]);
        let intent = build_refresh_intent("sys-test-net", &manifest);
        assert!(matches!(
            intent,
            RefreshIntent::Skip {
                reason: SkipReason::NetVm,
                ..
            }
        ));
    }

    #[test]
    fn skip_when_static_ip_missing_or_empty() {
        let manifest = manifest_with(vec![
            vm_entry("no-ip", false, None),
            vm_entry("empty-ip", false, Some("")),
        ]);
        assert!(matches!(
            build_refresh_intent("no-ip", &manifest),
            RefreshIntent::Skip {
                reason: SkipReason::NoStaticIp,
                ..
            }
        ));
        assert!(matches!(
            build_refresh_intent("empty-ip", &manifest),
            RefreshIntent::Skip {
                reason: SkipReason::NoStaticIp,
                ..
            }
        ));
    }

    #[test]
    fn rotate_intent_is_deterministic() {
        let manifest = manifest_with(vec![vm_entry("personal-dev", false, Some("192.0.2.20"))]);
        let a = build_refresh_intent("personal-dev", &manifest);
        let b = build_refresh_intent("personal-dev", &manifest);
        assert_eq!(a, b);
        let RefreshIntent::Rotate {
            vm,
            static_ip,
            broker_request,
        } = a
        else {
            panic!("expected Rotate intent");
        };
        assert_eq!(vm, "personal-dev");
        assert_eq!(static_ip, "192.0.2.20");
        assert_eq!(broker_request.vm, "personal-dev");
        assert_eq!(
            broker_request.bundle_rotate_known_host_intent_ref.as_str(),
            "rotate-known-host:vm:personal-dev"
        );
        assert!(broker_request.tracing_span_id.is_none());
    }

    /// Fake broker that records every `rotate` call and returns a
    /// deterministic response. Used to drive the idempotency
    /// assertion below — the same input must produce the same
    /// outcome, and the broker must see exactly one call per
    /// `refresh_known_hosts` invocation regardless of the call
    /// count.
    struct RecordingBroker {
        calls: RefCell<Vec<RunRotateKnownHostRequest>>,
        // The real broker writes idempotently: only the first
        // call rewrites the file; subsequent calls observe an
        // unchanged line and return `removed = false`. We mimic
        // that here so the integration test exercises the same
        // shape.
        remembered_ip: RefCell<BTreeMap<String, String>>,
    }

    impl RecordingBroker {
        fn new() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                remembered_ip: RefCell::new(BTreeMap::new()),
            }
        }
    }

    impl RotateKnownHostBroker for RecordingBroker {
        fn rotate(
            &self,
            request: RunRotateKnownHostRequest,
        ) -> Result<RunRotateKnownHostResponse, TypedError> {
            self.calls.borrow_mut().push(request.clone());
            let static_ip = "192.0.2.20".to_owned();
            let first = self
                .remembered_ip
                .borrow_mut()
                .insert(request.vm.clone(), static_ip.clone())
                .is_none();
            Ok(RunRotateKnownHostResponse {
                vm: request.vm,
                static_ip,
                known_hosts_path: "/var/lib/nixling/known_hosts.nixling".to_owned(),
                removed: first,
            })
        }
    }

    #[test]
    fn refresh_skips_without_calling_broker() {
        let manifest = manifest_with(vec![vm_entry("sys-test-net", true, Some("192.0.2.1"))]);
        let broker = RecordingBroker::new();
        let outcome = refresh_known_hosts("sys-test-net", &manifest, &broker);
        assert_eq!(
            outcome,
            RefreshOutcome::Skipped {
                vm: "sys-test-net".to_owned(),
                reason: SkipReason::NetVm,
            }
        );
        assert!(
            broker.calls.borrow().is_empty(),
            "skip must not call broker"
        );
    }

    /// Integration test for the idempotency contract: invoking
    /// `refresh_known_hosts` twice with identical inputs must
    /// produce two structurally identical `Rotated` outcomes (the
    /// only difference allowed is the broker's `removed` field,
    /// which mirrors the file-touching behaviour of the legacy
    /// flock-protected shell script — first call writes, second
    /// is a no-op). No spurious rotation is signalled by the pure
    /// intent layer.
    #[test]
    fn refresh_runs_idempotently() {
        let manifest = manifest_with(vec![vm_entry("personal-dev", false, Some("192.0.2.20"))]);
        let broker = RecordingBroker::new();

        let first = refresh_known_hosts("personal-dev", &manifest, &broker);
        let second = refresh_known_hosts("personal-dev", &manifest, &broker);

        let (
            RefreshOutcome::Rotated {
                vm: v1,
                response: r1,
            },
            RefreshOutcome::Rotated {
                vm: v2,
                response: r2,
            },
        ) = (&first, &second)
        else {
            panic!("expected both refreshes to rotate; got {first:?} / {second:?}");
        };
        assert_eq!(v1, v2);
        assert_eq!(r1.vm, r2.vm);
        assert_eq!(r1.static_ip, r2.static_ip);
        assert_eq!(r1.known_hosts_path, r2.known_hosts_path);
        // First call writes, second observes the unchanged line.
        assert!(r1.removed, "first refresh writes the line");
        assert!(!r2.removed, "second refresh is a no-op (idempotent)");

        // Both broker invocations were structurally identical —
        // the daemon never re-derives a different request for the
        // same `(vm, manifest)` pair.
        let calls = broker.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], calls[1]);
        assert_eq!(calls[0].vm, "personal-dev");
        assert_eq!(
            calls[0].bundle_rotate_known_host_intent_ref.as_str(),
            "rotate-known-host:vm:personal-dev"
        );
    }
}
