//! The committed broker operation catalog.
//!
//! One committed row per broker operation carries everything the broker and
//! the operator-facing views need: the wire discriminant it inherits (when it
//! has one), the three-way ownership triage, the declaring provider, the
//! profiles that admit it, the authorization facet, and the audit field its
//! records use.
//!
//! The rows are the source. The generated table in
//! `generated/broker_operation_catalog.rs` is written by
//! `xtask gen-broker-operations` from
//! `docs/reference/policy/broker-operations.json`, and every other catalog -
//! the wire enum, the fixed profile catalogs, the `W3BrokerOperation`
//! inventory, the authorization rows, and the typed audit fields - is
//! compared against the same rows by [`audit`], so a row cannot move without
//! the views moving with it.
//!
//! Two bindings are structural rather than checked. [`wire_row`] is an
//! exhaustive match over the wire enum, so a variant a row does not name does
//! not compile. [`audit_field_name`] is the same for the typed audit fields,
//! so an audit shape no row claims does not compile either.

use d2b_contracts_broker::broker_wire::BrokerRequest;

use crate::ops::audit_op::OperationFields;

/// The owner of one committed operation row.
///
/// The triage is closed: every row names exactly one owner, and a name
/// outside this set is not a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OperationOwner {
    /// A resource-family driver declares the operation's handler in its own
    /// crate.
    Family,
    /// The broker itself owns the effect; the operation has no resource
    /// family.
    BrokerGeneric,
    /// The name is a transport-layer concern the operation envelope does not
    /// carry.
    TransportExcluded,
}

impl OperationOwner {
    /// The operator-facing class label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Family => "family",
            Self::BrokerGeneric => "broker-generic",
            Self::TransportExcluded => "transport-excluded",
        }
    }
}

/// A fixed broker profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BrokerProfileId {
    Host,
    Guest,
}

/// Where one operation's payload contract comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadProvenance {
    /// The typed wire request is the payload contract; the operation is not
    /// reachable through the generic envelope.
    Wire,
    /// The caller supplies a payload object the row's schema validates.
    Request,
}

/// The committed authorization facet of one operation.
///
/// Grants are deny-by-default: a caller whose authority the facet does not
/// admit is refused, and an operation with no committed facet is not a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrokerAuthzFacets {
    /// Operation subject.
    pub subject: &'static str,
    /// Operation scope.
    pub scope: &'static str,
    /// Authority classes the operation admits; empty denies every caller.
    pub allowed_groups: &'static [&'static str],
    /// Whether state mutation, teardown, rollback, GC, or live routing
    /// changes are possible.
    pub destructive: bool,
    /// Secret exposure class.
    pub secret_access: &'static str,
    /// Broker-use class.
    pub broker_required: &'static str,
    /// Audit mode.
    pub audit_mode: &'static str,
}

/// One committed broker operation row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrokerOperationRow {
    /// The operation name.
    pub operation: &'static str,
    /// The wire discriminant the operation inherits, when it has one.
    pub wire_variant: Option<&'static str>,
    /// The ownership triage result.
    pub owner: OperationOwner,
    /// The resource family, for family-owned rows.
    pub family: Option<&'static str>,
    /// The crate that declares the family handler.
    pub declaring_provider: Option<&'static str>,
    /// The profiles that admit the operation.
    pub profiles: &'static [BrokerProfileId],
    /// Whether the operation is a member of the `W3BrokerOperation`
    /// inventory.
    pub w3: bool,
    /// Whether the operation is advertised by `BrokerCapabilities`.
    pub capabilities: bool,
    /// The disposition the operation was triaged under.
    pub disposition: &'static str,
    /// The wave a still-stubbed operation is reserved for.
    pub stub_wave: Option<&'static str>,
    /// Every typed audit field the operation's records can carry.
    pub audit_fields: &'static [&'static str],
    /// The committed authorization facet.
    pub authz: BrokerAuthzFacets,
    /// Where the payload contract comes from.
    pub payload_provenance: PayloadProvenance,
    /// The payload property names the operation's schema declares.
    pub payload_fields: &'static [&'static str],
    /// The payload property names the operation's schema requires.
    pub payload_required: &'static [&'static str],
}

