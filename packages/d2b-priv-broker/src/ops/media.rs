//! qemu-media physical USB enrollment and direct image open helpers.
//!
//! Public callers identify media by VM + bundle source id. The only place raw
//! USB identity appears is the root-owned registry and runtime udev rule file;
//! direct image paths come only from trusted operator-authored Nix config.

use std::collections::BTreeSet;
use std::io::{self, BufRead, BufReader, Write};
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use d2b_contracts::broker_wire::{
    QemuMediaBootRequest, QemuMediaEnrollRequest, QemuMediaEnrollResponse, QemuMediaHotplugEvent,
    QemuMediaHotplugRequest, QemuMediaHotplugResponse, QemuMediaHotplugStatus,
    QemuMediaLifecycleAction, QemuMediaLifecycleRequest, QemuMediaLifecycleResponse,
    QemuMediaQueryStatusRequest, QemuMediaQueryStatusResponse, QemuMediaRefreshRegistryResponse,
    QemuMediaVmStatus,
};
use d2b_core::bundle_resolver::BundleResolver;
use d2b_core::host::{
    QemuMediaFormat, QemuMediaSourceIntent, QemuMediaSourceKind, QemuMediaUsbSelector,
};
use d2b_host::media::{
    MediaAccessMode, QemuMediaHotplugAction, QemuMediaHotplugScaffold, SafeUsbCandidate,
    UsbPhysicalIdentity,
};
use nix::libc;
use nix::unistd::{Group, Uid};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Debug)]
pub enum MediaOpError {
    InvalidRef(String),
    InvalidBusId(String),
    MissingBundlePolicy,
    UnsupportedSourceKind,
    MissingImagePath,
    UnsupportedImageFormat,
    ImagePathUnsafe(String),
    Sysfs(String),
    NoBlockDevice,
    AmbiguousBlockDevice(Vec<String>),
    MissingById,
    DeviceBusy(String),
    IdentityMismatch(String),
    AmbiguousRuntimeSelector(Vec<String>),
    Io(String),
    Registry(String),
    Open(String),
    ImageBusy(String),
    QmpScaffold(String),
    Qmp(String),
}

impl std::fmt::Display for MediaOpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRef(reason) => write!(f, "invalid-media-ref:{reason}"),
            Self::InvalidBusId(reason) => write!(f, "invalid-busid:{reason}"),
            Self::MissingBundlePolicy => write!(f, "qemu-media-ref-not-declared"),
            Self::UnsupportedSourceKind => write!(f, "qemu-media-ref-not-physical-usb"),
            Self::MissingImagePath => write!(f, "qemu-media-image-path-missing"),
            Self::UnsupportedImageFormat => write!(f, "qemu-media-image-format-not-raw"),
            Self::ImagePathUnsafe(reason) => write!(f, "qemu-media-image-path-unsafe:{reason}"),
            Self::Sysfs(reason) => write!(f, "sysfs-readback-failed:{reason}"),
            Self::NoBlockDevice => write!(f, "usb-device-has-no-block-device"),
            Self::AmbiguousBlockDevice(devices) => {
                write!(f, "usb-device-has-multiple-block-devices:{}", devices.len())
            }
            Self::MissingById => write!(f, "usb-device-has-no-by-id-readback"),
            Self::DeviceBusy(reason) => write!(f, "media-device-busy:{reason}"),
            Self::IdentityMismatch(reason) => write!(f, "media-identity-mismatch:{reason}"),
            Self::AmbiguousRuntimeSelector(media_refs) => {
                write!(f, "media-runtime-selector-ambiguous:{}", media_refs.len())
            }
            Self::Io(reason) => write!(f, "io:{reason}"),
            Self::Registry(reason) => write!(f, "registry:{reason}"),
            Self::Open(reason) => write!(f, "open:{reason}"),
            Self::ImageBusy(reason) => write!(f, "qemu-media-image-busy:{reason}"),
            Self::QmpScaffold(reason) => write!(f, "qmp-scaffold:{reason}"),
            Self::Qmp(reason) => write!(f, "qmp:{reason}"),
        }
    }
}

impl std::error::Error for MediaOpError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MediaRegistryRecord {
    schema_version: u32,
    vm: String,
    media_ref: String,
    source_kind: String,
    format: String,
    read_only: bool,
    identity: RegistryIdentity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RegistryIdentity {
    bus_id: String,
    devnum: u16,
    vendor_id: String,
    product_id: String,
    by_id_names: Vec<String>,
    block_device: String,
}

const REDACTED_INDEX_STORAGE_REF: &str = "path:qemu-media-redacted-index";
const QMP_MAX_RESPONSE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RedactedRegistryIndex {
    schema_version: u32,
    records: Vec<RedactedRegistryRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RedactedRegistryRecord {
    vm: String,
    media_ref: String,
    source_kind: String,
    format: String,
    read_only: bool,
    identity_hash: String,
}

impl MediaRegistryRecord {
    fn from_identity(source: &QemuMediaSourceIntent, identity: UsbPhysicalIdentity) -> Self {
        Self {
            schema_version: 1,
            vm: source.vm.clone(),
            media_ref: source.media_ref.clone(),
            source_kind: "physical-usb".to_owned(),
            format: format!("{:?}", source.format).to_ascii_lowercase(),
            read_only: source.read_only,
            identity: RegistryIdentity {
                bus_id: identity.bus_id,
                devnum: identity.devnum,
                vendor_id: identity.vendor_id,
                product_id: identity.product_id,
                by_id_names: identity.by_id_names,
                block_device: identity.block_device,
            },
        }
    }

    fn redacted(&self) -> RedactedRegistryRecord {
        RedactedRegistryRecord {
            vm: self.vm.clone(),
            media_ref: self.media_ref.clone(),
            source_kind: self.source_kind.clone(),
            format: self.format.clone(),
            read_only: self.read_only,
            identity_hash: qemu_media_identity_hash(&self.identity.by_id_names),
        }
    }
}

pub struct EnrollOutcome {
    pub response: QemuMediaEnrollResponse,
    pub by_id_count: u32,
}

pub struct HotplugOutcome {
    pub response: QemuMediaHotplugResponse,
}

pub struct BootOutcome {
    pub response: QemuMediaHotplugResponse,
    pub registry_record_written: bool,
    pub redacted_index_written: bool,
    pub udev_rule_written: bool,
    pub udev_reloaded: bool,
}

pub struct RefreshOutcome {
    pub response: QemuMediaRefreshRegistryResponse,
}

pub fn enroll(
    resolver: &BundleResolver,
    req: &QemuMediaEnrollRequest,
) -> Result<EnrollOutcome, MediaOpError> {
    d2b_host::media::validate_media_ref(req.media_ref.as_str())
        .map_err(|err| MediaOpError::InvalidRef(err.to_string()))?;
    d2b_host::media::validate_usb_busid(&req.bus_id)
        .map_err(|err| MediaOpError::InvalidBusId(err.to_string()))?;
    let source = resolve_physical_source(resolver, req.vm_id.as_str(), req.media_ref.as_str())?;
    let identity = read_usb_identity(Path::new("/sys"), Path::new("/dev/disk/by-id"), &req.bus_id)?;
    preflight_identity_not_busy(Path::new("/sys"), &identity)?;
    let access = access_mode(source);
    let fd = open_block_device(&identity.block_device, access)?;
    drop(fd);

    let by_id_count = u32::try_from(identity.by_id_names.len()).unwrap_or(u32::MAX);
    let identity_hash = qemu_media_identity_hash(&identity.by_id_names);
    let existing_records = read_all_registry_records(resolver).unwrap_or_default();
    if existing_records.iter().any(|record| {
        record.vm == source.vm
            && record.media_ref != source.media_ref
            && qemu_media_identity_hash(&record.identity.by_id_names) == identity_hash
    }) {
        return Err(MediaOpError::IdentityMismatch(
            "already-enrolled-different-ref".to_owned(),
        ));
    }
    let record = MediaRegistryRecord::from_identity(source, identity);
    write_registry_record(resolver, &record)?;
    let records = read_all_registry_records(resolver).unwrap_or_else(|_| vec![record.clone()]);
    write_redacted_registry_index(resolver, &records)?;
    let udev_rule_written = write_runtime_udev_rules(resolver, &records)?;
    let udev_reloaded = reload_udev_rules();

    Ok(EnrollOutcome {
        response: QemuMediaEnrollResponse {
            vm_id: req.vm_id.clone(),
            media_ref: req.media_ref.clone(),
            read_only: source.read_only,
            enrolled: true,
            udev_rule_written,
            udev_reloaded,
        },
        by_id_count,
    })
}

pub fn refresh_registry(resolver: &BundleResolver) -> Result<RefreshOutcome, MediaOpError> {
    let records = read_all_registry_records(resolver)?;
    let redacted_index_written = write_redacted_registry_index(resolver, &records).map(|_| true)?;
    let udev_rule_written = write_runtime_udev_rules(resolver, &records)?;
    let udev_reloaded = reload_udev_rules();
    Ok(RefreshOutcome {
        response: QemuMediaRefreshRegistryResponse {
            record_count: u32::try_from(records.len()).unwrap_or(u32::MAX),
            redacted_index_written,
            udev_rule_written,
            udev_reloaded,
        },
    })
}

pub fn boot(
    resolver: &BundleResolver,
    req: &QemuMediaBootRequest,
) -> Result<BootOutcome, MediaOpError> {
    let source = resolve_boot_source(resolver, req.vm_id.as_str())?;
    let (opened, audit) = open_declared_source(resolver, source)?;
    let outcome = run_attach_transaction(req.vm_id.as_str(), opened, true)?;
    Ok(BootOutcome {
        response: outcome.response,
        registry_record_written: audit.registry_record_written,
        redacted_index_written: audit.redacted_index_written,
        udev_rule_written: audit.udev_rule_written,
        udev_reloaded: audit.udev_reloaded,
    })
}

pub fn system_powerdown(
    req: &QemuMediaLifecycleRequest,
) -> Result<QemuMediaLifecycleResponse, MediaOpError> {
    let mut client = QmpClient::connect(&qmp_socket_path(req.vm_id.as_str()))?;
    qmp_system_powerdown(&mut client)?;
    Ok(QemuMediaLifecycleResponse {
        vm_id: req.vm_id.clone(),
        command: QemuMediaLifecycleAction::SystemPowerdown,
    })
}

pub fn query_status(
    req: &QemuMediaQueryStatusRequest,
) -> Result<QemuMediaQueryStatusResponse, MediaOpError> {
    qmp_query_status_from_path(
        &req.vm_id,
        &qmp_socket_path(req.vm_id.as_str()),
        req.shutdown_context,
    )
}

fn qmp_query_status_from_path(
    vm_id: &d2b_contracts::types::VmId,
    path: &Path,
    shutdown_context: bool,
) -> Result<QemuMediaQueryStatusResponse, MediaOpError> {
    let mut client = match QmpClient::connect(path) {
        Ok(client) => client,
        Err(error) if shutdown_context && qmp_error_is_expected_shutdown_disconnect(&error) => {
            return Ok(QemuMediaQueryStatusResponse {
                vm_id: vm_id.clone(),
                status: QemuMediaVmStatus::ConnectionLostDuringShutdown,
            });
        }
        Err(error) => return Err(error),
    };
    match qmp_query_status(&mut client) {
        Ok(status) => Ok(QemuMediaQueryStatusResponse {
            vm_id: vm_id.clone(),
            status,
        }),
        Err(error) if shutdown_context && qmp_error_is_expected_shutdown_disconnect(&error) => {
            Ok(QemuMediaQueryStatusResponse {
                vm_id: vm_id.clone(),
                status: QemuMediaVmStatus::ConnectionLostDuringShutdown,
            })
        }
        Err(error) => Err(error),
    }
}

pub fn quit(req: &QemuMediaLifecycleRequest) -> Result<QemuMediaLifecycleResponse, MediaOpError> {
    let mut client = QmpClient::connect(&qmp_socket_path(req.vm_id.as_str()))?;
    qmp_quit(&mut client)?;
    Ok(QemuMediaLifecycleResponse {
        vm_id: req.vm_id.clone(),
        command: QemuMediaLifecycleAction::Quit,
    })
}

pub fn attach(
    resolver: &BundleResolver,
    req: &QemuMediaHotplugRequest,
) -> Result<HotplugOutcome, MediaOpError> {
    let opened = open_runtime_selector_source(resolver, req)?;
    run_attach_transaction(req.vm_id.as_str(), opened, false)
}

pub fn detach(
    resolver: &BundleResolver,
    req: &QemuMediaHotplugRequest,
) -> Result<HotplugOutcome, MediaOpError> {
    d2b_host::media::validate_usb_busid(&req.bus_id)
        .map_err(|err| MediaOpError::InvalidBusId(err.to_string()))?;
    let identity = read_usb_identity(Path::new("/sys"), Path::new("/dev/disk/by-id"), &req.bus_id)?;
    let mut client = QmpClient::connect(&qmp_socket_path(req.vm_id.as_str()))?;
    let (record, source) =
        resolve_detach_runtime_selector(resolver, req.vm_id.as_str(), &identity, &mut client)?;
    let scaffold = qmp_scaffold(
        &record.media_ref,
        &source.slot,
        QemuMediaHotplugAction::Detach,
    )?;
    let commands = qmp_detach(&mut client, &scaffold)?;
    Ok(HotplugOutcome {
        response: hotplug_response(
            req.vm_id.as_str(),
            source,
            scaffold,
            commands,
            vec![
                QemuMediaHotplugStatus::IdentityResolved,
                QemuMediaHotplugStatus::QmpConnected,
                QemuMediaHotplugStatus::QmpCapabilities,
                QemuMediaHotplugStatus::DeviceDeleted,
                QemuMediaHotplugStatus::BlockdevDeleted,
                QemuMediaHotplugStatus::FdRemoved,
            ],
        ),
    })
}

struct OpenedMedia<'a> {
    source: &'a QemuMediaSourceIntent,
    fd: OwnedFd,
}

#[derive(Debug, Clone, Copy, Default)]
struct BootSourceAudit {
    registry_record_written: bool,
    redacted_index_written: bool,
    udev_rule_written: bool,
    udev_reloaded: bool,
}

fn open_runtime_selector_source<'a>(
    resolver: &'a BundleResolver,
    req: &QemuMediaHotplugRequest,
) -> Result<OpenedMedia<'a>, MediaOpError> {
    d2b_host::media::validate_usb_busid(&req.bus_id)
        .map_err(|err| MediaOpError::InvalidBusId(err.to_string()))?;
    let identity = read_usb_identity(Path::new("/sys"), Path::new("/dev/disk/by-id"), &req.bus_id)?;
    let (_record, source) = resolve_runtime_selector(resolver, req.vm_id.as_str(), &identity)
        .or_else(|error| {
            if runtime_selector_allows_declared_fallback(&error) {
                resolve_declared_runtime_selector(resolver, req.vm_id.as_str(), &identity)
            } else {
                Err(error)
            }
        })?;
    preflight_identity_not_busy(Path::new("/sys"), &identity)?;
    let fd = open_block_device(&identity.block_device, access_mode(source))?;
    Ok(OpenedMedia { source, fd })
}

