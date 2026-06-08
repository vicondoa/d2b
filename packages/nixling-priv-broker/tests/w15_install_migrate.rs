use std::os::fd::AsRawFd;

use nix::sys::socket::{socketpair, AddressFamily, SockFlag, SockType};
#[cfg(not(feature = "layer1-bootstrap"))]
use nixling_core::{
    bundle::{Bundle, BundleGeneration, BundleManagedKeys},
    bundle_resolver::{
        BundleResolver, HostRuntime, HostRuntimeArtifact, InstallerArtifact,
        ResolvedInstallerIntent,
    },
    host::HostJson,
    manifest_v04::ManifestV04,
    processes::ProcessesJson,
};
use nixling_ipc::broker_wire::{
    BrokerErrorResponse, BrokerResponse, RunHostInstallResponse, RunMigrateResponse,
};
#[cfg(not(feature = "layer1-bootstrap"))]
use nixling_ipc::{broker_wire::RunHostInstallRequest, types::BundleOpId};
use nixling_priv_broker::protocol::{recv_json_frame, send_json_frame};
#[cfg(not(feature = "layer1-bootstrap"))]
use nixling_priv_broker::{
    ops::exec_reconcile::{IpRouteVerb, ReconcileExecError, ReconcileExecutor, UsbipSubcommand},
    runtime::{dispatch_run_host_install_response, dispatch_run_host_install_response_for_intent},
};
#[cfg(not(feature = "layer1-bootstrap"))]
use std::path::{Path, PathBuf};

#[cfg(not(feature = "layer1-bootstrap"))]
const HOST_JSON_FIXTURE: &str =
    include_str!("../../../tests/fixtures/deny-unknown/host-valid.json");
#[cfg(not(feature = "layer1-bootstrap"))]
const MANIFEST_FIXTURE: &str = include_str!("../../../tests/golden/manifest_v04/baseline-vms.json");
#[cfg(not(feature = "layer1-bootstrap"))]
const LIVE_HANDLER_ACTION: &str =
    "Inspect the broker audit log for the failing live executor's underlying syscall.";