include!("generated/broker_operation_catalog.rs");

impl BrokerOperationRow {
    /// The committed row one operation name declares.
    pub fn find(operation: &str) -> Option<&'static Self> {
        BROKER_OPERATION_CATALOG
            .iter()
            .find(|row| row.operation == operation)
    }

    /// Whether a fixed profile admits the operation.
    pub fn admits_profile(&self, profile: BrokerProfileId) -> bool {
        self.profiles.contains(&profile)
    }

    /// Whether the operation is reachable through the generic envelope.
    pub fn is_generic(&self) -> bool {
        self.payload_provenance == PayloadProvenance::Request
    }
}

/// The wave a still-stubbed operation is reserved for.
///
/// The reserved operations are the committed rows whose disposition is
/// `stubbed-unimplemented`; the broker serves them with one typed
/// "not implemented yet" refusal instead of one dispatch arm each.
pub fn stub_wave(operation: &str) -> Option<&'static str> {
    BrokerOperationRow::find(operation).and_then(|row| row.stub_wave)
}

/// The committed row one wire variant names.
///
/// The match is exhaustive, so a variant no committed row declares does not
/// compile: the wire enum cannot grow past the committed rows.
pub fn wire_row(request: &BrokerRequest) -> &'static BrokerOperationRow {
    fn row(operation: &str) -> &'static BrokerOperationRow {
        BrokerOperationRow::find(operation)
            .unwrap_or_else(|| panic!("wire variant {operation} has no committed row"))
    }
    match request {
        BrokerRequest::ApplyHostGenerationHandoff(..) => row("ApplyHostGenerationHandoff"),
        BrokerRequest::ApplyNftables(..) => row("ApplyNftables"),
        BrokerRequest::ApplyNftablesProjection(..) => row("ApplyNftablesProjection"),
        BrokerRequest::ApplyNmUnmanaged(..) => row("ApplyNmUnmanaged"),
        BrokerRequest::ApplyRoute(..) => row("ApplyRoute"),
        BrokerRequest::ApplySysctl(..) => row("ApplySysctl"),
        BrokerRequest::BindUnixSocket(..) => row("BindUnixSocket"),
        BrokerRequest::CreateOrReconcileUsersGroups(..) => row("CreateOrReconcileUsersGroups"),
        BrokerRequest::CreateBridge(..) => row("CreateBridge"),
        BrokerRequest::DeleteBridge(..) => row("DeleteBridge"),
        BrokerRequest::CreatePersistentTap(..) => row("CreatePersistentTap"),
        BrokerRequest::DeletePersistentTap(..) => row("DeletePersistentTap"),
        BrokerRequest::CreateTapFd(..) => row("CreateTapFd"),
        BrokerRequest::DelegateCgroupV2(..) => row("DelegateCgroupV2"),
        BrokerRequest::ExportBrokerAudit(..) => row("ExportBrokerAudit"),
        BrokerRequest::Hello(..) => row("Hello"),
        BrokerRequest::InjectSecretById(..) => row("InjectSecretById"),
        BrokerRequest::LaunchMinijailChild(..) => row("LaunchMinijailChild"),
        BrokerRequest::ModprobeIfAllowed(..) => row("ModprobeIfAllowed"),
        BrokerRequest::OpenCgroupDir(..) => row("OpenCgroupDir"),
        BrokerRequest::OpenDevice(..) => row("OpenDevice"),
        BrokerRequest::OpenFuse(..) => row("OpenFuse"),
        BrokerRequest::OpenHidrawSecurityKey(..) => row("OpenHidrawSecurityKey"),
        BrokerRequest::OpenKvm(..) => row("OpenKvm"),
        BrokerRequest::QemuMediaEnroll(..) => row("QemuMediaEnroll"),
        BrokerRequest::QemuMediaRefreshRegistry(..) => row("QemuMediaRefreshRegistry"),
        BrokerRequest::QemuMediaBoot(..) => row("QemuMediaBoot"),
        BrokerRequest::QemuMediaSystemPowerdown(..) => row("QemuMediaSystemPowerdown"),
        BrokerRequest::QemuMediaQueryStatus(..) => row("QemuMediaQueryStatus"),
        BrokerRequest::QemuMediaQuit(..) => row("QemuMediaQuit"),
        BrokerRequest::QemuMediaAttach(..) => row("QemuMediaAttach"),
        BrokerRequest::QemuMediaDetach(..) => row("QemuMediaDetach"),
        BrokerRequest::OpenPidfd(..) => row("OpenPidfd"),
        BrokerRequest::ConsumeLifecycleLease(..) => row("ConsumeLifecycleLease"),
        BrokerRequest::OpenPeerPidfdFromAcceptedSocket(..) => row("OpenPeerPidfdFromAcceptedSocket"),
        BrokerRequest::ObserveRunner(..) => row("ObserveRunner"),
        BrokerRequest::PipeWireAudio(..) => row("PipeWireAudio"),
        BrokerRequest::StartSystemdUnit(..) => row("StartSystemdUnit"),
        BrokerRequest::CheckSystemdUserManager(..) => row("CheckSystemdUserManager"),
        BrokerRequest::ObserveSystemdUnit(..) => row("ObserveSystemdUnit"),
        BrokerRequest::OpenSystemdUnitPidfd(..) => row("OpenSystemdUnitPidfd"),
        BrokerRequest::StopSystemdUnit(..) => row("StopSystemdUnit"),
        BrokerRequest::OpenVhostNet(..) => row("OpenVhostNet"),
        BrokerRequest::PauseBroker => row("PauseBroker"),
        BrokerRequest::PollChildReaped => row("PollChildReaped"),
        BrokerRequest::PrepareRuntimeDir(..) => row("PrepareRuntimeDir"),
        BrokerRequest::PrepareStateDir(..) => row("PrepareStateDir"),
        BrokerRequest::MigrateLegacySwtpmState(..) => row("MigrateLegacySwtpmState"),
        BrokerRequest::ReconcileStorageScope(..) => row("ReconcileStorageScope"),
        BrokerRequest::ValidateLockSpec(..) => row("ValidateLockSpec"),
        BrokerRequest::PrepareStoreView(..) => row("PrepareStoreView"),
        BrokerRequest::StoreSync(..) => row("StoreSync"),
        BrokerRequest::StoreVerify(..) => row("StoreVerify"),
        BrokerRequest::ReadSecretById(..) => row("ReadSecretById"),
        BrokerRequest::ResumeBroker => row("ResumeBroker"),
        BrokerRequest::RotateSecretById(..) => row("RotateSecretById"),
        BrokerRequest::RunHostInstall(..) => row("RunHostInstall"),
        BrokerRequest::RunMigrate(..) => row("RunMigrate"),
        BrokerRequest::RunActivation(..) => row("RunActivation"),
        BrokerRequest::RunGc(..) => row("RunGc"),
        BrokerRequest::RunKeysRotate(..) => row("RunKeysRotate"),
        BrokerRequest::RunHostKeyTrust(..) => row("RunHostKeyTrust"),
        BrokerRequest::RunRotateKnownHost(..) => row("RunRotateKnownHost"),
        BrokerRequest::SetBridgePortFlags(..) => row("SetBridgePortFlags"),
        BrokerRequest::SetSocketAcl(..) => row("SetSocketAcl"),
        BrokerRequest::SetupMountNamespace(..) => row("SetupMountNamespace"),
        BrokerRequest::CgroupKill(..) => row("CgroupKill"),
        BrokerRequest::SignalRunner(..) => row("SignalRunner"),
        BrokerRequest::DeregisterRunnerPidfd(..) => row("DeregisterRunnerPidfd"),
        BrokerRequest::SpawnRunner(..) => row("SpawnRunner"),
        BrokerRequest::UpdateHostsFile(..) => row("UpdateHostsFile"),
        BrokerRequest::UsbipBind(..) => row("UsbipBind"),
        BrokerRequest::UsbipBindFirewallRule(..) => row("UsbipBindFirewallRule"),
        BrokerRequest::UsbipProxyReconcile(..) => row("UsbipProxyReconcile"),
        BrokerRequest::UsbipUnbind(..) => row("UsbipUnbind"),
        BrokerRequest::UsbipExplicitBind(..) => row("UsbipExplicitBind"),
        BrokerRequest::UsbipExplicitFirewallRule(..) => row("UsbipExplicitFirewallRule"),
        BrokerRequest::ResourceActivationAudit(..) => row("ResourceActivationAudit"),
        BrokerRequest::ValidateBundle => row("ValidateBundle"),
        BrokerRequest::SeedDnsmasqLease(..) => row("SeedDnsmasqLease"),
        BrokerRequest::BindMountFromHardlinkFarm(..) => row("BindMountFromHardlinkFarm"),
        BrokerRequest::OwnershipMatrixCheck(..) => row("OwnershipMatrixCheck"),
        BrokerRequest::SshHostKeyPreflight(..) => row("SshHostKeyPreflight"),
        BrokerRequest::DiskInit(..) => row("DiskInit"),
        BrokerRequest::SecurityKeyOpenDevice(..) => row("SecurityKeyOpenDevice"),
        BrokerRequest::SecurityKeyApplyUdevRules(..) => row("SecurityKeyApplyUdevRules"),
        BrokerRequest::Invoke(..) => row("Invoke"),
    }
}