fn open_declared_source<'a>(
    resolver: &'a BundleResolver,
    source: &'a QemuMediaSourceIntent,
) -> Result<(OpenedMedia<'a>, BootSourceAudit), MediaOpError> {
    let access = access_mode(source);
    let mut audit = BootSourceAudit::default();
    let fd = match source.source_kind {
        QemuMediaSourceKind::PhysicalUsb => {
            let identity = if let Some(selector) = source.usb_selector.as_ref() {
                let identity = read_usb_identity_for_selector(selector)?;
                audit = write_declared_selector_artifacts(
                    &registry_dir(resolver)?,
                    &redacted_index_path(resolver)?,
                    &rules_path(resolver)?,
                    source,
                    &identity,
                    0,
                    0,
                    true,
                )?;
                if !audit.udev_reloaded {
                    tracing::warn!(
                        vm_id = %source.vm,
                        media_ref = %source.media_ref,
                        slot = %source.slot,
                        "qemu-media boot: udev rules reload failed after runtime selector update"
                    );
                }
                identity
            } else {
                let record =
                    read_registry_record(resolver, source.vm.as_str(), source.media_ref.as_str())?;
                read_current_identity_for_record(&record)?
            };
            preflight_identity_not_busy(Path::new("/sys"), &identity)?;
            open_block_device(&identity.block_device, access)?
        }
        QemuMediaSourceKind::ImageFile => open_image_file(source, access)?,
    };
    Ok((OpenedMedia { source, fd }, audit))
}

fn run_attach_transaction(
    vm: &str,
    opened: OpenedMedia<'_>,
    continue_vm: bool,
) -> Result<HotplugOutcome, MediaOpError> {
    let source = opened.source;
    let scaffold = qmp_scaffold(
        source.media_ref.as_str(),
        source.slot.as_str(),
        QemuMediaHotplugAction::Attach,
    )?;
    let mut client = QmpClient::connect(&qmp_socket_path(vm))?;
    let mut statuses = vec![
        QemuMediaHotplugStatus::IdentityResolved,
        QemuMediaHotplugStatus::QmpConnected,
        QemuMediaHotplugStatus::QmpCapabilities,
    ];
    let mut commands = qmp_attach(
        &mut client,
        &scaffold,
        opened.fd.as_raw_fd(),
        source.source_kind,
        source.read_only,
    )?;
    statuses.extend([
        QemuMediaHotplugStatus::FdAdded,
        QemuMediaHotplugStatus::BlockdevAdded,
        QemuMediaHotplugStatus::DeviceAdded,
    ]);
    if continue_vm {
        if let Err(error) = qmp_continue_vm(&mut client) {
            let _ = qmp_detach(&mut client, &scaffold);
            return Err(error);
        }
        commands.push("cont".to_owned());
        statuses.push(QemuMediaHotplugStatus::VmContinued);
    }
    Ok(HotplugOutcome {
        response: hotplug_response(vm, source, scaffold, commands, statuses),
    })
}

fn qmp_continue_vm(client: &mut QmpClient) -> Result<(), MediaOpError> {
    client.execute("cont", json!({}), None)?;
    Ok(())
}

fn qmp_system_powerdown(client: &mut QmpClient) -> Result<(), MediaOpError> {
    client.execute("system_powerdown", json!({}), None)?;
    Ok(())
}

fn qmp_quit(client: &mut QmpClient) -> Result<(), MediaOpError> {
    client.execute("quit", json!({}), None)?;
    Ok(())
}

fn qmp_query_status(client: &mut QmpClient) -> Result<QemuMediaVmStatus, MediaOpError> {
    let value = client.execute("query-status", json!({}), None)?;
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| MediaOpError::Qmp("query-status:missing-status".to_owned()))?;
    Ok(qmp_status_from_str(status))
}

fn qmp_status_from_str(status: &str) -> QemuMediaVmStatus {
    match status {
        "running" => QemuMediaVmStatus::Running,
        "paused" => QemuMediaVmStatus::Paused,
        "shutdown" => QemuMediaVmStatus::Shutdown,
        "suspended" => QemuMediaVmStatus::Suspended,
        "watchdog" => QemuMediaVmStatus::Watchdog,
        "debug" => QemuMediaVmStatus::Debug,
        "inmigrate" => QemuMediaVmStatus::Inmigrate,
        "internal-error" => QemuMediaVmStatus::InternalError,
        "io-error" => QemuMediaVmStatus::IoError,
        "postmigrate" => QemuMediaVmStatus::Postmigrate,
        "prelaunch" => QemuMediaVmStatus::Prelaunch,
        "finish-migrate" => QemuMediaVmStatus::FinishMigrate,
        "restore-vm" => QemuMediaVmStatus::RestoreVm,
        "save-vm" => QemuMediaVmStatus::SaveVm,
        "guest-panicked" => QemuMediaVmStatus::GuestPanicked,
        "colo" => QemuMediaVmStatus::Colo,
        "preconfig" => QemuMediaVmStatus::Preconfig,
        _ => QemuMediaVmStatus::Unknown,
    }
}

fn qmp_error_is_expected_shutdown_disconnect(error: &MediaOpError) -> bool {
    let MediaOpError::Qmp(reason) = error else {
        return false;
    };
    reason == "eof"
        || reason.contains("Broken pipe")
        || reason.contains("Connection reset by peer")
        || reason.contains("Connection refused")
        || reason.contains("No such file or directory")
        || reason.contains("Not connected")
}

fn hotplug_response(
    vm: &str,
    source: &QemuMediaSourceIntent,
    scaffold: QemuMediaHotplugScaffold,
    qmp_commands: Vec<String>,
    statuses: Vec<QemuMediaHotplugStatus>,
) -> QemuMediaHotplugResponse {
    QemuMediaHotplugResponse {
        vm_id: d2b_contracts::types::VmId::new(vm.to_owned()),
        media_ref: d2b_contracts::types::MediaRef::new(scaffold.media_ref),
        slot: scaffold.slot,
        read_only: source.read_only,
        qmp_commands,
        events: statuses
            .into_iter()
            .map(|status| QemuMediaHotplugEvent { status })
            .collect(),
    }
}

fn resolve_boot_source<'a>(
    resolver: &'a BundleResolver,
    vm: &str,
) -> Result<&'a QemuMediaSourceIntent, MediaOpError> {
    resolver
        .host
        .qemu_media
        .as_ref()
        .and_then(|qemu_media| {
            qemu_media
                .sources
                .iter()
                .find(|source| source.vm == vm && source.slot == "boot")
        })
        .ok_or(MediaOpError::MissingBundlePolicy)
}

fn qmp_scaffold(
    media_ref: &str,
    slot: &str,
    action: QemuMediaHotplugAction,
) -> Result<QemuMediaHotplugScaffold, MediaOpError> {
    d2b_host::media::qemu_media_hotplug_scaffold(media_ref, slot, action)
        .map_err(|err| MediaOpError::QmpScaffold(format!("{err:?}")))
}

fn qmp_socket_path(vm: &str) -> PathBuf {
    PathBuf::from("/run/d2b/vms").join(vm).join("qmp.sock")
}

fn qemu_media_file_node_id(media_ref: &str) -> String {
    format!("d2b-file-{media_ref}")
}

fn qmp_attach(
    client: &mut QmpClient,
    scaffold: &QemuMediaHotplugScaffold,
    fd: i32,
    source_kind: QemuMediaSourceKind,
    read_only: bool,
) -> Result<Vec<String>, MediaOpError> {
    let file_node_id = qemu_media_file_node_id(&scaffold.media_ref);
    let file_driver = match source_kind {
        QemuMediaSourceKind::PhysicalUsb => "host_device",
        QemuMediaSourceKind::ImageFile => "file",
    };
    let mut cleanup = QmpAttachCleanup::new(scaffold, file_node_id.clone());
    cleanup.fdset_added = true;
    match client.execute(
        "add-fd",
        json!({
            "opaque": format!("d2b:{}", scaffold.media_ref),
        }),
        Some(fd),
    ) {
        Ok(value) => {
            cleanup.fdset_id = value.get("fdset-id").and_then(Value::as_u64);
            cleanup.fd = value.get("fd").and_then(Value::as_u64);
            if cleanup.fdset_id.is_none() || cleanup.fd.is_none() {
                cleanup.rollback(client);
                return Err(MediaOpError::Qmp("add-fd-missing-return-fields".to_owned()));
            }
        }
        Err(error) => {
            cleanup.rollback(client);
            return Err(error);
        }
    }
    if let Err(error) = client.execute(
        "blockdev-add",
        json!({
            "driver": file_driver,
            "filename": format!("/dev/fdset/{}", cleanup.fdset_id.expect("validated fdset id")),
            "node-name": file_node_id.as_str(),
            "read-only": read_only,
        }),
        None,
    ) {
        if qmp_error_may_have_applied(&error) {
            cleanup.file_added = true;
        }
        cleanup.rollback(client);
        return Err(error);
    }
    cleanup.file_added = true;
    if let Err(error) = client.execute(
        "blockdev-add",
        json!({
            "driver": "raw",
            "file": qemu_media_file_node_id(&scaffold.media_ref),
            "node-name": scaffold.blockdev_id.as_str(),
            "read-only": read_only,
        }),
        None,
    ) {
        if qmp_error_may_have_applied(&error) {
            cleanup.raw_added = true;
        }
        cleanup.rollback(client);
        return Err(error);
    }
    cleanup.raw_added = true;
    let mut device_args = json!({
        "driver": "usb-storage",
        "drive": scaffold.blockdev_id.as_str(),
        "id": scaffold.device_id.as_str(),
        "bus": "ehci.0",
        "removable": true,
    });
    if scaffold.slot == "boot"
        && let Some(args) = device_args.as_object_mut()
    {
        args.insert("bootindex".to_owned(), json!(1));
    }
    if let Err(error) = client.execute("device_add", device_args, None) {
        if qmp_error_may_have_applied(&error) {
            cleanup.device_added = true;
        }
        cleanup.rollback(client);
        return Err(error);
    }
    cleanup.device_added = true;
    Ok(vec![
        "add-fd".to_owned(),
        format!("blockdev-add:{file_driver}"),
        "blockdev-add:raw".to_owned(),
        "device_add".to_owned(),
    ])
}

fn qmp_error_may_have_applied(error: &MediaOpError) -> bool {
    matches!(
        error,
        MediaOpError::Qmp(reason)
            if reason.starts_with("read:")
                || reason.starts_with("write:")
                || reason.starts_with("flush:")
                || reason.starts_with("send-fd:")
                || reason == "eof"
    )
}

fn runtime_selector_allows_declared_fallback(error: &MediaOpError) -> bool {
    matches!(
        error,
        MediaOpError::IdentityMismatch(reason) if reason == "runtime-selector"
    )
}

fn resolve_declared_runtime_selector<'a>(
    resolver: &'a BundleResolver,
    vm: &str,
    identity: &UsbPhysicalIdentity,
) -> Result<(MediaRegistryRecord, &'a QemuMediaSourceIntent), MediaOpError> {
    let source = select_unique_declared_physical_source(resolver, vm, identity, None)?;
    Ok((
        MediaRegistryRecord::from_identity(source, identity.clone()),
        source,
    ))
}

fn resolve_declared_attached_runtime_selector<'a>(
    resolver: &'a BundleResolver,
    vm: &str,
    identity: &UsbPhysicalIdentity,
    attached_refs: &BTreeSet<String>,
) -> Result<(MediaRegistryRecord, &'a QemuMediaSourceIntent), MediaOpError> {
    let source =
        select_unique_declared_physical_source(resolver, vm, identity, Some(attached_refs))?;
    Ok((
        MediaRegistryRecord::from_identity(source, identity.clone()),
        source,
    ))
}

fn select_unique_declared_physical_source<'a>(
    resolver: &'a BundleResolver,
    vm: &str,
    identity: &UsbPhysicalIdentity,
    attached_refs: Option<&BTreeSet<String>>,
) -> Result<&'a QemuMediaSourceIntent, MediaOpError> {
    let Some(qemu_media) = resolver.host.qemu_media.as_ref() else {
        return Err(MediaOpError::MissingBundlePolicy);
    };
    select_unique_declared_physical_source_from_sources(
        &qemu_media.sources,
        vm,
        identity,
        attached_refs,
    )
}