#[cfg(not(feature = "layer1-bootstrap"))]
fn run_host_install_request(intent_ref: &str) -> RunHostInstallRequest {
    RunHostInstallRequest {
        bundle_installer_intent_ref: BundleOpId::new(intent_ref),
        enable: false,
        start: false,
        no_start: false,
        tracing_span_id: None,
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn installer_bundle_resolver(public_manifest_path: &str) -> BundleResolver {
    let host: HostJson = serde_json::from_str(HOST_JSON_FIXTURE).expect("host fixture parses");
    let manifest =
        ManifestV04::from_slice(MANIFEST_FIXTURE.as_bytes()).expect("manifest fixture parses");
    let bundle = Bundle {
        bundle_version: 4,
        schema_version: "v2".to_owned(),
        public_manifest_path: public_manifest_path.to_owned(),
        host_path: "/ignored/host.json".to_owned(),
        processes_path: "/ignored/processes.json".to_owned(),
        privileges_path: "/ignored/privileges.json".to_owned(),
        closures: Vec::new(),
        minijail_profiles: Vec::new(),
        managed_keys: BundleManagedKeys::default(),
        generation: BundleGeneration {
            generator: "w15-test".to_owned(),
            source_revision: Some("test-rev".to_owned()),
            generated_at: Some("2025-01-01T00:00:00Z".to_owned()),
        },
        bundle_hash: None,
        artifact_hashes: None,
    };
    let processes = ProcessesJson {
        schema_version: "v2".to_owned(),
        vms: Vec::new(),
    };
    BundleResolver::from_artifacts(bundle, host, processes, manifest)
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn writable_artifact_path(name: &str) -> PathBuf {
    let base = std::env::var_os("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
    base.join("w15-install-negative").join(name)
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn writable_install_intent() -> ResolvedInstallerIntent {
    ResolvedInstallerIntent {
        intent_id: "installer:host".to_owned(),
        unit_path: writable_artifact_path("etc/systemd/system/nixlingd.service"),
        service_name: "nixlingd.service".to_owned(),
        daemon_config_path: writable_artifact_path("etc/nixling/daemon-config.json"),
        bundle_path: writable_artifact_path("current-bundle/manifest.json"),
        artifacts: vec![
            InstallerArtifact {
                path: writable_artifact_path("etc/systemd/system/nixlingd.service"),
                mode: 0o644,
                purpose: "nixlingd systemd unit (test override)".to_owned(),
            },
            InstallerArtifact {
                path: writable_artifact_path("etc/nixling/daemon-config.json"),
                mode: 0o640,
                purpose: "daemon configuration file (test override)".to_owned(),
            },
            InstallerArtifact {
                path: writable_artifact_path("share/nixling/vms.json"),
                mode: 0o644,
                purpose: "public manifest (test override)".to_owned(),
            },
        ],
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
fn writable_host_runtime() -> HostRuntimeArtifact {
    HostRuntimeArtifact {
        path: writable_artifact_path("runtime/host-runtime.json"),
        runtime: HostRuntime {
            schema_version: "v2".to_owned(),
            bundle_version: 4,
            generated_at: "2025-01-01T00:00:00Z".to_owned(),
            nft_applied_hash: None,
            ifnames: Vec::new(),
        },
    }
}

#[cfg(not(feature = "layer1-bootstrap"))]
#[derive(Debug, Default)]
struct FailingWriteExecutor;

#[cfg(not(feature = "layer1-bootstrap"))]
impl ReconcileExecutor for FailingWriteExecutor {
    fn apply_nft_script(
        &self,
        _nft_binary: &Path,
        _script: &str,
    ) -> Result<(), ReconcileExecError> {
        unreachable!("RunHostInstall should not apply nftables")
    }

    fn write_sysctl(&self, _key: &str, _value: &str) -> Result<(), ReconcileExecError> {
        unreachable!("RunHostInstall should not write sysctls")
    }

    fn write_atomic_file(
        &self,
        path: &Path,
        _contents: &[u8],
        _mode: u32,
    ) -> Result<(), ReconcileExecError> {
        Err(ReconcileExecError::Io {
            path: path.display().to_string(),
            detail: "injected write_atomic_file failure".to_owned(),
        })
    }

    fn write_path_value(&self, _path: &Path, _value: &str) -> Result<(), ReconcileExecError> {
        unreachable!("RunHostInstall should not write path values")
    }

    fn read_path_value(&self, _path: &Path) -> Result<String, ReconcileExecError> {
        unreachable!("RunHostInstall should not read path values")
    }

    fn ip_route(
        &self,
        _ip_binary: &Path,
        _verb: IpRouteVerb,
        _route_spec: &str,
    ) -> Result<(), ReconcileExecError> {
        unreachable!("RunHostInstall should not mutate routes")
    }

    fn run_usbip(
        &self,
        _usbip_binary: &Path,
        _subcommand: UsbipSubcommand,
        _bus_id: &str,
    ) -> Result<(), ReconcileExecError> {
        unreachable!("RunHostInstall should not invoke usbip")
    }

    fn prepare_store_view(
        &self,
        _intent: &nixling_core::bundle_resolver::ResolvedStoreViewIntent,
    ) -> Result<(), ReconcileExecError> {
        unreachable!("RunHostInstall should not prepare store views")
    }

    fn setup_mount_namespace(
        &self,
        _vm: &str,
        _role_id: &str,
        _source_view_path: &Path,
        _mount_root: &Path,
    ) -> Result<PathBuf, ReconcileExecError> {
        unreachable!("RunHostInstall should not set up mount namespaces")
    }

    fn run_activation_script(
        &self,
        _mode_arg: &str,
        _source_view_path: &Path,
        _mount_view_path: &Path,
    ) -> Result<String, ReconcileExecError> {
        unreachable!("RunHostInstall should not invoke activation")
    }

    fn run_gc(&self, _keep_generations: Option<u32>) -> Result<String, ReconcileExecError> {
        unreachable!("RunHostInstall should not invoke gc")
    }

    fn run_ssh_keygen(
        &self,
        _key_path: &Path,
        _comment: &str,
    ) -> Result<nixling_priv_broker::ops::exec_reconcile::GeneratedSshKey, ReconcileExecError> {
        unreachable!("RunHostInstall should not invoke ssh-keygen")
    }
}

#[test]
fn run_host_install_response_serializes_and_round_trips_via_send_json_frame() {
    let (left, right) = socketpair(
        AddressFamily::Unix,
        SockType::SeqPacket,
        None,
        SockFlag::SOCK_CLOEXEC,
    )
    .expect("socketpair");

    let body = BrokerResponse::RunHostInstall(RunHostInstallResponse {
        installed: true,
        enabled: false,
        started: false,
        artifacts_written: vec!["/etc/systemd/system/nixlingd.service".to_owned()],
    });
    send_json_frame(left.as_raw_fd(), &body).expect("send body");

    let decoded = recv_json_frame::<BrokerResponse>(right.as_raw_fd())
        .expect("recv frame")
        .expect("frame present");
    assert_eq!(decoded, body);
}

#[test]
fn run_migrate_response_serializes_and_round_trips_via_send_json_frame() {
    let (left, right) = socketpair(
        AddressFamily::Unix,
        SockType::SeqPacket,
        None,
        SockFlag::SOCK_CLOEXEC,
    )
    .expect("socketpair");

    let body = BrokerResponse::RunMigrate(RunMigrateResponse {
        migrated_vm_count: 3,
        notes: vec!["W15 marker test".to_owned()],
    });
    send_json_frame(left.as_raw_fd(), &body).expect("send body");

    let decoded = recv_json_frame::<BrokerResponse>(right.as_raw_fd())
        .expect("recv frame")
        .expect("frame present");
    assert_eq!(decoded, body);
}

#[test]
fn broker_error_response_for_bundle_resolver_unavailable_serializes_cleanly() {
    let (left, right) = socketpair(
        AddressFamily::Unix,
        SockType::SeqPacket,
        None,
        SockFlag::SOCK_CLOEXEC,
    )
    .expect("socketpair");

    let body = BrokerResponse::Error(BrokerErrorResponse {
        kind: "Broker.BundleResolverUnavailable".to_owned(),
        operation: "BundleResolver".to_owned(),
        target_wave: Some("W12".to_owned()),
        message: "Broker started without a loadable bundle".to_owned(),
        action: "Land bundle at /var/lib/nixling/current-bundle/manifest.json".to_owned(),
    });
    send_json_frame(left.as_raw_fd(), &body).expect("send body");

    let decoded = recv_json_frame::<BrokerResponse>(right.as_raw_fd())
        .expect("recv frame")
        .expect("frame present");
    assert_eq!(decoded, body);
}

#[cfg(not(feature = "layer1-bootstrap"))]
#[test]
fn bundle_resolver_unavailable_returns_typed_error() {
    let request = run_host_install_request("installer:host");

    let response = dispatch_run_host_install_response(&request, None, &FailingWriteExecutor);

    assert_eq!(
        response,
        BrokerResponse::Error(BrokerErrorResponse {
            kind: "Broker.BundleResolverUnavailable".to_owned(),
            operation: "BundleResolver".to_owned(),
            target_wave: Some("W12".to_owned()),
            message: "Broker started without a loadable bundle at ServerConfig.bundle_path. Bundle-dependent real-wire ops cannot resolve their BundleOpId refs.".to_owned(),
            action: "Land the bundle at /var/lib/nixling/current-bundle/manifest.json (or pass --bundle-path) and retry; the broker reloads the bundle on the next request.".to_owned(),
        })
    );
}

#[cfg(not(feature = "layer1-bootstrap"))]
#[test]
fn bundle_intent_missing_returns_typed_error() {
    let resolver = installer_bundle_resolver("/var/lib/nixling/current-bundle/vms.json");
    let request = run_host_install_request("installer:missing");

    let response =
        dispatch_run_host_install_response(&request, Some(&resolver), &FailingWriteExecutor);

    assert_eq!(
        response,
        BrokerResponse::Error(BrokerErrorResponse {
            kind: "Broker.BundleIntentMissing".to_owned(),
            operation: "BundleResolver".to_owned(),
            target_wave: Some("W12".to_owned()),
            message: "no installer intent in the trusted bundle for opaque id `installer:missing`"
                .to_owned(),
            action: "Confirm the daemon emitted the BundleOpId that matches the loaded bundle (nixos-modules/bundle.nix populates the intent table).".to_owned(),
        })
    );
}

#[cfg(not(feature = "layer1-bootstrap"))]
#[test]
fn live_run_host_install_propagates_write_failure() {
    let request = run_host_install_request("installer:host");
    let intent = writable_install_intent();
    let host_runtime = writable_host_runtime();

    let response = dispatch_run_host_install_response_for_intent(
        &request,
        &intent,
        Some(&host_runtime),
        &FailingWriteExecutor,
    );

    match response {
        BrokerResponse::Error(BrokerErrorResponse {
            kind,
            operation,
            target_wave,
            message,
            action,
        }) => {
            assert_eq!(kind, "Broker.LiveHandlerFailed");
            assert_eq!(operation, "LiveHandler");
            assert_eq!(target_wave.as_deref(), Some("W12"));
            assert!(message.contains("host install: failed to write artifact"));
            assert!(message.contains("I/O error on "));
            assert!(message.contains("injected write_atomic_file failure"));
            assert_eq!(action, LIVE_HANDLER_ACTION);
        }
        other => panic!("expected BrokerResponse::Error, got {other:?}"),
    }
}