/// Name every typed audit shape the broker records.
///
/// One list feeds both artifacts: the exhaustive match, so an audit shape no
/// arm names does not compile, and [`AUDIT_FIELD_NAMES`], the view the
/// completeness gate compares against the committed rows.
macro_rules! audit_fields {
    ($($variant:pat => $name:literal),* $(,)?) => {
        /// The audit field name of one typed audit record.
        pub fn audit_field_name(fields: &OperationFields) -> &'static str {
            match fields {
                $($variant => $name,)*
            }
        }

        /// Every audit field name a record can carry.
        pub const AUDIT_FIELD_NAMES: &[&str] = &[$($name,)*];
    };
}

audit_fields! {
        OperationFields::ApplyNftables { .. } => "ApplyNftables",
        OperationFields::ApplyNftablesProjection { .. } => "ApplyNftablesProjection",
        OperationFields::ApplyRoute { .. } => "ApplyRoute",
        OperationFields::DelegateCgroupV2 { .. } => "DelegateCgroupV2",
        OperationFields::CgroupKill { .. } => "CgroupKill",
        OperationFields::OpenCgroupDir { .. } => "OpenCgroupDir",
        OperationFields::CreateTapFd { .. } => "CreateTapFd",
        OperationFields::CreatePersistentTap { .. } => "CreatePersistentTap",
        OperationFields::CreateBridge { .. } => "CreateBridge",
        OperationFields::DeleteBridge { .. } => "DeleteBridge",
        OperationFields::DeletePersistentTap { .. } => "DeletePersistentTap",
        OperationFields::OpenKvm { .. } => "OpenKvm",
        OperationFields::OpenVhostNet { .. } => "OpenVhostNet",
        OperationFields::OpenFuse { .. } => "OpenFuse",
        OperationFields::OpenHidrawSecurityKey { .. } => "OpenHidrawSecurityKey",
        OperationFields::OpenDevice { .. } => "OpenDevice",
        OperationFields::ModprobeIfAllowed { .. } => "ModprobeIfAllowed",
        OperationFields::PrepareRuntimeDir { .. } => "PrepareRuntimeDir",
        OperationFields::PrepareStateDir { .. } => "PrepareStateDir",
        OperationFields::MigrateLegacySwtpmState { .. } => "MigrateLegacySwtpmState",
        OperationFields::PrepareSwtpmDir(..) => "PrepareSwtpmDir",
        OperationFields::PrepareStoreView { .. } => "PrepareStoreView",
        OperationFields::StoreSync(..) => "StoreSync",
        OperationFields::StoreVerify { .. } => "StoreVerify",
        OperationFields::SetBridgePortFlags { .. } => "SetBridgePortFlags",
        OperationFields::SetupMountNamespace { .. } => "SetupMountNamespace",
        OperationFields::SpawnRunner { .. } => "SpawnRunner",
        OperationFields::OpenPidfd { .. } => "OpenPidfd",
        OperationFields::ConsumeLifecycleLease { .. } => "ConsumeLifecycleLease",
        OperationFields::OpenPeerPidfdFromAcceptedSocket { .. } => "OpenPeerPidfdFromAcceptedSocket",
        OperationFields::ObserveRunner { .. } => "ObserveRunner",
        OperationFields::PipeWireAudio { .. } => "PipeWireAudio",
        OperationFields::SystemdUnit { .. } => "SystemdUnit",
        OperationFields::RunHostInstall { .. } => "RunHostInstall",
        OperationFields::RunMigrate { .. } => "RunMigrate",
        OperationFields::UsbipBind { .. } => "UsbipBind",
        OperationFields::UsbSerialCorrelationKeyRotate(..) => "UsbSerialCorrelationKeyRotate",
        OperationFields::UsbipUnbind { .. } => "UsbipUnbind",
        OperationFields::UsbipProxyReconcile { .. } => "UsbipProxyReconcile",
        OperationFields::UsbipBindFirewallRule { .. } => "UsbipBindFirewallRule",
        OperationFields::UsbipExplicitBind { .. } => "UsbipExplicitBind",
        OperationFields::UsbipExplicitFirewallRule { .. } => "UsbipExplicitFirewallRule",
        OperationFields::QemuMediaEnroll { .. } => "QemuMediaEnroll",
        OperationFields::QemuMediaRefreshRegistry { .. } => "QemuMediaRefreshRegistry",
        OperationFields::QemuMediaAttach { .. } => "QemuMediaAttach",
        OperationFields::QemuMediaBoot { .. } => "QemuMediaBoot",
        OperationFields::QemuMediaSystemPowerdown { .. } => "QemuMediaSystemPowerdown",
        OperationFields::QemuMediaQuit { .. } => "QemuMediaQuit",
        OperationFields::QemuMediaDetach { .. } => "QemuMediaDetach",
        OperationFields::ApplyNmUnmanaged { .. } => "ApplyNmUnmanaged",
        OperationFields::ApplySysctl { .. } => "ApplySysctl",
        OperationFields::UpdateHostsFile { .. } => "UpdateHostsFile",
        OperationFields::SeedDnsmasqLease { .. } => "SeedDnsmasqLease",
        OperationFields::BindMountFromHardlinkFarm { .. } => "BindMountFromHardlinkFarm",
        OperationFields::SignalRunner { .. } => "SignalRunner",
        OperationFields::DeregisterRunnerPidfd { .. } => "DeregisterRunnerPidfd",
        OperationFields::RunActivation { .. } => "RunActivation",
        OperationFields::ApplyHostGenerationHandoff { .. } => "ApplyHostGenerationHandoff",
        OperationFields::RunGc { .. } => "RunGc",
        OperationFields::RunKeysRotate { .. } => "RunKeysRotate",
        OperationFields::RunHostKeyTrust { .. } => "RunHostKeyTrust",
        OperationFields::RunRotateKnownHost { .. } => "RunRotateKnownHost",
        OperationFields::Hello { .. } => "Hello",
        OperationFields::ResourceActivationAudit { .. } => "ResourceActivationAudit",
        OperationFields::ValidateBundle { .. } => "ValidateBundle",
        OperationFields::ExportBrokerAudit { .. } => "ExportBrokerAudit",
        OperationFields::DiskInit { .. } => "DiskInit",
        OperationFields::ReconcileStorageScope { .. } => "ReconcileStorageScope",
        OperationFields::ValidateLockSpec { .. } => "ValidateLockSpec",
        OperationFields::Invoke { .. } => "Invoke",
}