fn select_unique_declared_physical_source_from_sources<'a>(
    sources: &'a [QemuMediaSourceIntent],
    vm: &str,
    identity: &UsbPhysicalIdentity,
    attached_refs: Option<&BTreeSet<String>>,
) -> Result<&'a QemuMediaSourceIntent, MediaOpError> {
    let candidates = sources
        .iter()
        .filter(|source| source.vm == vm)
        .filter(|source| matches!(source.source_kind, QemuMediaSourceKind::PhysicalUsb))
        .filter(|source| {
            attached_refs
                .map(|refs| refs.contains(&source.media_ref))
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();
    let selector_matches = candidates
        .iter()
        .copied()
        .filter(|source| {
            source
                .usb_selector
                .as_ref()
                .is_some_and(|selector| usb_selector_matches(selector, identity))
        })
        .collect::<Vec<_>>();
    let matches = if selector_matches.is_empty() {
        candidates
            .into_iter()
            .filter(|source| source.usb_selector.is_none())
            .collect::<Vec<_>>()
    } else {
        selector_matches
    };
    match matches.as_slice() {
        [source] => Ok(*source),
        [] => Err(MediaOpError::IdentityMismatch(
            "runtime-selector".to_owned(),
        )),
        many => {
            let mut refs = many
                .iter()
                .map(|source| source.media_ref.clone())
                .collect::<Vec<_>>();
            refs.sort();
            refs.dedup();
            Err(MediaOpError::AmbiguousRuntimeSelector(refs))
        }
    }
}

#[derive(Debug, Clone)]
struct QmpAttachCleanup {
    device_id: String,
    blockdev_id: String,
    file_node_id: String,
    media_ref: String,
    fdset_id: Option<u64>,
    fd: Option<u64>,
    device_added: bool,
    raw_added: bool,
    file_added: bool,
    fdset_added: bool,
}

impl QmpAttachCleanup {
    fn new(scaffold: &QemuMediaHotplugScaffold, file_node_id: String) -> Self {
        Self {
            device_id: scaffold.device_id.clone(),
            blockdev_id: scaffold.blockdev_id.clone(),
            file_node_id,
            media_ref: scaffold.media_ref.clone(),
            fdset_id: None,
            fd: None,
            device_added: false,
            raw_added: false,
            file_added: false,
            fdset_added: false,
        }
    }

    fn rollback(&self, client: &mut QmpClient) {
        if self.device_added
            && client
                .execute("device_del", json!({ "id": self.device_id.as_str() }), None)
                .is_ok()
        {
            let _ = client.wait_for_device_deleted(&self.device_id);
        }
        if self.raw_added {
            let _ = client.execute(
                "blockdev-del",
                json!({ "node-name": self.blockdev_id.as_str() }),
                None,
            );
        }
        if self.file_added {
            let _ = client.execute(
                "blockdev-del",
                json!({ "node-name": self.file_node_id.as_str() }),
                None,
            );
        }
        if self.fdset_added
            && let Some((fdset_id, fd)) = self
                .fdset_id
                .zip(self.fd)
                .or_else(|| client.query_fdset_entry(&self.media_ref).ok().flatten())
        {
            let _ = client.execute("remove-fd", json!({ "fdset-id": fdset_id, "fd": fd }), None);
        }
    }
}

fn qmp_detach(
    client: &mut QmpClient,
    scaffold: &QemuMediaHotplugScaffold,
) -> Result<Vec<String>, MediaOpError> {
    let file_node_id = qemu_media_file_node_id(&scaffold.media_ref);
    let mut first_error = None;
    let mut commands = Vec::new();
    match client.execute(
        "device_del",
        json!({ "id": scaffold.device_id.as_str() }),
        None,
    ) {
        Ok(_) => {
            commands.push("device_del".to_owned());
            if let Err(error) = client.wait_for_device_deleted(&scaffold.device_id) {
                first_error.get_or_insert(error);
                commands.push("DEVICE_DELETED:reconciled".to_owned());
            } else {
                commands.push("DEVICE_DELETED".to_owned());
            }
        }
        Err(error) => {
            first_error.get_or_insert(error);
            commands.push("device_del:absent".to_owned());
        }
    }
    qmp_delete_block_node(
        client,
        scaffold.blockdev_id.as_str(),
        "blockdev-del:raw",
        &mut commands,
        &mut first_error,
    );
    qmp_delete_block_node(
        client,
        file_node_id.as_str(),
        "blockdev-del:file",
        &mut commands,
        &mut first_error,
    );
    qmp_remove_fdset_entry(
        client,
        scaffold.media_ref.as_str(),
        &mut commands,
        &mut first_error,
    );
    if let Some(error) = first_error {
        let raw_absent = client
            .named_block_node_exists(scaffold.blockdev_id.as_str())
            .map(|exists| !exists)
            .unwrap_or(false);
        let file_absent = client
            .named_block_node_exists(file_node_id.as_str())
            .map(|exists| !exists)
            .unwrap_or(false);
        let fd_absent = client
            .query_fdset_entry(scaffold.media_ref.as_str())
            .map(|entry| entry.is_none())
            .unwrap_or(false);
        if !(raw_absent && file_absent && fd_absent) {
            return Err(error);
        }
    }
    Ok(commands)
}

fn qmp_delete_block_node(
    client: &mut QmpClient,
    node_name: &str,
    label: &str,
    commands: &mut Vec<String>,
    first_error: &mut Option<MediaOpError>,
) {
    match client.execute("blockdev-del", json!({ "node-name": node_name }), None) {
        Ok(_) => commands.push(label.to_owned()),
        Err(error) => match client.named_block_node_exists(node_name) {
            Ok(false) => {
                first_error.get_or_insert(error);
                commands.push(format!("{label}:absent"));
            }
            Ok(true) | Err(_) => {
                first_error.get_or_insert(error);
            }
        },
    }
}

fn qmp_remove_fdset_entry(
    client: &mut QmpClient,
    media_ref: &str,
    commands: &mut Vec<String>,
    first_error: &mut Option<MediaOpError>,
) {
    let fdset_entry = match client.query_fdset_entry(media_ref) {
        Ok(entry) => entry,
        Err(error) => {
            first_error.get_or_insert(error);
            None
        }
    };
    let Some((fdset_id, fd)) = fdset_entry else {
        commands.push("remove-fd:absent".to_owned());
        return;
    };
    match client.execute("remove-fd", json!({ "fdset-id": fdset_id, "fd": fd }), None) {
        Ok(_) => commands.push("remove-fd".to_owned()),
        Err(error) => match client.query_fdset_entry(media_ref) {
            Ok(None) => {
                first_error.get_or_insert(error);
                commands.push("remove-fd:absent".to_owned());
            }
            Ok(Some(_)) | Err(_) => {
                first_error.get_or_insert(error);
            }
        },
    }
}

struct QmpClient {
    vm: String,
    next_id: u64,
    writer: UnixStream,
    reader: BufReader<UnixStream>,
}

impl QmpClient {
    fn connect(path: &Path) -> Result<Self, MediaOpError> {
        Self::connect_with_timeout(path, Duration::from_secs(5))
    }

    fn connect_with_timeout(path: &Path, timeout: Duration) -> Result<Self, MediaOpError> {
        let vm = path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or("unknown")
            .to_owned();
        let writer =
            UnixStream::connect(path).map_err(|err| MediaOpError::Qmp(format!("connect:{err}")))?;
        writer
            .set_read_timeout(Some(timeout))
            .map_err(|err| MediaOpError::Qmp(format!("timeout:{err}")))?;
        writer
            .set_write_timeout(Some(timeout))
            .map_err(|err| MediaOpError::Qmp(format!("timeout:{err}")))?;
        let reader_stream = writer
            .try_clone()
            .map_err(|err| MediaOpError::Qmp(format!("clone:{err}")))?;
        reader_stream
            .set_read_timeout(Some(timeout))
            .map_err(|err| MediaOpError::Qmp(format!("timeout:{err}")))?;
        let mut client = Self {
            vm,
            next_id: 1,
            writer,
            reader: BufReader::new(reader_stream),
        };
        let greeting = client.read_message()?;
        if greeting.get("QMP").is_none() {
            return Err(MediaOpError::Qmp("missing-greeting".to_owned()));
        }
        client.execute("qmp_capabilities", json!({}), None)?;
        Ok(client)
    }

    fn vm(&self) -> &str {
        &self.vm
    }

    fn execute(
        &mut self,
        command: &str,
        arguments: Value,
        fd: Option<i32>,
    ) -> Result<Value, MediaOpError> {
        let id = format!("d2b-{}", self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        let request = json!({
            "execute": command,
            "arguments": arguments,
            "id": id,
        });
        let mut payload = serde_json::to_vec(&request)
            .map_err(|err| MediaOpError::Qmp(format!("encode:{command}:{err}")))?;
        payload.push(b'\n');
        if let Some(fd) = fd {
            crate::fd_passing::send_fds(self.writer.as_raw_fd(), &payload, &[fd])
                .map_err(|err| MediaOpError::Qmp(format!("send-fd:{command}:{err}")))?;
        } else {
            self.writer
                .write_all(&payload)
                .map_err(|err| MediaOpError::Qmp(format!("write:{command}:{err}")))?;
            self.writer
                .flush()
                .map_err(|err| MediaOpError::Qmp(format!("flush:{command}:{err}")))?;
        }
        loop {
            let message = self.read_message()?;
            if message.get("event").is_some() {
                continue;
            }
            if message.get("id").and_then(Value::as_str) != Some(id.as_str()) {
                continue;
            }
            if let Some(error) = message.get("error") {
                let class = error
                    .get("class")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                return Err(MediaOpError::Qmp(format!("{command}:{class}")));
            }
            if let Some(ret) = message.get("return") {
                return Ok(ret.clone());
            }
        }
    }

    fn query_fdset_entry(&mut self, media_ref: &str) -> Result<Option<(u64, u64)>, MediaOpError> {
        let expected_opaque = format!("d2b:{media_ref}");
        let value = self.execute("query-fdsets", json!({}), None)?;
        let Some(fdsets) = value.as_array() else {
            return Ok(None);
        };
        for fdset in fdsets {
            let Some(fdset_id) = fdset.get("fdset-id").and_then(Value::as_u64) else {
                continue;
            };
            let Some(fds) = fdset.get("fds").and_then(Value::as_array) else {
                continue;
            };
            for fd in fds {
                if fd.get("opaque").and_then(Value::as_str) == Some(expected_opaque.as_str()) {
                    return Ok(fd
                        .get("fd")
                        .and_then(Value::as_u64)
                        .map(|fd| (fdset_id, fd)));
                }
            }
        }
        Ok(None)
    }

    fn query_attached_media_refs(&mut self) -> Result<BTreeSet<String>, MediaOpError> {
        let value = self.execute("query-fdsets", json!({}), None)?;
        let mut refs = BTreeSet::new();
        let Some(fdsets) = value.as_array() else {
            return Ok(refs);
        };
        for fdset in fdsets {
            let Some(fds) = fdset.get("fds").and_then(Value::as_array) else {
                continue;
            };
            for fd in fds {
                if let Some(media_ref) = fd
                    .get("opaque")
                    .and_then(Value::as_str)
                    .and_then(|opaque| opaque.strip_prefix("d2b:"))
                    && d2b_host::media::validate_media_ref(media_ref).is_ok()
                {
                    refs.insert(media_ref.to_owned());
                }
            }
        }
        Ok(refs)
    }

    fn named_block_node_exists(&mut self, node_name: &str) -> Result<bool, MediaOpError> {
        let value = self.execute("query-named-block-nodes", json!({}), None)?;
        let Some(nodes) = value.as_array() else {
            return Ok(false);
        };
        Ok(nodes.iter().any(|node| {
            node.get("node-name")
                .and_then(Value::as_str)
                .is_some_and(|name| name == node_name)
        }))
    }

    fn wait_for_device_deleted(&mut self, device_id: &str) -> Result<(), MediaOpError> {
        loop {
            let message = self.read_message()?;
            if message.get("event").and_then(Value::as_str) != Some("DEVICE_DELETED") {
                continue;
            }
            let observed = message
                .get("data")
                .and_then(|data| data.get("device"))
                .and_then(Value::as_str);
            if observed.is_none() || observed == Some(device_id) {
                return Ok(());
            }
        }
    }

    fn read_message(&mut self) -> Result<Value, MediaOpError> {
        let line = self.read_line_bounded()?;
        if line.is_empty() {
            return Err(MediaOpError::Qmp("eof".to_owned()));
        }
        serde_json::from_slice(&line).map_err(|err| MediaOpError::Qmp(format!("decode:{err}")))
    }

    fn read_line_bounded(&mut self) -> Result<Vec<u8>, MediaOpError> {
        let mut line = Vec::new();
        loop {
            let available = self
                .reader
                .fill_buf()
                .map_err(|err| MediaOpError::Qmp(format!("read:{err}")))?;
            if available.is_empty() {
                return Ok(line);
            }
            let take = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(available.len(), |pos| pos + 1);
            if line.len().saturating_add(take) > QMP_MAX_RESPONSE_BYTES {
                return Err(MediaOpError::Qmp("response-too-large".to_owned()));
            }
            line.extend_from_slice(&available[..take]);
            self.reader.consume(take);
            if line.last() == Some(&b'\n') {
                return Ok(line);
            }
        }
    }
}

fn verify_identity_matches_record(
    record: &MediaRegistryRecord,
    identity: &UsbPhysicalIdentity,
) -> Result<(), MediaOpError> {
    if record.identity.devnum != identity.devnum {
        return Err(MediaOpError::IdentityMismatch("devnum".to_owned()));
    }

    if record.identity.vendor_id != identity.vendor_id
        || record.identity.product_id != identity.product_id
    {
        return Err(MediaOpError::IdentityMismatch("vendor-product".to_owned()));
    }
    if record.identity.block_device != identity.block_device {
        return Err(MediaOpError::IdentityMismatch("block-device".to_owned()));
    }
    let expected: BTreeSet<_> = record.identity.by_id_names.iter().collect();
    let actual: BTreeSet<_> = identity.by_id_names.iter().collect();
    if expected != actual {
        return Err(MediaOpError::IdentityMismatch("by-id".to_owned()));
    }
    Ok(())
}

fn read_current_identity_for_record(
    record: &MediaRegistryRecord,
) -> Result<UsbPhysicalIdentity, MediaOpError> {
    let candidates =
        d2b_host::media::safe_usb_block_candidates(Path::new("/sys"), Path::new("/dev/disk/by-id"));
    let matches: Vec<_> = candidates
        .into_iter()
        .filter(|candidate| {
            let expected: BTreeSet<_> = record.identity.by_id_names.iter().collect();
            let actual: BTreeSet<_> = candidate.by_id_names.iter().collect();
            !expected.is_empty() && expected == actual
        })
        .collect();
    match matches.as_slice() {
        [] => Err(MediaOpError::IdentityMismatch(
            "runtime-selector".to_owned(),
        )),
        [candidate] => {
            let identity = read_usb_identity(
                Path::new("/sys"),
                Path::new("/dev/disk/by-id"),
                &candidate.bus_id,
            )?;
            if record.identity.vendor_id != identity.vendor_id
                || record.identity.product_id != identity.product_id
            {
                return Err(MediaOpError::IdentityMismatch("vendor-product".to_owned()));
            }
            let expected: BTreeSet<_> = record.identity.by_id_names.iter().collect();
            let actual: BTreeSet<_> = identity.by_id_names.iter().collect();
            if expected != actual {
                return Err(MediaOpError::IdentityMismatch("by-id".to_owned()));
            }
            Ok(identity)
        }
        many => Err(MediaOpError::AmbiguousRuntimeSelector(
            many.iter()
                .map(|candidate| candidate.bus_id.clone())
                .collect(),
        )),
    }
}

fn read_usb_identity_for_selector(
    selector: &QemuMediaUsbSelector,
) -> Result<UsbPhysicalIdentity, MediaOpError> {
    read_usb_identity_for_selector_at_roots(
        Path::new("/sys"),
        Path::new("/dev/disk/by-id"),
        selector,
    )
}

fn read_usb_identity_for_selector_at_roots(
    sysfs_root: &Path,
    by_id_root: &Path,
    selector: &QemuMediaUsbSelector,
) -> Result<UsbPhysicalIdentity, MediaOpError> {
    let candidates = d2b_host::media::safe_usb_block_candidates(sysfs_root, by_id_root);
    let candidate = select_unique_usb_candidate_for_selector(&candidates, selector)?;
    read_usb_identity(sysfs_root, by_id_root, &candidate.bus_id)
}

fn select_unique_usb_candidate_for_selector<'a>(
    candidates: &'a [SafeUsbCandidate],
    selector: &QemuMediaUsbSelector,
) -> Result<&'a SafeUsbCandidate, MediaOpError> {
    let matches = candidates
        .iter()
        .filter(|candidate| {
            candidate
                .by_id_names
                .iter()
                .any(|name| name == &selector.by_id_name)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Err(MediaOpError::IdentityMismatch(
            "configured-selector".to_owned(),
        )),
        [candidate] => Ok(*candidate),
        many => Err(MediaOpError::AmbiguousRuntimeSelector(
            many.iter()
                .map(|candidate| candidate.bus_id.clone())
                .collect(),
        )),
    }
}

fn usb_selector_matches(selector: &QemuMediaUsbSelector, identity: &UsbPhysicalIdentity) -> bool {
    identity
        .by_id_names
        .iter()
        .any(|name| name == &selector.by_id_name)
}

fn runtime_identity_matches_record(
    record: &MediaRegistryRecord,
    identity: &UsbPhysicalIdentity,
) -> bool {
    if record.vm.is_empty() || record.media_ref.is_empty() {
        return false;
    }
    if record.identity.vendor_id != identity.vendor_id
        || record.identity.product_id != identity.product_id
    {
        return false;
    }
    let expected: BTreeSet<_> = record.identity.by_id_names.iter().collect();
    let actual: BTreeSet<_> = identity.by_id_names.iter().collect();
    !expected.is_empty() && expected == actual
}

fn resolve_runtime_selector<'a>(
    resolver: &'a BundleResolver,
    vm: &str,
    identity: &UsbPhysicalIdentity,
) -> Result<(MediaRegistryRecord, &'a QemuMediaSourceIntent), MediaOpError> {
    let records = read_all_registry_records(resolver)?;
    let record = select_unique_runtime_record(&records, vm, identity)?;
    let source = resolve_physical_source(resolver, vm, &record.media_ref)?;
    Ok((record, source))
}

