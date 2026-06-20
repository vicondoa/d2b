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

use nix::libc;
use nixling_core::bundle_resolver::BundleResolver;
use nixling_core::host::{QemuMediaFormat, QemuMediaSourceIntent, QemuMediaSourceKind};
use nixling_host::media::{
    MediaAccessMode, QemuMediaHotplugAction, QemuMediaHotplugScaffold, UsbPhysicalIdentity,
};
use nixling_ipc::broker_wire::{
    QemuMediaBootRequest, QemuMediaEnrollRequest, QemuMediaEnrollResponse, QemuMediaHotplugEvent,
    QemuMediaHotplugRequest, QemuMediaHotplugResponse, QemuMediaHotplugStatus,
    QemuMediaRefreshRegistryResponse,
};
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

const REDACTED_INDEX_PATH: &str = "/run/nixling/qemu-media-registry-index.json";

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

pub struct RefreshOutcome {
    pub response: QemuMediaRefreshRegistryResponse,
}

pub fn enroll(
    resolver: &BundleResolver,
    req: &QemuMediaEnrollRequest,
) -> Result<EnrollOutcome, MediaOpError> {
    nixling_host::media::validate_media_ref(req.media_ref.as_str())
        .map_err(|err| MediaOpError::InvalidRef(err.to_string()))?;
    nixling_host::media::validate_usb_busid(&req.bus_id)
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
    write_redacted_registry_index(&records)?;
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
    let redacted_index_written = write_redacted_registry_index(&records).map(|_| true)?;
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
) -> Result<HotplugOutcome, MediaOpError> {
    let source = resolve_boot_source(resolver, req.vm_id.as_str())?;
    let opened = open_declared_source(resolver, source)?;
    run_attach_transaction(req.vm_id.as_str(), opened, true)
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
    nixling_host::media::validate_usb_busid(&req.bus_id)
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

fn open_runtime_selector_source<'a>(
    resolver: &'a BundleResolver,
    req: &QemuMediaHotplugRequest,
) -> Result<OpenedMedia<'a>, MediaOpError> {
    nixling_host::media::validate_usb_busid(&req.bus_id)
        .map_err(|err| MediaOpError::InvalidBusId(err.to_string()))?;
    let identity = read_usb_identity(Path::new("/sys"), Path::new("/dev/disk/by-id"), &req.bus_id)?;
    let (_record, source) = resolve_runtime_selector(resolver, req.vm_id.as_str(), &identity)?;
    preflight_identity_not_busy(Path::new("/sys"), &identity)?;
    let fd = open_block_device(&identity.block_device, access_mode(source))?;
    Ok(OpenedMedia { source, fd })
}

fn open_declared_source<'a>(
    resolver: &'a BundleResolver,
    source: &'a QemuMediaSourceIntent,
) -> Result<OpenedMedia<'a>, MediaOpError> {
    let access = access_mode(source);
    let fd = match source.source_kind {
        QemuMediaSourceKind::PhysicalUsb => {
            let record =
                read_registry_record(resolver, source.vm.as_str(), source.media_ref.as_str())?;
            let identity = read_current_identity_for_record(&record)?;
            preflight_identity_not_busy(Path::new("/sys"), &identity)?;
            open_block_device(&identity.block_device, access)?
        }
        QemuMediaSourceKind::ImageFile => open_image_file(source, access)?,
    };
    Ok(OpenedMedia { source, fd })
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