/// One mismatch between the committed rows and a derived view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogMismatch {
    /// The view that disagrees with the rows.
    pub view: &'static str,
    /// What disagrees, named.
    pub detail: String,
}

/// The derived views the completeness gate compares against the rows.
#[derive(Debug, Clone, Default)]
pub struct CatalogViews {
    /// The closed Host profile catalog.
    pub host: Vec<&'static str>,
    /// The closed Guest profile catalog.
    pub guest: Vec<&'static str>,
    /// The `W3BrokerOperation` inventory, by wire tag.
    pub w3: Vec<&'static str>,
    /// The private broker authorization rows, by operation name.
    pub authz: Vec<&'static str>,
    /// Every typed audit field a record can carry.
    pub audit: Vec<&'static str>,
    /// The advertised broker capabilities.
    pub capabilities: Vec<String>,
}

fn missing(view: &'static str, detail: String) -> CatalogMismatch {
    CatalogMismatch { view, detail }
}

fn sequence_matches<T, U>(
    view: &'static str,
    declared: &[T],
    expected: &[U],
    mismatches: &mut Vec<CatalogMismatch>,
) where
    T: AsRef<str>,
    U: AsRef<str>,
{
    if declared.len() == expected.len()
        && declared
            .iter()
            .zip(expected)
            .all(|(left, right)| left.as_ref() == right.as_ref())
    {
        return;
    }
    let declared_set: std::collections::BTreeSet<&str> =
        declared.iter().map(AsRef::as_ref).collect();
    let expected_set: std::collections::BTreeSet<&str> =
        expected.iter().map(AsRef::as_ref).collect();
    for name in expected_set.difference(&declared_set) {
        mismatches.push(missing(view, format!("{name}: declared by the rows, absent")));
    }
    for name in declared_set.difference(&expected_set) {
        mismatches.push(missing(view, format!("{name}: present, no row declares it")));
    }
    if declared_set == expected_set {
        mismatches.push(missing(
            view,
            "declared order differs from the committed row order".to_owned(),
        ));
    }
}