fn resolve_detach_runtime_selector<'a>(
    resolver: &'a BundleResolver,
    vm: &str,
    identity: &UsbPhysicalIdentity,
    client: &mut QmpClient,
) -> Result<(MediaRegistryRecord, &'a QemuMediaSourceIntent), MediaOpError> {
    let records = read_all_registry_records(resolver)?;
    match select_unique_runtime_record(&records, vm, identity) {
        Ok(record) => {
            let source = resolve_physical_source(resolver, vm, &record.media_ref)?;
            Ok((record, source))
        }
        Err(error) if runtime_selector_allows_detach_fallback(&error) => {
            let attached_refs = client.query_attached_media_refs()?;
            match select_unique_attached_detach_record(&records, vm, identity, &attached_refs) {
                Ok(record) => {
                    let source = resolve_physical_source(resolver, vm, &record.media_ref)?;
                    Ok((record, source))
                }
                Err(fallback_error)
                    if runtime_selector_allows_declared_fallback(&fallback_error) =>
                {
                    resolve_declared_attached_runtime_selector(
                        resolver,
                        vm,
                        identity,
                        &attached_refs,
                    )
                }
                Err(fallback_error) => Err(fallback_error),
            }
        }
        Err(error) => Err(error),
    }
}

fn runtime_selector_allows_detach_fallback(error: &MediaOpError) -> bool {
    matches!(
        error,
        MediaOpError::IdentityMismatch(reason) if reason == "runtime-selector"
    )
}

fn select_unique_runtime_record(
    records: &[MediaRegistryRecord],
    vm: &str,
    identity: &UsbPhysicalIdentity,
) -> Result<MediaRegistryRecord, MediaOpError> {
    let matches: Vec<_> = records
        .iter()
        .filter(|record| record.vm == vm && runtime_identity_matches_record(record, identity))
        .cloned()
        .collect();
    match matches.as_slice() {
        [] => Err(MediaOpError::IdentityMismatch(
            "runtime-selector".to_owned(),
        )),
        [record] => Ok(record.clone()),
        many => {
            let mut refs: Vec<_> = many.iter().map(|record| record.media_ref.clone()).collect();
            refs.sort();
            refs.dedup();
            Err(MediaOpError::AmbiguousRuntimeSelector(refs))
        }
    }
}

fn select_unique_attached_detach_record(
    records: &[MediaRegistryRecord],
    vm: &str,
    identity: &UsbPhysicalIdentity,
    attached_refs: &BTreeSet<String>,
) -> Result<MediaRegistryRecord, MediaOpError> {
    let matches: Vec<_> = records
        .iter()
        .filter(|record| {
            record.vm == vm
                && attached_refs.contains(&record.media_ref)
                && record.identity.vendor_id == identity.vendor_id
                && record.identity.product_id == identity.product_id
        })
        .cloned()
        .collect();
    match matches.as_slice() {
        [record] => Ok(record.clone()),
        [] => Err(MediaOpError::IdentityMismatch(
            "runtime-selector".to_owned(),
        )),
        many => {
            let mut refs: Vec<_> = many.iter().map(|record| record.media_ref.clone()).collect();
            refs.sort();
            refs.dedup();
            Err(MediaOpError::AmbiguousRuntimeSelector(refs))
        }
    }
}

fn resolve_physical_source<'a>(
    resolver: &'a BundleResolver,
    vm: &str,
    media_ref: &str,
) -> Result<&'a QemuMediaSourceIntent, MediaOpError> {
    let source = resolve_declared_source(resolver, vm, media_ref)?;
    if !matches!(source.source_kind, QemuMediaSourceKind::PhysicalUsb) {
        return Err(MediaOpError::UnsupportedSourceKind);
    }
    Ok(source)
}

fn resolve_declared_source<'a>(
    resolver: &'a BundleResolver,
    vm: &str,
    media_ref: &str,
) -> Result<&'a QemuMediaSourceIntent, MediaOpError> {
    resolver
        .find_qemu_media_source(vm, media_ref)
        .ok_or(MediaOpError::MissingBundlePolicy)
}

fn access_mode(source: &QemuMediaSourceIntent) -> MediaAccessMode {
    if source.read_only {
        MediaAccessMode::ReadOnly
    } else {
        MediaAccessMode::ReadWrite
    }
}

fn read_usb_identity(
    sysfs_root: &Path,
    by_id_root: &Path,
    bus_id: &str,
) -> Result<UsbPhysicalIdentity, MediaOpError> {
    let usb_dir = d2b_host::media::sysfs_usb_device_dir(sysfs_root, bus_id);
    let devnum = read_trimmed(&usb_dir.join("devnum"))
        .map_err(|err| MediaOpError::Sysfs(format!("devnum:{err}")))
        .and_then(|value| {
            d2b_host::media::parse_devnum(&value)
                .map_err(|err| MediaOpError::Sysfs(format!("devnum:{err:?}")))
        })?;
    let vendor_id = read_trimmed(&usb_dir.join("idVendor"))
        .map_err(|err| MediaOpError::Sysfs(err.to_string()))?;
    let product_id = read_trimmed(&usb_dir.join("idProduct"))
        .map_err(|err| MediaOpError::Sysfs(err.to_string()))?;
    let mut block_devices = BTreeSet::new();
    collect_block_devices_under(&usb_dir, 0, &mut block_devices);
    if let Some(parent) = usb_dir.parent()
        && let Ok(entries) = std::fs::read_dir(parent)
    {
        let prefix = format!("{bus_id}:");
        for entry in entries.flatten() {
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(&prefix))
            {
                collect_block_devices_under(&entry.path(), 0, &mut block_devices);
            }
        }
    }
    let block_devices: Vec<String> = block_devices.into_iter().collect();
    let block_device = match block_devices.as_slice() {
        [] => return Err(MediaOpError::NoBlockDevice),
        [one] => one.clone(),
        many => return Err(MediaOpError::AmbiguousBlockDevice(many.to_vec())),
    };
    let by_id_names = by_id_names_for_block(by_id_root, &block_device)?;
    if by_id_names.is_empty() {
        return Err(MediaOpError::MissingById);
    }
    Ok(UsbPhysicalIdentity {
        bus_id: bus_id.to_owned(),
        devnum,
        vendor_id,
        product_id,
        by_id_names,
        block_device,
    })
}

fn read_trimmed(path: &Path) -> io::Result<String> {
    std::fs::read_to_string(path).map(|value| value.trim().to_owned())
}

fn collect_block_devices_under(path: &Path, depth: u8, out: &mut BTreeSet<String>) {
    if depth > 12 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        if entry.file_name() == "block" {
            if let Ok(blocks) = std::fs::read_dir(entry.path()) {
                for block in blocks.flatten() {
                    if let Some(name) = block.file_name().to_str()
                        && !name.is_empty()
                    {
                        out.insert(name.to_owned());
                    }
                }
            }
            continue;
        }
        collect_block_devices_under(&entry.path(), depth + 1, out);
    }
}

fn by_id_names_for_block(
    by_id_root: &Path,
    block_device: &str,
) -> Result<Vec<String>, MediaOpError> {
    let expected = PathBuf::from("/dev").join(block_device);
    let mut names = Vec::new();
    let entries = std::fs::read_dir(by_id_root).map_err(|err| {
        if err.kind() == io::ErrorKind::NotFound {
            MediaOpError::MissingById
        } else {
            MediaOpError::Io(format!("by-id-read-dir:{err}"))
        }
    })?;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(target) = std::fs::canonicalize(&path) else {
            continue;
        };
        if target == expected
            && let Ok(name) = d2b_host::media::by_id_name_from_path(&path)
        {
            names.push(name);
        }
    }
    names.sort();
    names.dedup();
    Ok(names)
}

fn preflight_identity_not_busy(
    sysfs_root: &Path,
    identity: &UsbPhysicalIdentity,
) -> Result<(), MediaOpError> {
    let mounts =
        std::fs::read_to_string("/proc/mounts").map_err(|err| MediaOpError::Io(err.to_string()))?;
    let swaps =
        std::fs::read_to_string("/proc/swaps").map_err(|err| MediaOpError::Io(err.to_string()))?;
    let holders = holder_names_for_block_and_children(sysfs_root, &identity.block_device);
    let report = d2b_host::media::preflight_device_not_in_use(
        &identity.block_device,
        &mounts,
        &swaps,
        &holders,
    );
    if report.is_clear() {
        return Ok(());
    }
    Err(MediaOpError::DeviceBusy(format!(
        "uses={}",
        report.uses.len()
    )))
}

fn holder_names_for_block_and_children(sysfs_root: &Path, block_device: &str) -> Vec<String> {
    let block_dir = d2b_host::media::sysfs_block_device_dir(sysfs_root, block_device);
    let mut holders = Vec::new();
    append_holder_names(&block_dir.join("holders"), block_device, &mut holders);
    if let Ok(entries) = std::fs::read_dir(&block_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !is_child_block_name(block_device, &name) {
                continue;
            }
            append_holder_names(&entry.path().join("holders"), &name, &mut holders);
        }
    }
    holders.sort();
    holders.dedup();
    holders
}

fn append_holder_names(holder_dir: &Path, block_name: &str, holders: &mut Vec<String>) {
    let Some(entries) = std::fs::read_dir(holder_dir).ok() else {
        return;
    };
    for entry in entries.flatten() {
        if let Some(holder) = entry.file_name().to_str()
            && !holder.is_empty()
        {
            holders.push(format!("{block_name}:{holder}"));
        }
    }
}

fn is_child_block_name(block_device: &str, candidate: &str) -> bool {
    let Some(rest) = candidate.strip_prefix(block_device) else {
        return false;
    };
    rest.as_bytes()
        .first()
        .is_some_and(|b| b.is_ascii_digit() || *b == b'p')
}