fn hotplug_response(
    vm: &str,
    source: &QemuMediaSourceIntent,
    scaffold: QemuMediaHotplugScaffold,
    qmp_commands: Vec<String>,
    statuses: Vec<QemuMediaHotplugStatus>,
) -> QemuMediaHotplugResponse {
    QemuMediaHotplugResponse {
        vm_id: nixling_ipc::types::VmId::new(vm.to_owned()),
        media_ref: nixling_ipc::types::MediaRef::new(scaffold.media_ref),
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
    nixling_host::media::qemu_media_hotplug_scaffold(media_ref, slot, action)
        .map_err(|err| MediaOpError::QmpScaffold(format!("{err:?}")))
}

fn qmp_socket_path(vm: &str) -> PathBuf {
    PathBuf::from("/run/nixling/vms").join(vm).join("qmp.sock")
}

fn qemu_media_file_node_id(media_ref: &str) -> String {
    format!("nl-file-{media_ref}")
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
            "opaque": format!("nixling:{}", scaffold.media_ref),
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
        if self.fdset_added {
            if let Some((fdset_id, fd)) = self
                .fdset_id
                .zip(self.fd)
                .or_else(|| client.query_fdset_entry(&self.media_ref).ok().flatten())
            {
                let _ =
                    client.execute("remove-fd", json!({ "fdset-id": fdset_id, "fd": fd }), None);
            }
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
    match client.execute(
        "remove-fd",
        json!({ "fdset-id": fdset_id, "fd": fd }),
        None,
    ) {
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
        let id = format!("nixling-{}", self.next_id);
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
        let expected_opaque = format!("nixling:{media_ref}");
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
                    .and_then(|opaque| opaque.strip_prefix("nixling:"))
                    && nixling_host::media::validate_media_ref(media_ref).is_ok()
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
        let mut line = String::new();
        let bytes = self
            .reader
            .read_line(&mut line)
            .map_err(|err| MediaOpError::Qmp(format!("read:{err}")))?;
        if bytes == 0 {
            return Err(MediaOpError::Qmp("eof".to_owned()));
        }
        serde_json::from_str(&line).map_err(|err| MediaOpError::Qmp(format!("decode:{err}")))
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
    let candidates = nixling_host::media::safe_usb_block_candidates(
        Path::new("/sys"),
        Path::new("/dev/disk/by-id"),
    );
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
            let record =
                select_unique_attached_detach_record(&records, vm, identity, &attached_refs)?;
            let source = resolve_physical_source(resolver, vm, &record.media_ref)?;
            Ok((record, source))
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
    let usb_dir = nixling_host::media::sysfs_usb_device_dir(sysfs_root, bus_id);
    let devnum = read_trimmed(&usb_dir.join("devnum"))
        .map_err(|err| MediaOpError::Sysfs(format!("devnum:{err}")))
        .and_then(|value| {
            nixling_host::media::parse_devnum(&value)
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
            && let Ok(name) = nixling_host::media::by_id_name_from_path(&path)
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
    let report = nixling_host::media::preflight_device_not_in_use(
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
    let block_dir = nixling_host::media::sysfs_block_device_dir(sysfs_root, block_device);
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
    Ok(fd.into())
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
    crate::sys::path_safe::ensure_dir(root, 0o700, Some(owner_uid), Some(owner_gid))
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
    crate::sys::path_safe::atomic_replace_fd(
        &vm_fd,
        &format!("{}.json", record.media_ref),
        &bytes,
        0o600,
    )
    .map_err(|err| MediaOpError::Registry(err.to_string()))
}

fn write_redacted_registry_index(records: &[MediaRegistryRecord]) -> Result<(), MediaOpError> {
    let path = Path::new(REDACTED_INDEX_PATH);
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
    crate::sys::path_safe::atomic_replace_fd(&parent_fd, name, &bytes, 0o644)
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
        "# nixling qemu-media physical USB ignore rules\n\
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
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn qmp_attach_sends_fd_and_device_commands() {
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("qmp tempdir");
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
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("qmp tempdir");
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
                    br#"{"event":"DEVICE_DELETED","data":{"device":"nl-usb-installer-usb"}}"#,
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
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("qmp tempdir");
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
                r#"[{"fdset-id":1000,"fds":[{"fd":0,"opaque":"nixling:installer-usb"}]}]"#,
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
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("qmp tempdir");
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
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("qmp tempdir");
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
                r#"[{"node-name":"nl-media-installer-usb"}]"#,
            );
            expect_qmp_command(&mut writer, &mut reader, "blockdev-del");
            expect_qmp_query_fdsets_with(&mut writer, &mut reader, "[]");
            expect_qmp_query_named_block_nodes(
                &mut writer,
                &mut reader,
                r#"[{"node-name":"nl-media-installer-usb"}]"#,
            );
            expect_qmp_query_named_block_nodes(&mut writer, &mut reader, "[]");
            expect_qmp_query_fdsets_with(&mut writer, &mut reader, "[]");
        });

        let mut client = QmpClient::connect(&socket).expect("connect fake qmp");
        let scaffold = qmp_scaffold("installer-usb", "boot", QemuMediaHotplugAction::Detach)
            .expect("scaffold");
        let err = qmp_detach(&mut client, &scaffold).expect_err("qmp detach must fail closed");
        assert!(
            matches!(err, MediaOpError::Qmp(reason) if reason == "device_del:DeviceNotFound")
        );
        server.join().expect("fake qmp server joins");
    }

    #[test]
    fn qmp_attached_media_refs_ignore_non_nixling_and_invalid_opaque_values() {
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("qmp tempdir");
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
                r#"[{"fdset-id":1000,"fds":[{"fd":0,"opaque":"nixling:installer-usb"},{"fd":1,"opaque":"nixling:backup"},{"fd":2,"opaque":"nixling:Invalid"},{"fd":3,"opaque":"other:ignored"},{"fd":4}]}]"#,
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
        let dir = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("qmp tempdir");
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
            r#"[{"fdset-id":1000,"fds":[{"fd":0,"opaque":"nixling:installer-usb"}]}]"#,
        );
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
            .join("run/udev/rules.d/99-nixling-media-ignore.rules");
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
            validate_image_path_shape(Path::new("/var/lib/nixling/../escape.img")),
            Err(MediaOpError::ImagePathUnsafe(reason)) if reason == "parent-dir-component"
        ));
        assert!(
            validate_image_path_shape(Path::new("/var/lib/nixling/images/installer.img")).is_ok()
        );
    }

    #[test]
    fn image_mount_and_loop_preflights_detect_busy_paths() {
        let image = Path::new("/var/lib/nixling/images/space image.img");
        assert!(image_path_mounted_in_proc_mounts(
            "/var/lib/nixling/images/space\\040image.img /mnt ext4 rw 0 0\n",
            image
        ));

        let root = tempfile::tempdir_in(env!("CARGO_MANIFEST_DIR")).expect("tempdir");
        let loop_dir = root.path().join("block/loop0/loop");
        std::fs::create_dir_all(&loop_dir).expect("loop dir");
        std::fs::write(
            loop_dir.join("backing_file"),
            "/var/lib/nixling/images/space image.img\n",
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