fn names(rows: impl Iterator<Item = &'static str>) -> Vec<&'static str> {
    rows.collect()
}

/// Compare every derived view against the committed rows.
///
/// The gate fails on any variant, row, profile, authorization, or audit
/// mismatch, and names what disagrees.
pub fn audit(rows: &[BrokerOperationRow], views: &CatalogViews) -> Vec<CatalogMismatch> {
    let mut mismatches = Vec::new();
    let mut operations = std::collections::BTreeSet::new();
    let mut wire_variants = std::collections::BTreeSet::new();
    for row in rows {
        if !operations.insert(row.operation) {
            mismatches.push(missing("rows", format!("{}: declared twice", row.operation)));
        }
        if let Some(variant) = row.wire_variant {
            if !wire_variants.insert(variant) {
                mismatches.push(missing("rows", format!("{variant}: two rows inherit it")));
            }
            if variant != row.operation {
                mismatches.push(missing(
                    "rows",
                    format!("{}: inherits wire variant {variant}", row.operation),
                ));
            }
            if !row.admits_profile(BrokerProfileId::Host) {
                mismatches.push(missing(
                    "rows",
                    format!("{}: a wire operation no fixed profile admits", row.operation),
                ));
            }
        }
        if row.owner == OperationOwner::Family && row.declaring_provider.is_none() {
            mismatches.push(missing(
                "rows",
                format!("{}: family-owned without a declaring provider", row.operation),
            ));
        }
        if row.owner != OperationOwner::Family && row.family.is_some() {
            mismatches.push(missing(
                "rows",
                format!("{}: not family-owned but names a family", row.operation),
            ));
        }
        if (row.disposition == "stubbed-unimplemented") != row.stub_wave.is_some() {
            mismatches.push(missing(
                "dispositions",
                format!(
                    "{}: a {} row with {} reserved wave",
                    row.operation,
                    row.disposition,
                    if row.stub_wave.is_some() { "a" } else { "no" }
                ),
            ));
        }
        if row.authz.allowed_groups.is_empty() {
            mismatches.push(missing(
                "authz",
                format!("{}: no allowed group, so every caller is denied", row.operation),
            ));
        }
        for field in row.audit_fields {
            if !views.audit.contains(field) {
                mismatches.push(missing(
                    "audit",
                    format!("{field}: declared by {}, not an audit field", row.operation),
                ));
            }
        }
    }
    for operation in operations.iter().copied() {
        if !views.authz.contains(&operation) {
            mismatches.push(missing(
                "authz",
                format!("{operation}: committed row, no authorization row"),
            ));
        }
    }
    sequence_matches(
        "profiles",
        &views.host.clone(),
        &names(
            rows.iter()
                .filter(|row| {
                    row.wire_variant.is_some() && row.admits_profile(BrokerProfileId::Host)
                })
                .filter_map(|row| row.wire_variant),
        ),
        &mut mismatches,
    );
    sequence_matches(
        "profiles",
        &views.guest.clone(),
        &names(
            rows.iter()
                .filter(|row| {
                    row.wire_variant.is_some() && row.admits_profile(BrokerProfileId::Guest)
                })
                .filter_map(|row| row.wire_variant),
        ),
        &mut mismatches,
    );
    sequence_matches(
        "w3",
        &views.w3.clone(),
        &names(rows.iter().filter(|row| row.w3).map(|row| row.operation)),
        &mut mismatches,
    );
    sequence_matches(
        "authz",
        &views.authz.clone(),
        &names(rows.iter().map(|row| row.operation)),
        &mut mismatches,
    );
    // The capability advertisement is a set: it is sorted and deduplicated
    // before it is advertised, so only its membership is compared.
    let declared: std::collections::BTreeSet<&str> =
        views.capabilities.iter().map(String::as_str).collect();
    let expected: std::collections::BTreeSet<&str> = rows
        .iter()
        .filter(|row| row.capabilities)
        .map(|row| row.operation)
        .collect();
    for name in expected.difference(&declared) {
        mismatches.push(missing(
            "capabilities",
            format!("{name}: declared by the rows, absent"),
        ));
    }
    for name in declared.difference(&expected) {
        mismatches.push(missing(
            "capabilities",
            format!("{name}: present, no row declares it"),
        ));
    }
    for field in &views.audit {
        if !rows
            .iter()
            .any(|row| row.audit_fields.contains(field))
        {
            mismatches.push(missing(
                "audit",
                format!("{field}: an audit field no committed row uses"),
            ));
        }
    }
    mismatches
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::privileges_w3::W3BrokerOperation;
    use d2b_contracts_broker::BrokerCapabilities;
    use d2b_contracts_broker::broker_wire::{
        GUEST_OPERATION_CATALOG, HOST_OPERATION_CATALOG,
    };

    /// Every view as the running binary carries it.
    fn live_views() -> CatalogViews {
        CatalogViews {
            host: HOST_OPERATION_CATALOG.to_vec(),
            guest: GUEST_OPERATION_CATALOG.to_vec(),
            w3: W3BrokerOperation::all()
                .iter()
                .map(|operation| operation.wire_tag())
                .collect(),
            authz: d2b_core::privileges::BROKER_OPERATION_AUTHZ
                .iter()
                .map(|row| row.operation)
                .collect(),
            audit: audit_field_names(),
            capabilities: BrokerCapabilities::w3().broker_operations,
        }
    }

    fn audit_field_names() -> Vec<&'static str> {
        AUDIT_FIELD_NAMES.to_vec()
    }

    #[test]
    fn committed_rows_cover_every_view() {
        let mismatches = audit(BROKER_OPERATION_CATALOG, &live_views());
        assert_eq!(mismatches, Vec::new(), "catalog views drifted from the rows");
    }

    #[test]
    fn every_reserved_stub_carries_its_wave() {
        let stubs = BROKER_OPERATION_CATALOG
            .iter()
            .filter(|row| row.disposition == "stubbed-unimplemented")
            .count();
        assert!(stubs > 0, "the triage records reserved operations");
        for row in BROKER_OPERATION_CATALOG {
            assert_eq!(
                stub_wave(row.operation).is_some(),
                row.disposition == "stubbed-unimplemented",
                "{}: the reserved wave and the disposition disagree",
                row.operation
            );
        }
    }

    #[test]
    fn a_stub_without_a_wave_fails_the_gate() {
        let mut rows = BROKER_OPERATION_CATALOG.to_vec();
        let stub = rows
            .iter_mut()
            .find(|row| row.disposition == "stubbed-unimplemented")
            .expect("the catalog reserves at least one operation");
        stub.stub_wave = None;
        let mismatches = audit(&rows, &live_views());
        assert!(
            mismatches
                .iter()
                .any(|mismatch| mismatch.view == "dispositions"),
            "a stub with no reserved wave must fail: {mismatches:?}"
        );
    }

    #[test]
    fn every_ownership_class_owns_rows() {
        for owner in [
            OperationOwner::Family,
            OperationOwner::BrokerGeneric,
            OperationOwner::TransportExcluded,
        ] {
            assert!(
                BROKER_OPERATION_CATALOG
                    .iter()
                    .any(|row| row.owner == owner),
                "{} owns no committed row",
                owner.as_str()
            );
        }
    }

    #[test]
    fn a_mismatched_wire_variant_fails_the_gate() {
        let mut views = live_views();
        views.host.push("NoSuchOperation");
        let mismatches = audit(BROKER_OPERATION_CATALOG, &views);
        assert!(
            mismatches.iter().any(|mismatch| mismatch.view == "profiles"
                && mismatch.detail.contains("NoSuchOperation")),
            "a profile catalog entry no row declares must fail: {mismatches:?}"
        );
    }

    #[test]
    fn a_mismatched_row_fails_the_gate() {
        let mut rows = BROKER_OPERATION_CATALOG.to_vec();
        rows[0].operation = "RenamedOperation";
        let mismatches = audit(&rows, &live_views());
        assert!(
            mismatches
                .iter()
                .any(|mismatch| mismatch.view == "authz" || mismatch.view == "rows"),
            "a renamed row must fail the gate: {mismatches:?}"
        );
    }

    #[test]
    fn a_dropped_authorization_row_fails_the_gate() {
        let mut views = live_views();
        views.authz.remove(0);
        let mismatches = audit(BROKER_OPERATION_CATALOG, &views);
        assert!(
            mismatches
                .iter()
                .any(|mismatch| mismatch.view == "authz" && mismatch.detail.contains("absent")),
            "a missing authorization row must fail: {mismatches:?}"
        );
    }

    #[test]
    fn an_ungranted_row_fails_the_gate() {
        let mut rows = BROKER_OPERATION_CATALOG.to_vec();
        rows[0].authz.allowed_groups = &[];
        let mismatches = audit(&rows, &live_views());
        assert!(
            mismatches
                .iter()
                .any(|mismatch| mismatch.view == "authz" && mismatch.detail.contains("denied")),
            "a row with no allowed group must fail: {mismatches:?}"
        );
    }

    #[test]
    fn every_wire_variant_resolves_to_its_row() {
        for name in HOST_OPERATION_CATALOG {
            let row = BrokerOperationRow::find(name)
                .unwrap_or_else(|| panic!("{name} has no committed row"));
            assert_eq!(row.wire_variant, Some(*name));
        }
    }

    #[test]
    fn the_generic_entry_point_is_generic() {
        let row = BrokerOperationRow::find("Invoke").expect("Invoke is a committed row");
        assert!(row.is_generic());
        assert_eq!(row.owner, OperationOwner::BrokerGeneric);
    }
}