fn open_block_device(block_device: &str, access: MediaAccessMode) -> Result<OwnedFd, MediaOpError> {
    use rustix::fs::{FileType, Mode, OFlags, fstat, open};

    let path = PathBuf::from("/dev").join(block_device);
    let flags = match access {
        MediaAccessMode::ReadOnly => {
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::EXCL
        }
        MediaAccessMode::ReadWrite => {
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::EXCL
        }
    };
    let fd =
        open(&path, flags, Mode::empty()).map_err(|err| MediaOpError::Open(err.to_string()))?;
    let stat = fstat(&fd).map_err(|err| MediaOpError::Open(err.to_string()))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::BlockDevice {
        return Err(MediaOpError::Open("not-block-device".to_owned()));
    }
    Ok(fd)
}

fn open_image_file(
    source: &QemuMediaSourceIntent,
    access: MediaAccessMode,
) -> Result<OwnedFd, MediaOpError> {
    use rustix::fs::{FileType, OFlags};

    if source.format != QemuMediaFormat::Raw {
        return Err(MediaOpError::UnsupportedImageFormat);
    }
    let path = source
        .image_path
        .as_deref()
        .ok_or(MediaOpError::MissingImagePath)?;
    let path = Path::new(path);
    validate_image_path_shape(path)?;
    validate_image_parent_dirs(path)?;

    let parent = path
        .parent()
        .ok_or_else(|| MediaOpError::ImagePathUnsafe("missing-parent".to_owned()))?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| MediaOpError::ImagePathUnsafe("missing-basename".to_owned()))?;
    let parent_fd = crate::sys::path_safe::open_dir_path_safe(parent)
        .map_err(|err| MediaOpError::ImagePathUnsafe(err.to_string()))?;
    let flags = match access {
        MediaAccessMode::ReadOnly => OFlags::RDONLY,
        MediaAccessMode::ReadWrite => OFlags::RDWR,
    };
    let fd = crate::sys::path_safe::open_at(parent_fd.as_fd(), Path::new(name), flags)
        .map_err(|err| MediaOpError::Open(err.to_string()))?;
    let stat = crate::sys::path_safe::fstat_fd(fd.as_fd())
        .map_err(|err| MediaOpError::Open(err.to_string()))?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile {
        return Err(MediaOpError::Open("not-regular-file".to_owned()));
    }
    validate_image_file_metadata(&stat)?;
    preflight_image_not_in_use(Path::new("/sys"), path)?;
    lock_image_fd(&fd, access)?;
    Ok(fd)
}

fn validate_image_path_shape(path: &Path) -> Result<(), MediaOpError> {
    if !path.is_absolute() {
        return Err(MediaOpError::ImagePathUnsafe(
            "path-not-absolute".to_owned(),
        ));
    }
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(MediaOpError::ImagePathUnsafe(
            "parent-dir-component".to_owned(),
        ));
    }
    let text = path.as_os_str().to_string_lossy();
    if text.contains('\n') || text.contains('\r') {
        return Err(MediaOpError::ImagePathUnsafe(
            "path-contains-newline".to_owned(),
        ));
    }
    Ok(())
}

fn validate_image_parent_dirs(path: &Path) -> Result<(), MediaOpError> {
    let parent = path
        .parent()
        .ok_or_else(|| MediaOpError::ImagePathUnsafe("missing-parent".to_owned()))?;
    let mut current = PathBuf::from("/");
    for component in parent.components() {
        use std::path::Component;
        let Component::Normal(name) = component else {
            continue;
        };
        current.push(name);
        let metadata = std::fs::symlink_metadata(&current)
            .map_err(|err| MediaOpError::ImagePathUnsafe(err.to_string()))?;
        if metadata.file_type().is_symlink() {
            return Err(MediaOpError::ImagePathUnsafe("symlink-parent".to_owned()));
        }
        if !metadata.file_type().is_dir() {
            return Err(MediaOpError::ImagePathUnsafe(
                "non-directory-parent".to_owned(),
            ));
        }
        if metadata.uid() != 0 {
            return Err(MediaOpError::ImagePathUnsafe(
                "parent-not-root-owned".to_owned(),
            ));
        }
        let mode = metadata.permissions().mode();
        if mode & 0o002 != 0 {
            return Err(MediaOpError::ImagePathUnsafe(
                "parent-world-writable".to_owned(),
            ));
        }
        if mode & 0o020 != 0 && mode & 0o1000 == 0 {
            return Err(MediaOpError::ImagePathUnsafe(
                "parent-group-writable-without-sticky".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_image_file_metadata(stat: &libc::stat) -> Result<(), MediaOpError> {
    if stat.st_uid != 0 {
        return Err(MediaOpError::ImagePathUnsafe(format!(
            "file-not-root-owned:uid={}",
            stat.st_uid
        )));
    }
    if stat.st_mode & 0o022 != 0 {
        return Err(MediaOpError::ImagePathUnsafe(format!(
            "file-writable-by-group-or-other:mode={:#o}",
            stat.st_mode & 0o7777
        )));
    }
    Ok(())
}

fn preflight_image_not_in_use(sysfs_root: &Path, image_path: &Path) -> Result<(), MediaOpError> {
    let mounts =
        std::fs::read_to_string("/proc/mounts").map_err(|err| MediaOpError::Io(err.to_string()))?;
    if image_path_mounted_in_proc_mounts(&mounts, image_path) {
        return Err(MediaOpError::ImageBusy("mounted".to_owned()));
    }
    if image_has_loop_backing(sysfs_root, image_path)? {
        return Err(MediaOpError::ImageBusy("loop-backed".to_owned()));
    }
    Ok(())
}

fn lock_image_fd(fd: &OwnedFd, access: MediaAccessMode) -> Result<(), MediaOpError> {
    let mode = match access {
        MediaAccessMode::ReadOnly => nix::fcntl::FlockArg::LockSharedNonblock,
        MediaAccessMode::ReadWrite => nix::fcntl::FlockArg::LockExclusiveNonblock,
    };
    nix::fcntl::flock(fd.as_raw_fd(), mode)
        .map_err(|err| MediaOpError::ImageBusy(format!("lock:{err}")))
}

fn image_path_mounted_in_proc_mounts(mounts: &str, image_path: &Path) -> bool {
    let image_path = image_path.as_os_str().to_string_lossy();
    mounts.lines().any(|line| {
        line.split_whitespace()
            .next()
            .map(unescape_proc_mount_field)
            .is_some_and(|source| source == image_path.as_ref())
    })
}

fn unescape_proc_mount_field(value: &str) -> String {
    value
        .replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

fn image_has_loop_backing(sysfs_root: &Path, image_path: &Path) -> Result<bool, MediaOpError> {
    let block_root = sysfs_root.join("block");
    let entries = std::fs::read_dir(&block_root)
        .map_err(|err| MediaOpError::Sysfs(format!("loop-scan:{err}")))?;
    let image_path = image_path.as_os_str().to_string_lossy();
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if !name.starts_with("loop") {
            continue;
        }
        let backing_path = entry.path().join("loop/backing_file");
        match std::fs::read_to_string(&backing_path) {
            Ok(contents) if contents.trim() == image_path.as_ref() => return Ok(true),
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(MediaOpError::Sysfs(format!("loop-backing-file:{err}")));
            }
        }
    }
    Ok(false)
}

fn registry_dir(resolver: &BundleResolver) -> Result<PathBuf, MediaOpError> {
    resolver
        .host
        .qemu_media
        .as_ref()
        .map(|media| PathBuf::from(&media.registry_dir))
        .ok_or(MediaOpError::MissingBundlePolicy)
}

fn rules_path(resolver: &BundleResolver) -> Result<PathBuf, MediaOpError> {
    resolver
        .host
        .qemu_media
        .as_ref()
        .map(|media| PathBuf::from(&media.runtime_rules_path))
        .ok_or(MediaOpError::MissingBundlePolicy)
}

fn write_registry_record(
    resolver: &BundleResolver,
    record: &MediaRegistryRecord,
) -> Result<(), MediaOpError> {
    let root = registry_dir(resolver)?;
    write_registry_record_at_root(&root, record, 0, 0)
}

fn write_registry_record_at_root(
    root: &Path,
    record: &MediaRegistryRecord,
    owner_uid: u32,
    owner_gid: u32,
) -> Result<(), MediaOpError> {
    std::fs::create_dir_all(root).map_err(|err| MediaOpError::Registry(err.to_string()))?;
    let root_owner = if Uid::effective().is_root() {
        Some(0)
    } else {
        None
    };
    crate::sys::path_safe::ensure_dir(root, 0o700, root_owner, root_owner)
        .map_err(|err| MediaOpError::Registry(err.to_string()))?;
    let root_fd = crate::sys::path_safe::open_dir_path_safe(&root)
        .map_err(|err| MediaOpError::Registry(err.to_string()))?;
    let vm_fd = crate::sys::path_safe::ensure_dir_path_safe(
        &root_fd, &record.vm, 0o700, owner_uid, owner_gid,
    )
    .map_err(|err| MediaOpError::Registry(err.to_string()))?;
    let mut bytes =
        serde_json::to_vec_pretty(record).map_err(|err| MediaOpError::Registry(err.to_string()))?;
    bytes.push(b'\n');
    let record_owner = if Uid::effective().is_root() {
        (Some(owner_uid), Some(owner_gid))
    } else {
        (None, None)
    };
    crate::sys::path_safe::atomic_replace_fd_with_owner(
        &vm_fd,
        &format!("{}.json", record.media_ref),
        &bytes,
        0o600,
        record_owner.0,
        record_owner.1,
    )
    .map_err(|err| MediaOpError::Registry(err.to_string()))
}

fn write_declared_selector_artifacts(
    registry_root: &Path,
    redacted_index_path: &Path,
    rules_path: &Path,
    source: &QemuMediaSourceIntent,
    identity: &UsbPhysicalIdentity,
    owner_uid: u32,
    owner_gid: u32,
    reload_rules: bool,
) -> Result<BootSourceAudit, MediaOpError> {
    let record = MediaRegistryRecord::from_identity(source, identity.clone());
    let mut audit = BootSourceAudit::default();
    write_registry_record_at_root(registry_root, &record, owner_uid, owner_gid)?;
    audit.registry_record_written = true;
    let records = read_all_registry_records_at_root(registry_root).unwrap_or_else(|_| vec![record]);
    write_redacted_registry_index_at_path(redacted_index_path, &records)?;
    audit.redacted_index_written = true;
    audit.udev_rule_written = write_runtime_udev_rules_at_path(rules_path, &records)?;
    audit.udev_reloaded = reload_rules && reload_udev_rules();
    Ok(audit)
}

fn redacted_index_path(resolver: &BundleResolver) -> Result<PathBuf, MediaOpError> {
    resolver
        .find_storage_path_spec(REDACTED_INDEX_STORAGE_REF)
        .map(|spec| PathBuf::from(spec.path_template.as_str()))
        .ok_or_else(|| MediaOpError::Registry("redacted-index-storage-ref-missing".to_owned()))
}

fn write_redacted_registry_index(
    resolver: &BundleResolver,
    records: &[MediaRegistryRecord],
) -> Result<(), MediaOpError> {
    let path = redacted_index_path(resolver)?;
    write_redacted_registry_index_at_path(&path, records)
}

fn write_redacted_registry_index_at_path(
    path: &Path,
    records: &[MediaRegistryRecord],
) -> Result<(), MediaOpError> {
    let Some(parent) = path.parent() else {
        return Err(MediaOpError::Registry(
            "redacted-index-no-parent".to_owned(),
        ));
    };
    std::fs::create_dir_all(parent).map_err(|err| MediaOpError::Registry(err.to_string()))?;
    let parent_fd = crate::sys::path_safe::open_dir_path_safe(parent)
        .map_err(|err| MediaOpError::Registry(err.to_string()))?;
    let index = RedactedRegistryIndex {
        schema_version: 1,
        records: records.iter().map(MediaRegistryRecord::redacted).collect(),
    };
    let mut bytes =
        serde_json::to_vec_pretty(&index).map_err(|err| MediaOpError::Registry(err.to_string()))?;
    bytes.push(b'\n');
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| MediaOpError::Registry("redacted-index-name-invalid".to_owned()))?;
    let owner_gid = if Uid::effective().is_root() {
        let gid = Group::from_name("d2bd")
            .map_err(|err| MediaOpError::Registry(format!("resolve d2bd group: {err}")))?
            .map(|group| group.gid)
            .ok_or_else(|| MediaOpError::Registry("d2bd group missing".to_owned()))?;
        Some(gid.as_raw())
    } else {
        None
    };
    crate::sys::path_safe::atomic_replace_fd_with_owner(
        &parent_fd, name, &bytes, 0o640, None, owner_gid,
    )
    .map_err(|err| MediaOpError::Registry(err.to_string()))
}

fn qemu_media_identity_hash(by_id_names: &[String]) -> String {
    let mut names = by_id_names.to_vec();
    names.sort();
    names.dedup();
    let mut hasher = Sha256::new();
    for name in names {
        hasher.update(name.as_bytes());
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}

fn read_registry_record(
    resolver: &BundleResolver,
    vm: &str,
    media_ref: &str,
) -> Result<MediaRegistryRecord, MediaOpError> {
    let path = registry_dir(resolver)?
        .join(vm)
        .join(format!("{media_ref}.json"));
    let bytes = std::fs::read(&path).map_err(|err| MediaOpError::Registry(err.to_string()))?;
    serde_json::from_slice(&bytes).map_err(|err| MediaOpError::Registry(err.to_string()))
}

fn read_all_registry_records(
    resolver: &BundleResolver,
) -> Result<Vec<MediaRegistryRecord>, MediaOpError> {
    let root = registry_dir(resolver)?;
    read_all_registry_records_at_root(&root)
}

fn read_all_registry_records_at_root(
    root: &Path,
) -> Result<Vec<MediaRegistryRecord>, MediaOpError> {
    let mut records = Vec::new();
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(records),
        Err(err) => return Err(MediaOpError::Registry(err.to_string())),
    };
    for vm_entry in entries.flatten() {
        let Ok(file_type) = vm_entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(vm_entry.path()) else {
            continue;
        };
        for file in files.flatten() {
            if file.path().extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            if let Ok(bytes) = std::fs::read(file.path())
                && let Ok(record) = serde_json::from_slice::<MediaRegistryRecord>(&bytes)
            {
                records.push(record);
            }
        }
    }
    Ok(records)
}

fn write_runtime_udev_rules(
    resolver: &BundleResolver,
    records: &[MediaRegistryRecord],
) -> Result<bool, MediaOpError> {
    let path = rules_path(resolver)?;
    write_runtime_udev_rules_at_path(&path, records)
}

fn write_runtime_udev_rules_at_path(
    path: &Path,
    records: &[MediaRegistryRecord],
) -> Result<bool, MediaOpError> {
    let Some(parent) = path.parent() else {
        return Err(MediaOpError::Registry(
            "udev-rule-path-has-no-parent".to_owned(),
        ));
    };
    std::fs::create_dir_all(parent).map_err(|err| MediaOpError::Registry(err.to_string()))?;
    let parent_fd = crate::sys::path_safe::open_dir_path_safe(parent)
        .map_err(|err| MediaOpError::Registry(err.to_string()))?;
    let text = render_runtime_udev_rules(records);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| MediaOpError::Registry("udev-rule-name-invalid".to_owned()))?;
    crate::sys::path_safe::atomic_replace_fd(&parent_fd, name, text.as_bytes(), 0o600)
        .map_err(|err| MediaOpError::Registry(err.to_string()))?;
    Ok(true)
}

fn render_runtime_udev_rules(records: &[MediaRegistryRecord]) -> String {
    let mut text = String::from(
        "# d2b qemu-media physical USB ignore rules\n\
         # root-only runtime artifact; public bundle contains opaque refs only\n",
    );
    for record in records {
        for by_id in &record.identity.by_id_names {
            text.push_str(&format!(
                "SUBSYSTEM==\"block\", ENV{{DEVLINKS}}==\"*/dev/disk/by-id/{}*\", ENV{{UDISKS_IGNORE}}=\"1\"\n",
                escape_udev_value(by_id)
            ));
        }
    }
    text
}

fn escape_udev_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn reload_udev_rules() -> bool {
    Command::new("udevadm")
        .arg("control")
        .arg("--reload-rules")
        .env_remove("NOTIFY_SOCKET")
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_core::host::QemuMediaRegistryScope;
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixListener;

    fn registry_record() -> MediaRegistryRecord {
        MediaRegistryRecord {
            schema_version: 1,
            vm: "media".to_owned(),
            media_ref: "installer-usb".to_owned(),
            source_kind: "physical-usb".to_owned(),
            format: "raw".to_owned(),
            read_only: true,
            identity: RegistryIdentity {
                bus_id: "1-2.3".to_owned(),
                devnum: 7,
                vendor_id: "abcd".to_owned(),
                product_id: "1234".to_owned(),
                by_id_names: vec!["usb-Vendor_SecretSerial".to_owned()],
                block_device: "sdb".to_owned(),
            },
        }
    }

    fn physical_identity() -> UsbPhysicalIdentity {
        UsbPhysicalIdentity {
            bus_id: "1-2.3".to_owned(),
            devnum: 7,
            vendor_id: "abcd".to_owned(),
            product_id: "1234".to_owned(),
            by_id_names: vec!["usb-Vendor_SecretSerial".to_owned()],
            block_device: "sdb".to_owned(),
        }
    }

    fn declared_source(media_ref: &str, selector: Option<&str>) -> QemuMediaSourceIntent {
        QemuMediaSourceIntent {
            vm: "media".to_owned(),
            media_ref: media_ref.to_owned(),
            slot: if media_ref == "installer-usb" {
                "boot".to_owned()
            } else {
                media_ref.to_owned()
            },
            source_kind: QemuMediaSourceKind::PhysicalUsb,
            format: QemuMediaFormat::Raw,
            read_only: true,
            registry_scope: QemuMediaRegistryScope::RootOnlyRuntimeState,
            image_path: None,
            usb_selector: selector.map(|by_id_name| QemuMediaUsbSelector {
                by_id_name: by_id_name.to_owned(),
            }),
        }
    }

    fn candidate(bus_id: &str, block_device: &str, by_id_names: &[&str]) -> SafeUsbCandidate {
        SafeUsbCandidate {
            bus_id: bus_id.to_owned(),
            block_device: block_device.to_owned(),
            by_id_names: by_id_names.iter().map(|name| (*name).to_owned()).collect(),
        }
    }

    fn write_usb_candidate_fixture(
        sysfs_root: &Path,
        by_id_root: &Path,
        bus_id: &str,
        block_device: &str,
        by_id_name: &str,
    ) {
        let usb_dir = sysfs_root.join("bus/usb/devices").join(bus_id);
        std::fs::create_dir_all(usb_dir.join("block").join(block_device)).expect("usb block dir");
        std::fs::write(usb_dir.join("devnum"), b"7\n").expect("devnum");
        std::fs::write(usb_dir.join("idVendor"), b"abcd\n").expect("idVendor");
        std::fs::write(usb_dir.join("idProduct"), b"1234\n").expect("idProduct");
        std::fs::create_dir_all(by_id_root).expect("by-id dir");
        std::os::unix::fs::symlink(
            Path::new("/dev").join(block_device),
            by_id_root.join(by_id_name),
        )
        .expect("by-id symlink");
    }

    fn qmp_tempdir() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("qmp-")
            .tempdir()
            .expect("qmp tempdir")
    }

    #[test]
    fn qmp_attach_sends_fd_and_device_commands() {
        let dir = qmp_tempdir();
        let socket = dir.path().join("qmp.sock");
        let listener = UnixListener::bind(&socket).expect("bind qmp");
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept qmp");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("server read timeout");
            let mut writer = stream.try_clone().expect("clone qmp writer");
            let mut reader = BufReader::new(stream.try_clone().expect("clone qmp reader"));
            write_qmp_greeting_and_capabilities(&mut writer, &mut reader);

            let (payload, fds) = crate::fd_passing::recv_fds(stream.as_raw_fd())
                .expect("add-fd must carry SCM_RIGHTS");
            assert_eq!(fds.len(), 1);
            let add_fd = String::from_utf8(payload).expect("add-fd utf8");
            assert!(add_fd.contains(r#""execute":"add-fd""#));
            let add_fd_id = qmp_id_from_line(&add_fd);
            writer
                .write_all(
                    format!(r#"{{"return":{{"fdset-id":1000,"fd":0}},"id":"{add_fd_id}"}}"#)
                        .as_bytes(),
                )
                .expect("add-fd return");
            writer.write_all(b"\n").expect("add-fd newline");

            for expected in [
                r#""driver":"host_device""#,
                r#""driver":"raw""#,
                "device_add",
            ] {
                expect_qmp_command(&mut writer, &mut reader, expected);
            }
        });

        let mut client = QmpClient::connect(&socket).expect("connect fake qmp");
        let file = std::fs::File::open("/dev/null").expect("open harmless fd");
        let scaffold = qmp_scaffold("installer-usb", "boot", QemuMediaHotplugAction::Attach)
            .expect("scaffold");
        let commands = qmp_attach(
            &mut client,
            &scaffold,
            file.as_raw_fd(),
            QemuMediaSourceKind::PhysicalUsb,
            true,
        )
        .expect("qmp attach");
        assert_eq!(
            commands,
            [
                "add-fd",
                "blockdev-add:host_device",
                "blockdev-add:raw",
                "device_add"
            ]
        );
        server.join().expect("fake qmp server joins");
    }

    #[test]
    fn qmp_detach_removes_device_blocks_and_fdset() {
        let dir = qmp_tempdir();
        let socket = dir.path().join("qmp.sock");
        let listener = UnixListener::bind(&socket).expect("bind qmp");
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept qmp");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("server read timeout");
            let mut writer = stream.try_clone().expect("clone qmp writer");
            let mut reader = BufReader::new(stream.try_clone().expect("clone qmp reader"));
            write_qmp_greeting_and_capabilities(&mut writer, &mut reader);

            expect_qmp_command(&mut writer, &mut reader, "device_del");
            writer
                .write_all(
                    br#"{"event":"DEVICE_DELETED","data":{"device":"d2b-usb-installer-usb"}}"#,
                )
                .expect("device deleted event");
            writer.write_all(b"\n").expect("event newline");
            for expected in ["blockdev-del", "blockdev-del"] {
                expect_qmp_command(&mut writer, &mut reader, expected);
            }
            expect_qmp_query_fdsets(&mut writer, &mut reader);
            expect_qmp_command(&mut writer, &mut reader, "remove-fd");
        });

        let mut client = QmpClient::connect(&socket).expect("connect fake qmp");
        let scaffold = qmp_scaffold("installer-usb", "boot", QemuMediaHotplugAction::Detach)
            .expect("scaffold");
        let commands = qmp_detach(&mut client, &scaffold).expect("qmp detach");
        assert_eq!(
            commands,
            [
                "device_del",
                "DEVICE_DELETED",
                "blockdev-del:raw",
                "blockdev-del:file",
                "remove-fd"
            ]
        );
        server.join().expect("fake qmp server joins");
    }

    #[test]
    fn qmp_detach_reconciles_missed_device_deleted_event() {
        let dir = qmp_tempdir();
        let socket = dir.path().join("qmp.sock");
        let listener = UnixListener::bind(&socket).expect("bind qmp");
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept qmp");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("server read timeout");
            let mut writer = stream.try_clone().expect("clone qmp writer");
            let mut reader = BufReader::new(stream.try_clone().expect("clone qmp reader"));
            write_qmp_greeting_and_capabilities(&mut writer, &mut reader);

            expect_qmp_command(&mut writer, &mut reader, "device_del");
            for expected in ["blockdev-del", "blockdev-del"] {
                expect_qmp_command(&mut writer, &mut reader, expected);
            }
            expect_qmp_query_fdsets_with(
                &mut writer,
                &mut reader,
                r#"[{"fdset-id":1000,"fds":[{"fd":0,"opaque":"d2b:installer-usb"}]}]"#,
            );
            expect_qmp_command(&mut writer, &mut reader, "remove-fd");
            expect_qmp_query_named_block_nodes(&mut writer, &mut reader, "[]");
            expect_qmp_query_named_block_nodes(&mut writer, &mut reader, "[]");
            expect_qmp_query_fdsets_with(&mut writer, &mut reader, "[]");
        });

        let mut client = QmpClient::connect_with_timeout(&socket, Duration::from_millis(50))
            .expect("connect fake qmp");
        let scaffold = qmp_scaffold("installer-usb", "boot", QemuMediaHotplugAction::Detach)
            .expect("scaffold");
        let commands = qmp_detach(&mut client, &scaffold).expect("qmp detach reconciles");
        assert_eq!(
            commands,
            [
                "device_del",
                "DEVICE_DELETED:reconciled",
                "blockdev-del:raw",
                "blockdev-del:file",
                "remove-fd"
            ]
        );
        server.join().expect("fake qmp server joins");
    }

    #[test]
    fn qmp_detach_is_idempotent_when_media_nodes_are_already_absent() {
        let dir = qmp_tempdir();
        let socket = dir.path().join("qmp.sock");
        let listener = UnixListener::bind(&socket).expect("bind qmp");
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept qmp");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("server read timeout");
            let mut writer = stream.try_clone().expect("clone qmp writer");
            let mut reader = BufReader::new(stream.try_clone().expect("clone qmp reader"));
            write_qmp_greeting_and_capabilities(&mut writer, &mut reader);

            expect_qmp_command_error(&mut writer, &mut reader, "device_del", "DeviceNotFound");
            for expected in ["blockdev-del", "blockdev-del"] {
                expect_qmp_command_error(&mut writer, &mut reader, expected, "GenericError");
                expect_qmp_query_named_block_nodes(&mut writer, &mut reader, "[]");
            }
            expect_qmp_query_fdsets_with(&mut writer, &mut reader, "[]");
            expect_qmp_query_named_block_nodes(&mut writer, &mut reader, "[]");
            expect_qmp_query_named_block_nodes(&mut writer, &mut reader, "[]");
            expect_qmp_query_fdsets_with(&mut writer, &mut reader, "[]");
        });

        let mut client = QmpClient::connect(&socket).expect("connect fake qmp");
        let scaffold = qmp_scaffold("installer-usb", "boot", QemuMediaHotplugAction::Detach)
            .expect("scaffold");
        let commands = qmp_detach(&mut client, &scaffold).expect("idempotent qmp detach");
        assert_eq!(
            commands,
            [
                "device_del:absent",
                "blockdev-del:raw:absent",
                "blockdev-del:file:absent",
                "remove-fd:absent"
            ]
        );
        server.join().expect("fake qmp server joins");
    }

    #[test]
    fn qmp_detach_fails_closed_when_block_node_remains_after_error() {
        let dir = qmp_tempdir();
        let socket = dir.path().join("qmp.sock");
        let listener = UnixListener::bind(&socket).expect("bind qmp");
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept qmp");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("server read timeout");
            let mut writer = stream.try_clone().expect("clone qmp writer");
            let mut reader = BufReader::new(stream.try_clone().expect("clone qmp reader"));
            write_qmp_greeting_and_capabilities(&mut writer, &mut reader);

            expect_qmp_command_error(&mut writer, &mut reader, "device_del", "DeviceNotFound");
            expect_qmp_command_error(&mut writer, &mut reader, "blockdev-del", "GenericError");
            expect_qmp_query_named_block_nodes(
                &mut writer,
                &mut reader,
                r#"[{"node-name":"d2b-media-installer-usb"}]"#,
            );
            expect_qmp_command(&mut writer, &mut reader, "blockdev-del");
            expect_qmp_query_fdsets_with(&mut writer, &mut reader, "[]");
            expect_qmp_query_named_block_nodes(
                &mut writer,
                &mut reader,
                r#"[{"node-name":"d2b-media-installer-usb"}]"#,
            );
            expect_qmp_query_named_block_nodes(&mut writer, &mut reader, "[]");
            expect_qmp_query_fdsets_with(&mut writer, &mut reader, "[]");
        });

        let mut client = QmpClient::connect(&socket).expect("connect fake qmp");
        let scaffold = qmp_scaffold("installer-usb", "boot", QemuMediaHotplugAction::Detach)
            .expect("scaffold");
        let err = qmp_detach(&mut client, &scaffold).expect_err("qmp detach must fail closed");
        assert!(matches!(err, MediaOpError::Qmp(reason) if reason == "device_del:DeviceNotFound"));
        server.join().expect("fake qmp server joins");
    }

    #[test]
    fn qmp_attached_media_refs_ignore_non_d2b_and_invalid_opaque_values() {
        let dir = qmp_tempdir();
        let socket = dir.path().join("qmp.sock");
        let listener = UnixListener::bind(&socket).expect("bind qmp");
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept qmp");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("server read timeout");
            let mut writer = stream.try_clone().expect("clone qmp writer");
            let mut reader = BufReader::new(stream.try_clone().expect("clone qmp reader"));
            write_qmp_greeting_and_capabilities(&mut writer, &mut reader);
            expect_qmp_query_fdsets_with(
                &mut writer,
                &mut reader,
                r#"[{"fdset-id":1000,"fds":[{"fd":0,"opaque":"d2b:installer-usb"},{"fd":1,"opaque":"d2b:backup"},{"fd":2,"opaque":"d2b:Invalid"},{"fd":3,"opaque":"other:ignored"},{"fd":4}]}]"#,
            );
        });

        let mut client = QmpClient::connect(&socket).expect("connect fake qmp");
        assert_eq!(
            client
                .query_attached_media_refs()
                .expect("query attached media refs"),
            BTreeSet::from(["backup".to_owned(), "installer-usb".to_owned()])
        );
        server.join().expect("fake qmp server joins");
    }

    #[test]
    fn qmp_boot_path_attaches_media_and_continues_vm() {
        let dir = qmp_tempdir();
        let socket = dir.path().join("qmp.sock");
        let listener = UnixListener::bind(&socket).expect("bind qmp");
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept qmp");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("server read timeout");
            let mut writer = stream.try_clone().expect("clone qmp writer");
            let mut reader = BufReader::new(stream.try_clone().expect("clone qmp reader"));
            write_qmp_greeting_and_capabilities(&mut writer, &mut reader);

            let (_payload, fds) = crate::fd_passing::recv_fds(stream.as_raw_fd())
                .expect("boot add-fd must carry SCM_RIGHTS");
            assert_eq!(fds.len(), 1);
            let add_fd_id = String::from_utf8(_payload)
                .ok()
                .map(|line| qmp_id_from_line(&line))
                .expect("add-fd id");
            writer
                .write_all(
                    format!(r#"{{"return":{{"fdset-id":1000,"fd":0}},"id":"{add_fd_id}"}}"#)
                        .as_bytes(),
                )
                .expect("add-fd return");
            writer.write_all(b"\n").expect("add-fd newline");
            for expected in ["blockdev-add", "blockdev-add", "device_add", "cont"] {
                expect_qmp_command(&mut writer, &mut reader, expected);
            }
        });

        let mut client = QmpClient::connect(&socket).expect("connect fake qmp");
        let file = std::fs::File::open("/dev/null").expect("open harmless fd");
        let scaffold = qmp_scaffold("installer-usb", "boot", QemuMediaHotplugAction::Attach)
            .expect("scaffold");
        let mut commands = qmp_attach(
            &mut client,
            &scaffold,
            file.as_raw_fd(),
            QemuMediaSourceKind::PhysicalUsb,
            true,
        )
        .expect("qmp attach");
        qmp_continue_vm(&mut client).expect("qmp cont");
        commands.push("cont".to_owned());
        assert!(commands.contains(&"cont".to_owned()));
        server.join().expect("fake qmp server joins");
    }

    #[test]
    fn qmp_lifecycle_sends_powerdown_status_and_quit_as_typed_ops() {
        let dir = qmp_tempdir();
        let socket = dir.path().join("qmp.sock");
        let listener = UnixListener::bind(&socket).expect("bind qmp");
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept qmp");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("server read timeout");
            let mut writer = stream.try_clone().expect("clone qmp writer");
            let mut reader = BufReader::new(stream.try_clone().expect("clone qmp reader"));
            write_qmp_greeting_and_capabilities(&mut writer, &mut reader);

            expect_qmp_command(&mut writer, &mut reader, "system_powerdown");
            expect_qmp_query_status_with(&mut writer, &mut reader, "shutdown");
            expect_qmp_command(&mut writer, &mut reader, "quit");
        });

        let mut client = QmpClient::connect(&socket).expect("connect fake qmp");
        qmp_system_powerdown(&mut client).expect("system_powerdown");
        assert_eq!(
            qmp_query_status(&mut client).expect("query-status"),
            QemuMediaVmStatus::Shutdown
        );
        qmp_quit(&mut client).expect("quit");
        server.join().expect("fake qmp server joins");
    }

    #[test]
    fn qmp_query_status_maps_unknown_state_to_closed_enum() {
        assert_eq!(qmp_status_from_str("running"), QemuMediaVmStatus::Running);
        assert_eq!(
            qmp_status_from_str("new-future-state"),
            QemuMediaVmStatus::Unknown
        );
    }

    #[test]
    fn qmp_query_status_treats_missing_socket_as_shutdown_context() {
        let dir = qmp_tempdir();
        let missing = dir.path().join("missing.sock");
        let vm = d2b_contracts::types::VmId::new("media");

        let response = qmp_query_status_from_path(&vm, &missing, true)
            .expect("missing QMP socket during shutdown is expected");
        assert_eq!(
            response.status,
            QemuMediaVmStatus::ConnectionLostDuringShutdown
        );

        let err = qmp_query_status_from_path(&vm, &missing, false)
            .expect_err("missing QMP socket outside shutdown is an error");
        assert!(matches!(err, MediaOpError::Qmp(reason) if reason.contains("connect:")));
    }

    #[test]
    fn qmp_reader_rejects_oversized_response_before_json_parse() {
        let dir = qmp_tempdir();
        let socket = dir.path().join("qmp.sock");
        let listener = UnixListener::bind(&socket).expect("bind qmp");
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept qmp");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("server read timeout");
            let mut writer = stream.try_clone().expect("clone qmp writer");
            let mut reader = BufReader::new(stream.try_clone().expect("clone qmp reader"));
            write_qmp_greeting_and_capabilities(&mut writer, &mut reader);

            let mut line = String::new();
            reader.read_line(&mut line).expect("read query-status");
            assert!(line.contains("query-status"));
            let id = qmp_id_from_line(&line);
            let oversized = "x".repeat(QMP_MAX_RESPONSE_BYTES);
            writer
                .write_all(
                    format!(r#"{{"return":{{"status":"{oversized}"}},"id":"{id}"}}"#).as_bytes(),
                )
                .expect("oversized response");
            writer.write_all(b"\n").expect("oversized newline");
        });

        let mut client = QmpClient::connect(&socket).expect("connect fake qmp");
        let err = qmp_query_status(&mut client).expect_err("oversized response must fail");
        assert!(matches!(err, MediaOpError::Qmp(reason) if reason == "response-too-large"));
        server.join().expect("fake qmp server joins");
    }

    fn write_qmp_greeting_and_capabilities(
        writer: &mut UnixStream,
        reader: &mut BufReader<UnixStream>,
    ) {
        writer
            .write_all(
                br#"{"QMP":{"version":{"qemu":{"major":9,"minor":0,"micro":0}},"capabilities":[]}}"#,
            )
            .expect("write greeting");
        writer.write_all(b"\n").expect("write greeting newline");
        let mut line = String::new();
        reader.read_line(&mut line).expect("read capabilities");
        assert!(line.contains("qmp_capabilities"));
        let id = qmp_id_from_line(&line);
        writer
            .write_all(format!(r#"{{"return":{{}},"id":"{id}"}}"#).as_bytes())
            .expect("capabilities return");
        writer.write_all(b"\n").expect("capabilities newline");
    }

    fn expect_qmp_command(
        writer: &mut UnixStream,
        reader: &mut BufReader<UnixStream>,
        expected: &str,
    ) {
        let mut line = String::new();
        reader.read_line(&mut line).expect("read qmp command");
        assert!(line.contains(expected), "missing {expected} in {line}");
        let id = qmp_id_from_line(&line);
        writer
            .write_all(format!(r#"{{"return":{{}},"id":"{id}"}}"#).as_bytes())
            .expect("command return");
        writer.write_all(b"\n").expect("command newline");
    }

    fn expect_qmp_command_error(
        writer: &mut UnixStream,
        reader: &mut BufReader<UnixStream>,
        expected: &str,
        class: &str,
    ) {
        let mut line = String::new();
        reader.read_line(&mut line).expect("read qmp command");
        assert!(line.contains(expected), "missing {expected} in {line}");
        let id = qmp_id_from_line(&line);
        writer
            .write_all(
                format!(r#"{{"error":{{"class":"{class}","desc":"not present"}},"id":"{id}"}}"#)
                    .as_bytes(),
            )
            .expect("command error");
        writer.write_all(b"\n").expect("command error newline");
    }

    fn expect_qmp_query_named_block_nodes(
        writer: &mut UnixStream,
        reader: &mut BufReader<UnixStream>,
        nodes_json: &str,
    ) {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .expect("read query-named-block-nodes");
        assert!(
            line.contains("query-named-block-nodes"),
            "missing query-named-block-nodes in {line}"
        );
        let id = qmp_id_from_line(&line);
        writer
            .write_all(format!(r#"{{"return":{nodes_json},"id":"{id}"}}"#).as_bytes())
            .expect("query-named-block-nodes return");
        writer
            .write_all(b"\n")
            .expect("query-named-block-nodes newline");
    }

    fn expect_qmp_query_fdsets(writer: &mut UnixStream, reader: &mut BufReader<UnixStream>) {
        expect_qmp_query_fdsets_with(
            writer,
            reader,
            r#"[{"fdset-id":1000,"fds":[{"fd":0,"opaque":"d2b:installer-usb"}]}]"#,
        );
    }

    fn expect_qmp_query_status_with(
        writer: &mut UnixStream,
        reader: &mut BufReader<UnixStream>,
        status: &str,
    ) {
        let mut line = String::new();
        reader.read_line(&mut line).expect("read query-status");
        assert!(
            line.contains("query-status"),
            "missing query-status in {line}"
        );
        let id = qmp_id_from_line(&line);
        writer
            .write_all(format!(r#"{{"return":{{"status":"{status}"}},"id":"{id}"}}"#).as_bytes())
            .expect("query-status return");
        writer.write_all(b"\n").expect("query-status newline");
    }

    fn expect_qmp_query_fdsets_with(
        writer: &mut UnixStream,
        reader: &mut BufReader<UnixStream>,
        fdsets_json: &str,
    ) {
        let mut line = String::new();
        reader.read_line(&mut line).expect("read query-fdsets");
        assert!(
            line.contains("query-fdsets"),
            "missing query-fdsets in {line}"
        );
        let id = qmp_id_from_line(&line);
        writer
            .write_all(format!(r#"{{"return":{fdsets_json},"id":"{id}"}}"#).as_bytes())
            .expect("query-fdsets return");
        writer.write_all(b"\n").expect("query-fdsets newline");
    }

    fn qmp_id_from_line(line: &str) -> String {
        serde_json::from_str::<Value>(line)
            .expect("qmp json")
            .get("id")
            .and_then(Value::as_str)
            .expect("qmp id")
            .to_owned()
    }

    #[test]
    fn registry_writes_runtime_only_root_private_modes() {
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("registry tempdir");
        let root = dir.path().join("registry");
        let uid = rustix::process::getuid().as_raw();
        let gid = rustix::process::getgid().as_raw();
        write_registry_record_at_root(&root, &registry_record(), uid, gid).expect("write registry");

        let vm_dir = root.join("media");
        let record_path = vm_dir.join("installer-usb.json");
        assert_eq!(
            std::fs::metadata(&root)
                .expect("registry root metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&vm_dir)
                .expect("registry vm metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&record_path)
                .expect("registry record metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let raw = std::fs::read_to_string(record_path).expect("record json");
        assert!(raw.contains("usb-Vendor_SecretSerial"));
    }

    #[test]
    fn runtime_udev_rules_are_root_private_and_cover_partitions() {
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("udev tempdir");
        let path = dir
            .path()
            .join("run/udev/rules.d/99-d2b-media-ignore.rules");
        let mut record = registry_record();
        record.identity.by_id_names = vec![
            "usb-Vendor_SecretSerial".to_owned(),
            "usb-Vendor_SecretSerial-part1".to_owned(),
            "usb-Quoted\"Serial\\Path".to_owned(),
        ];

        write_runtime_udev_rules_at_path(&path, &[record]).expect("write udev rules");

        assert_eq!(
            std::fs::metadata(&path)
                .expect("udev rule metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let text = std::fs::read_to_string(&path).expect("udev rules");
        assert!(text.contains("ENV{UDISKS_IGNORE}=\"1\""));
        assert!(text.contains("*/dev/disk/by-id/usb-Vendor_SecretSerial*"));
        assert!(text.contains("*/dev/disk/by-id/usb-Vendor_SecretSerial-part1*"));
        assert!(text.contains("usb-Quoted\\\"Serial\\\\Path"));
        assert!(!text.contains("/nix/store"));
    }

    #[test]
    fn open_readback_identity_must_match_registry_record() {
        let record = registry_record();
        assert!(verify_identity_matches_record(&record, &physical_identity()).is_ok());

        let mut devnum_changed = physical_identity();
        devnum_changed.devnum = 8;
        assert!(matches!(
            verify_identity_matches_record(&record, &devnum_changed),
            Err(MediaOpError::IdentityMismatch(reason)) if reason == "devnum"
        ));

        let mut by_id_changed = physical_identity();
        by_id_changed.by_id_names = vec!["usb-Other_Device".to_owned()];
        assert!(matches!(
            verify_identity_matches_record(&record, &by_id_changed),
            Err(MediaOpError::IdentityMismatch(reason)) if reason == "by-id"
        ));
    }

    #[test]
    fn runtime_selector_matching_ignores_stale_busid_and_block_device() {
        let mut record = registry_record();
        record.identity.bus_id = "9-9".to_owned();
        record.identity.devnum = 99;
        record.identity.block_device = "sdz".to_owned();

        assert!(runtime_identity_matches_record(
            &record,
            &physical_identity()
        ));
    }

    #[test]
    fn runtime_selector_matching_requires_stable_by_id_identity() {
        let record = registry_record();
        let mut identity = physical_identity();
        identity.by_id_names = vec!["usb-Other_Device".to_owned()];

        assert!(!runtime_identity_matches_record(&record, &identity));
    }

    #[test]
    fn runtime_selector_ambiguity_fails_closed() {
        let first = registry_record();
        let mut second = registry_record();
        second.media_ref = "tools-usb".to_owned();

        assert!(matches!(
            select_unique_runtime_record(&[first, second], "media", &physical_identity()),
            Err(MediaOpError::AmbiguousRuntimeSelector(refs))
                if refs == vec!["installer-usb".to_owned(), "tools-usb".to_owned()]
        ));
    }

    #[test]
    fn declared_selector_prefers_explicit_match_and_falls_back_to_unique_unselected_source() {
        let identity = physical_identity();
        let selected = declared_source("installer-usb", Some("usb-Vendor_SecretSerial"));
        let fallback = declared_source("fallback-usb", None);
        let other_vm = QemuMediaSourceIntent {
            vm: "other".to_owned(),
            ..declared_source("other-usb", Some("usb-Vendor_SecretSerial"))
        };
        let sources = vec![fallback.clone(), other_vm, selected.clone()];

        assert_eq!(
            select_unique_declared_physical_source_from_sources(&sources, "media", &identity, None)
                .expect("selector match")
                .media_ref,
            selected.media_ref
        );

        let mut moved_identity = identity.clone();
        moved_identity.by_id_names = vec!["usb-New_Runtime_Name".to_owned()];
        assert_eq!(
            select_unique_declared_physical_source_from_sources(
                &sources,
                "media",
                &moved_identity,
                None
            )
            .expect("unique unselected fallback")
            .media_ref,
            fallback.media_ref
        );
    }

    #[test]
    fn declared_selector_ambiguity_and_empty_matches_fail_closed() {
        let identity = physical_identity();
        let first = declared_source("installer-usb", Some("usb-Vendor_SecretSerial"));
        let second = declared_source("tools-usb", Some("usb-Vendor_SecretSerial"));
        assert!(matches!(
            select_unique_declared_physical_source_from_sources(
                &[first, second],
                "media",
                &identity,
                None
            ),
            Err(MediaOpError::AmbiguousRuntimeSelector(refs))
                if refs == vec!["installer-usb".to_owned(), "tools-usb".to_owned()]
        ));

        let no_selector_a = declared_source("backup-usb", None);
        let no_selector_b = declared_source("tools-usb", None);
        let mut moved_identity = identity.clone();
        moved_identity.by_id_names = vec!["usb-New_Runtime_Name".to_owned()];
        assert!(matches!(
            select_unique_declared_physical_source_from_sources(
                &[no_selector_a, no_selector_b],
                "media",
                &moved_identity,
                None
            ),
            Err(MediaOpError::AmbiguousRuntimeSelector(refs))
                if refs == vec!["backup-usb".to_owned(), "tools-usb".to_owned()]
        ));

        let selected = declared_source("installer-usb", Some("usb-Other_Device"));
        assert!(matches!(
            select_unique_declared_physical_source_from_sources(
                &[selected],
                "media",
                &identity,
                None
            ),
            Err(MediaOpError::IdentityMismatch(reason)) if reason == "runtime-selector"
        ));
    }

    #[test]
    fn usb_selector_matches_by_id_names_only() {
        let selector = QemuMediaUsbSelector {
            by_id_name: "usb-Vendor_SecretSerial".to_owned(),
        };
        assert!(usb_selector_matches(&selector, &physical_identity()));

        let mut identity = physical_identity();
        identity.by_id_names = vec!["usb-Other_Device".to_owned()];
        assert!(!usb_selector_matches(&selector, &identity));
    }

    #[test]
    fn read_usb_identity_for_selector_uses_injected_roots_and_fails_closed() {
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("selector tempdir");
        let sysfs_root = dir.path().join("sys");
        let by_id_root = dir.path().join("by-id");
        write_usb_candidate_fixture(
            &sysfs_root,
            &by_id_root,
            "1-2.3",
            "null",
            "usb-Vendor_SecretSerial",
        );
        let selector = QemuMediaUsbSelector {
            by_id_name: "usb-Vendor_SecretSerial".to_owned(),
        };

        let identity = read_usb_identity_for_selector_at_roots(&sysfs_root, &by_id_root, &selector)
            .expect("selector identity");
        assert_eq!(identity.bus_id, "1-2.3");
        assert_eq!(identity.block_device, "null");
        assert_eq!(identity.by_id_names, vec!["usb-Vendor_SecretSerial"]);

        let missing = QemuMediaUsbSelector {
            by_id_name: "usb-Missing".to_owned(),
        };
        assert!(matches!(
            read_usb_identity_for_selector_at_roots(&sysfs_root, &by_id_root, &missing),
            Err(MediaOpError::IdentityMismatch(reason)) if reason == "configured-selector"
        ));

        let ambiguous = [
            candidate("1-2.3", "null", &["usb-Same"]),
            candidate("2-1", "zero", &["usb-Same"]),
        ];
        let selector = QemuMediaUsbSelector {
            by_id_name: "usb-Same".to_owned(),
        };
        assert!(matches!(
            select_unique_usb_candidate_for_selector(&ambiguous, &selector),
            Err(MediaOpError::AmbiguousRuntimeSelector(refs))
                if refs == vec!["1-2.3".to_owned(), "2-1".to_owned()]
        ));
    }

    #[test]
    fn declared_selector_artifacts_write_registry_index_and_udev_rules() {
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("boot artifacts tempdir");
        let registry_root = dir.path().join("registry");
        let redacted_index = dir.path().join("run/d2b/qemu-media-registry-index.json");
        let rules_path = dir
            .path()
            .join("run/udev/rules.d/99-d2b-media-ignore.rules");
        let uid = rustix::process::getuid().as_raw();
        let gid = rustix::process::getgid().as_raw();

        let audit = write_declared_selector_artifacts(
            &registry_root,
            &redacted_index,
            &rules_path,
            &declared_source("installer-usb", Some("usb-Vendor_SecretSerial")),
            &physical_identity(),
            uid,
            gid,
            false,
        )
        .expect("write boot selector artifacts");

        assert!(audit.registry_record_written);
        assert!(audit.redacted_index_written);
        assert!(audit.udev_rule_written);
        assert!(!audit.udev_reloaded);

        let record = std::fs::read_to_string(registry_root.join("media/installer-usb.json"))
            .expect("registry record");
        assert!(record.contains("usb-Vendor_SecretSerial"));

        let index = std::fs::read_to_string(redacted_index).expect("redacted index");
        assert!(index.contains("identityHash"));
        assert!(!index.contains("usb-Vendor_SecretSerial"));

        let rules = std::fs::read_to_string(rules_path).expect("udev rules");
        assert!(rules.contains("ENV{UDISKS_IGNORE}=\"1\""));
        assert!(rules.contains("*/dev/disk/by-id/usb-Vendor_SecretSerial*"));
    }

    #[test]
    fn detach_selector_falls_back_to_unique_attached_same_model_ref() {
        let record = registry_record();
        let mut moved_identity = physical_identity();
        moved_identity.by_id_names = vec!["usb-Same_Model_New_Runtime_Id".to_owned()];
        let attached_refs = BTreeSet::from(["installer-usb".to_owned()]);

        assert!(matches!(
            select_unique_runtime_record(std::slice::from_ref(&record), "media", &moved_identity),
            Err(MediaOpError::IdentityMismatch(reason)) if reason == "runtime-selector"
        ));
        assert_eq!(
            select_unique_attached_detach_record(
                &[record],
                "media",
                &moved_identity,
                &attached_refs
            )
            .expect("unique attached fallback")
            .media_ref,
            "installer-usb"
        );
    }

    #[test]
    fn detach_selector_fallback_uses_attached_ref_among_same_model_records() {
        let first = registry_record();
        let mut second = registry_record();
        second.media_ref = "tools-usb".to_owned();
        second.identity.by_id_names = vec!["usb-Tools_Original".to_owned()];
        let mut moved_identity = physical_identity();
        moved_identity.by_id_names = vec!["usb-Same_Model_New_Runtime_Id".to_owned()];

        assert_eq!(
            select_unique_attached_detach_record(
                &[first.clone(), second.clone()],
                "media",
                &moved_identity,
                &BTreeSet::from(["tools-usb".to_owned()])
            )
            .expect("unique attached fallback")
            .media_ref,
            "tools-usb"
        );
        assert!(matches!(
            select_unique_attached_detach_record(
                &[first, second],
                "media",
                &moved_identity,
                &BTreeSet::new()
            ),
            Err(MediaOpError::IdentityMismatch(reason)) if reason == "runtime-selector"
        ));
    }

    #[test]
    fn detach_selector_fallback_fails_closed_when_ambiguous() {
        let first = registry_record();
        let mut second = registry_record();
        second.media_ref = "tools-usb".to_owned();
        let attached_refs = BTreeSet::from(["installer-usb".to_owned(), "tools-usb".to_owned()]);

        assert!(matches!(
            select_unique_attached_detach_record(
                &[first, second],
                "media",
                &physical_identity(),
                &attached_refs
            ),
            Err(MediaOpError::AmbiguousRuntimeSelector(refs))
                if refs == vec!["installer-usb".to_owned(), "tools-usb".to_owned()]
        ));
    }

    #[test]
    fn detach_selector_fallback_is_allowed_only_for_empty_runtime_selector() {
        let empty = MediaOpError::IdentityMismatch("runtime-selector".to_owned());
        assert!(runtime_selector_allows_detach_fallback(&empty));

        let by_id = MediaOpError::IdentityMismatch("by-id".to_owned());
        assert!(!runtime_selector_allows_detach_fallback(&by_id));

        let ambiguous =
            MediaOpError::AmbiguousRuntimeSelector(vec!["a".to_owned(), "b".to_owned()]);
        assert!(!runtime_selector_allows_detach_fallback(&ambiguous));
    }

    #[test]
    fn image_path_shape_rejects_relative_and_parent_escape() {
        assert!(matches!(
            validate_image_path_shape(Path::new("images/installer.img")),
            Err(MediaOpError::ImagePathUnsafe(reason)) if reason == "path-not-absolute"
        ));
        assert!(matches!(
            validate_image_path_shape(Path::new("/var/lib/d2b/../escape.img")),
            Err(MediaOpError::ImagePathUnsafe(reason)) if reason == "parent-dir-component"
        ));
        assert!(validate_image_path_shape(Path::new("/var/lib/d2b/images/installer.img")).is_ok());
    }

    #[test]
    fn image_mount_and_loop_preflights_detect_busy_paths() {
        let image = Path::new("/var/lib/d2b/images/space image.img");
        assert!(image_path_mounted_in_proc_mounts(
            "/var/lib/d2b/images/space\\040image.img /mnt ext4 rw 0 0\n",
            image
        ));

        let root = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("tempdir");
        let loop_dir = root.path().join("block/loop0/loop");
        std::fs::create_dir_all(&loop_dir).expect("loop dir");
        std::fs::write(
            loop_dir.join("backing_file"),
            "/var/lib/d2b/images/space image.img\n",
        )
        .expect("backing file");

        assert!(image_has_loop_backing(root.path(), image).expect("loop scan"));
    }

    #[test]
    fn image_metadata_rejects_non_root_owner_or_writable_mode() {
        let file = std::fs::File::open("/etc/hosts").expect("/etc/hosts");
        let mut stat = crate::sys::path_safe::fstat_fd(file.as_fd()).expect("stat /etc/hosts");
        stat.st_uid = 1000;
        stat.st_mode = libc::S_IFREG | 0o600;
        assert!(matches!(
            validate_image_file_metadata(&stat),
            Err(MediaOpError::ImagePathUnsafe(reason)) if reason.starts_with("file-not-root-owned")
        ));

        stat.st_uid = 0;
        stat.st_mode = libc::S_IFREG | 0o620;
        assert!(matches!(
            validate_image_file_metadata(&stat),
            Err(MediaOpError::ImagePathUnsafe(reason))
                if reason.starts_with("file-writable-by-group-or-other")
        ));
    }

    #[test]
    fn image_fd_locking_fails_when_already_locked() {
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("image tempdir");
        let path = dir.path().join("installer.img");
        std::fs::write(&path, b"not a real disk image").expect("image file");

        let first: OwnedFd = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("first open")
            .into();
        lock_image_fd(&first, MediaAccessMode::ReadWrite).expect("first lock");

        let second: OwnedFd = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .expect("second open")
            .into();
        assert!(matches!(
            lock_image_fd(&second, MediaAccessMode::ReadWrite),
            Err(MediaOpError::ImageBusy(reason)) if reason.starts_with("lock:")
        ));
    }
}
