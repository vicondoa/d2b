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
//! Two bindings are structural rather than checked. The `wire_variants!`
//! name match is exhaustive over the wire enum, so a variant a name list does
//! not carry does not compile, and [`WIRE_VARIANTS`] is the view the
//! completeness gate compares against the rows' `wire_variant` facets.
//! [`audit_field_name`] is the same for the typed audit fields, so an audit
//! shape no row claims does not compile either.

use std::collections::{BTreeMap, BTreeSet};

use d2b_contracts_broker::broker_wire::{
    BrokerRequest, DEFAULT_CONTEXT_DEADLINE_MS, FdKind, MAX_CONTEXT_DEADLINE_MS,
};
use d2b_contracts_resource::v3::{CanonicalJsonObject, CanonicalJsonValue, canonical_json_bytes};

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

/// The declared durability facet of one state cell.
///
/// The facet rides the committed row (U3/KTD3): a one-time cell persists its
/// consumed records under the broker's state root and refuses re-consume of a
/// completed record across a broker restart; an ephemeral cell keeps
/// in-process reset-on-restart semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellDurability {
    /// Completed records persist durably; re-consume refuses across restarts
    /// and retention never evicts a consumed marker.
    OneTime,
    /// Records live in the broker process only and reset on restart.
    Ephemeral,
}

/// The closed deadline-tier set of one committed operation row (KTD4).
///
/// The tier is the row's declared per-call budget: the broker mints the
/// tier's concrete budget into the attested context block, and both
/// execution legs serve the context's budget as the handler deadline -
/// never a flat per-leg constant. The budgets are the shared carrier's own
/// constants (see [`DeadlineTier::budget_ms`]), so a tier cannot drift
/// from the contract the receiving leg enforces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadlineTier {
    /// The default per-call budget: the carrier's historical fixed handler
    /// deadline, which every context-free leg also serves.
    Standard,
    /// The largest per-call budget the carrier admits: a row whose work
    /// legitimately outruns the standard tier runs under the shared
    /// absolute ceiling.
    Extended,
}

impl DeadlineTier {
    /// The concrete budget one tier admits, in milliseconds.
    ///
    /// The values are the shared carrier's own constants (`broker_wire`
    /// remains the source: `DEFAULT_CONTEXT_DEADLINE_MS` and
    /// `MAX_CONTEXT_DEADLINE_MS`), so the envelope and the receiving leg
    /// cannot disagree about what a tier means.
    pub const fn budget_ms(self) -> u64 {
        match self {
            Self::Standard => DEFAULT_CONTEXT_DEADLINE_MS,
            Self::Extended => MAX_CONTEXT_DEADLINE_MS,
        }
    }
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
    /// Why a row no family owns is not provider-owned.
    ///
    /// A row the broker or a transport concern owns records what owns it and
    /// why; a family-owned row carries its declaring provider instead, so
    /// exactly one of the two facets is set.
    pub justification: Option<&'static str>,
    /// The profiles that admit the operation.
    pub profiles: &'static [BrokerProfileId],
    /// Whether the operation is a member of the `W3BrokerOperation`
    /// inventory.
    pub w3: bool,
    /// Whether the operation is advertised by `BrokerCapabilities`.
    pub capabilities: bool,
    /// The disposition the operation was triaged under.
    pub disposition: &'static str,
    /// The deferral marker a reserved stub carries, when it is one; the
    /// closed set the generator maps the committed disposition target onto.
    pub stub_target: Option<StubTarget>,
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
    /// The payload property names the operation's audit join is derived
    /// from, when the operation declares a durable per-invocation identity.
    pub audit_join: Option<&'static [&'static str]>,
    /// The most descriptors one invocation of this operation may carry in
    /// the forward frame. 0 when the operation declares no fd carriage.


    pub max_fds: u8,
    /// The kernel kind every descriptor this operation carries must present,
    /// required when [`Self::max_fds`] is nonzero. 
    pub fd_kind: Option<FdKind>,
    /// The declared state cell the operation's broker-owned state lives on,
    /// when it has one (U3/KTD3).
    pub state_cell: Option<&'static str>,
    /// The cell's declared durability facet, present exactly when
    /// [`Self::state_cell`] is.
    pub cell_durability: Option<CellDurability>,
    /// The row's declared deadline tier (KTD4): the concrete budget the
    /// broker mints into the attested context block, which both execution
    /// legs serve as the per-call handler deadline. Rows without a
    /// declared tier sit on the standard tier.
    pub deadline_tier: DeadlineTier,
}

include!("generated/broker_operation_catalog.rs");

impl BrokerOperationRow {
    /// The committed row one operation name declares.
    pub fn find(operation: &str) -> Option<&'static Self> {
        BROKER_OPERATION_CATALOG
            .iter()
            .find(|row| row.operation == operation)
    }

    /// The audit identity one validated payload carries.
    ///
    /// The key is canonical JSON over the operation's declared join fields,
    /// taken from the validated payload: two invocations of one operation
    /// join under one identity exactly when they carry the same declared
    /// fields. An operation that declares no join has no per-invocation
    /// identity, and its record derives its own.
    pub fn audit_join_identity(&self, payload: &CanonicalJsonObject) -> Option<String> {
        let fields = self.audit_join?;
        let mut declared: BTreeMap<&str, &CanonicalJsonValue> = BTreeMap::new();
        for name in fields {
            declared.insert(name, payload.get(name)?);
        }
        // A payload the envelope validated carries canonical values already,
        // so the declared key always canonicalizes: the fallback keeps a
        // malformed payload from panicking the daemon rather than hiding a
        // reachable path.
        let bytes = canonical_json_bytes(&declared).ok()?;
        Some(d2b_audit::operation_identity_of_canonical_json(&bytes))
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

/// Where a reserved stub's implementation is expected to land.
///
/// A still-stubbed operation's refusal carries this marker on the wire and in
/// its audit record; the generator maps each reserved row's committed
/// disposition target onto the closed set, so the marker cannot drift into
/// free prose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StubTarget {
    /// The implementation is still to be written, with no committed home yet.
    FutureWork,
    /// The wire shape exists and is reserved; the effect does not.
    Reserved,
    /// Only the bootstrap dispatch path realizes the operation.
    BootstrapOnly,
}

impl StubTarget {
    /// The marker the refusal carries on the wire and in its audit record.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FutureWork => "future-work",
            Self::Reserved => "reserved",
            Self::BootstrapOnly => "bootstrap-only",
        }
    }
}

/// The deferral marker of a still-stubbed operation, when the operation is one.
///
/// The reserved operations are the committed rows whose disposition is
/// `stubbed-unimplemented`; the broker serves them with one typed
/// "not implemented yet" refusal instead of one dispatch arm each, and the
/// refusal names where the implementation is expected to land.
pub fn stub_target(operation: &str) -> Option<StubTarget> {
    BrokerOperationRow::find(operation).and_then(|row| row.stub_target)
}

/// Name every wire variant of `BrokerRequest`, in enum order.
///
/// One list feeds both artifacts: the exhaustive name match, so a variant no
/// arm names does not compile, and [`WIRE_VARIANTS`], the view the
/// completeness gate compares against the committed rows.
macro_rules! wire_variants {
    ($($variant:pat => $name:literal),* $(,)?) => {
        /// The committed name of one wire variant.
        pub fn wire_variant_name(request: &BrokerRequest) -> &'static str {
            match request {
                $($variant => $name,)*
            }
        }

        /// Every wire variant name, in the order the wire enum declares them.
        pub const WIRE_VARIANTS: &[&str] = &[$($name,)*];
    };
}

wire_variants! {
        BrokerRequest::ApplyHostGenerationHandoff(..) => "ApplyHostGenerationHandoff",
        BrokerRequest::ApplyNftables(..) => "ApplyNftables",
        BrokerRequest::ApplyNftablesProjection(..) => "ApplyNftablesProjection",
        BrokerRequest::ApplyNmUnmanaged(..) => "ApplyNmUnmanaged",
        BrokerRequest::ApplyRoute(..) => "ApplyRoute",
        BrokerRequest::ApplySysctl(..) => "ApplySysctl",
        BrokerRequest::CreateOrReconcileUsersGroups(..) => "CreateOrReconcileUsersGroups",
        BrokerRequest::CreateBridge(..) => "CreateBridge",
        BrokerRequest::DeleteBridge(..) => "DeleteBridge",
        BrokerRequest::CreatePersistentTap(..) => "CreatePersistentTap",
        BrokerRequest::DeletePersistentTap(..) => "DeletePersistentTap",
        BrokerRequest::CreateTapFd(..) => "CreateTapFd",
        BrokerRequest::DelegateCgroupV2(..) => "DelegateCgroupV2",
        BrokerRequest::ExportBrokerAudit(..) => "ExportBrokerAudit",
        BrokerRequest::Hello(..) => "Hello",
        BrokerRequest::PublishTrustedContext(..) => "PublishTrustedContext",
        BrokerRequest::InjectSecretById(..) => "InjectSecretById",
        BrokerRequest::LaunchMinijailChild(..) => "LaunchMinijailChild",
        BrokerRequest::ModprobeIfAllowed(..) => "ModprobeIfAllowed",
        BrokerRequest::OpenCgroupDir(..) => "OpenCgroupDir",
        BrokerRequest::OpenDevice(..) => "OpenDevice",
        BrokerRequest::OpenFuse(..) => "OpenFuse",
        BrokerRequest::OpenHidrawSecurityKey(..) => "OpenHidrawSecurityKey",
        BrokerRequest::OpenKvm(..) => "OpenKvm",
        BrokerRequest::QemuMediaEnroll(..) => "QemuMediaEnroll",
        BrokerRequest::QemuMediaRefreshRegistry(..) => "QemuMediaRefreshRegistry",
        BrokerRequest::QemuMediaBoot(..) => "QemuMediaBoot",
        BrokerRequest::QemuMediaSystemPowerdown(..) => "QemuMediaSystemPowerdown",
        BrokerRequest::QemuMediaQueryStatus(..) => "QemuMediaQueryStatus",
        BrokerRequest::QemuMediaQuit(..) => "QemuMediaQuit",
        BrokerRequest::QemuMediaAttach(..) => "QemuMediaAttach",
        BrokerRequest::QemuMediaDetach(..) => "QemuMediaDetach",
        BrokerRequest::OpenPidfd(..) => "OpenPidfd",
        BrokerRequest::ConsumeLifecycleLease(..) => "ConsumeLifecycleLease",
        BrokerRequest::OpenPeerPidfdFromAcceptedSocket(..) => "OpenPeerPidfdFromAcceptedSocket",
        BrokerRequest::ObserveRunner(..) => "ObserveRunner",
        BrokerRequest::PipeWireAudio(..) => "PipeWireAudio",
        BrokerRequest::StartSystemdUnit(..) => "StartSystemdUnit",
        BrokerRequest::CheckSystemdUserManager(..) => "CheckSystemdUserManager",
        BrokerRequest::ObserveSystemdUnit(..) => "ObserveSystemdUnit",
        BrokerRequest::OpenSystemdUnitPidfd(..) => "OpenSystemdUnitPidfd",
        BrokerRequest::StopSystemdUnit(..) => "StopSystemdUnit",
        BrokerRequest::OpenVhostNet(..) => "OpenVhostNet",
        BrokerRequest::PollChildReaped => "PollChildReaped",
        BrokerRequest::PrepareRuntimeDir(..) => "PrepareRuntimeDir",
        BrokerRequest::PrepareStateDir(..) => "PrepareStateDir",
        BrokerRequest::ReconcileStorageScope(..) => "ReconcileStorageScope",
        BrokerRequest::ValidateLockSpec(..) => "ValidateLockSpec",
        BrokerRequest::StoreSync(..) => "StoreSync",
        BrokerRequest::ReadSecretById(..) => "ReadSecretById",
        BrokerRequest::RotateSecretById(..) => "RotateSecretById",
        BrokerRequest::SetBridgePortFlags(..) => "SetBridgePortFlags",
        BrokerRequest::CgroupKill(..) => "CgroupKill",
        BrokerRequest::SignalRunner(..) => "SignalRunner",
        BrokerRequest::DeregisterRunnerPidfd(..) => "DeregisterRunnerPidfd",
        BrokerRequest::SpawnRunner(..) => "SpawnRunner",
        BrokerRequest::UpdateHostsFile(..) => "UpdateHostsFile",
        BrokerRequest::UsbipBind(..) => "UsbipBind",
        BrokerRequest::UsbipBindFirewallRule(..) => "UsbipBindFirewallRule",
        BrokerRequest::UsbipProxyReconcile(..) => "UsbipProxyReconcile",
        BrokerRequest::UsbipUnbind(..) => "UsbipUnbind",
        BrokerRequest::UsbipExplicitBind(..) => "UsbipExplicitBind",
        BrokerRequest::UsbipExplicitFirewallRule(..) => "UsbipExplicitFirewallRule",
        BrokerRequest::SeedDnsmasqLease(..) => "SeedDnsmasqLease",
        BrokerRequest::OwnershipMatrixCheck(..) => "OwnershipMatrixCheck",
        BrokerRequest::SshHostKeyPreflight(..) => "SshHostKeyPreflight",
        BrokerRequest::DiskInit(..) => "DiskInit",
        BrokerRequest::SecurityKeyOpenDevice(..) => "SecurityKeyOpenDevice",
        BrokerRequest::SecurityKeyApplyUdevRules(..) => "SecurityKeyApplyUdevRules",
}

/// The committed row one wire variant names, when a row declares it.
///
/// The name match is exhaustive, so a variant no row declares cannot be
/// reached by naming it: a new variant is a compile error until it is named
/// here, and [`audit`] refuses a name whose row is missing.
pub fn wire_row(request: &BrokerRequest) -> Option<&'static BrokerOperationRow> {
    BrokerOperationRow::find(wire_variant_name(request))
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
        OperationFields::PrepareSwtpmDir(..) => "PrepareSwtpmDir",
        OperationFields::StoreSync(..) => "StoreSync",
        OperationFields::SetBridgePortFlags { .. } => "SetBridgePortFlags",
        OperationFields::SpawnRunner { .. } => "SpawnRunner",
        OperationFields::OpenPidfd { .. } => "OpenPidfd",
        OperationFields::ConsumeLifecycleLease { .. } => "ConsumeLifecycleLease",
        OperationFields::OpenPeerPidfdFromAcceptedSocket { .. } => "OpenPeerPidfdFromAcceptedSocket",
        OperationFields::ObserveRunner { .. } => "ObserveRunner",
        OperationFields::PipeWireAudio { .. } => "PipeWireAudio",
        OperationFields::SystemdUnit { .. } => "SystemdUnit",
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
        OperationFields::SignalRunner { .. } => "SignalRunner",
        OperationFields::DeregisterRunnerPidfd { .. } => "DeregisterRunnerPidfd",
        OperationFields::ApplyHostGenerationHandoff { .. } => "ApplyHostGenerationHandoff",
        OperationFields::Hello { .. } => "Hello",
        OperationFields::PublishTrustedContext { .. } => "PublishTrustedContext",
        OperationFields::ExportBrokerAudit { .. } => "ExportBrokerAudit",
        OperationFields::DiskInit { .. } => "DiskInit",
        OperationFields::ReconcileStorageScope { .. } => "ReconcileStorageScope",
        OperationFields::ValidateLockSpec { .. } => "ValidateLockSpec",
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
    /// Every wire variant name of the request enum.
    pub wire: Vec<&'static str>,
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
        if row.owner != OperationOwner::Family && row.justification.is_none() {
            mismatches.push(missing(
                "rows",
                format!(
                    "{}: not family-owned without a recorded justification",
                    row.operation
                ),
            ));
        }
        if row.owner == OperationOwner::Family && row.justification.is_some() {
            mismatches.push(missing(
                "rows",
                format!("{}: family-owned but records a justification", row.operation),
            ));
        }
        if row.disposition == "stubbed-unimplemented" && row.stub_target.is_none() {
            mismatches.push(missing(
                "dispositions",
                format!(
                    "{}: a reserved stub without a deferral marker",
                    row.operation
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
        if let Some(join) = row.audit_join {
            if join.is_empty() {
                mismatches.push(missing(
                    "audit-join",
                    format!("{}: declares an empty join", row.operation),
                ));
            }
            for field in join {
                if !row.payload_required.contains(field) {
                    mismatches.push(missing(
                        "audit-join",
                        format!(
                            "{field}: {} joins on a field its payload does not require",
                            row.operation
                        ),
                    ));
                }
            }
        }
    }
    for operation in operations.iter() {
        if !views.authz.contains(operation) {
            mismatches.push(missing(
                "authz",
                format!("{operation}: committed row, no authorization row"),
            ));
        }
    }
    // The wire enum and the rows are two independent declarations, so the
    // gate compares them as sets: a variant no row inherits and a row
    // inheriting a variant the enum does not declare both fail.
    let enum_variants: BTreeSet<&str> = views.wire.iter().copied().collect();
    let row_variants: BTreeSet<&str> = rows.iter().filter_map(|row| row.wire_variant).collect();
    for name in row_variants.difference(&enum_variants) {
        mismatches.push(missing(
            "wire",
            format!("{name}: a row inherits it, the wire enum does not declare it"),
        ));
    }
    for name in enum_variants.difference(&row_variants) {
        mismatches.push(missing(
            "wire",
            format!("{name}: the wire enum declares it, no committed row declares it"),
        ));
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
            wire: WIRE_VARIANTS.to_vec(),
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
    fn every_reserved_stub_names_a_closed_deferral_marker() {
        let stubs = BROKER_OPERATION_CATALOG
            .iter()
            .filter(|row| row.disposition == "stubbed-unimplemented")
            .count();
        assert!(stubs > 0, "the triage records reserved operations");
        for row in BROKER_OPERATION_CATALOG {
            assert_eq!(
                stub_target(row.operation).is_some(),
                row.disposition == "stubbed-unimplemented",
                "{}: the deferral marker and the disposition disagree",
                row.operation
            );
        }
    }

    #[test]
    fn a_stub_without_a_deferral_marker_fails_the_gate() {
        let mut rows = BROKER_OPERATION_CATALOG.to_vec();
        let stub = rows
            .iter_mut()
            .find(|row| row.disposition == "stubbed-unimplemented")
            .expect("the catalog reserves at least one operation");
        stub.stub_target = None;
        let mismatches = audit(&rows, &live_views());
        assert!(
            mismatches
                .iter()
                .any(|mismatch| mismatch.view == "dispositions"),
            "a stub with no deferral marker must fail: {mismatches:?}"
        );
    }

    #[test]
    fn every_ownership_class_a_row_carries_is_declared() {
        // Retiring the callerless transport concerns emptied the
        // transport-excluded class, so owning rows is no longer what every
        // class does. What must hold is that no row carries a class outside
        // the declared vocabulary, and that a class still in use owns rows -
        // an owner a generator invents, or an in-use class that silently
        // empties, both fail here.
        let declared = [
            OperationOwner::Family,
            OperationOwner::BrokerGeneric,
            OperationOwner::TransportExcluded,
        ];
        for row in BROKER_OPERATION_CATALOG {
            assert!(
                declared.contains(&row.owner),
                "{} is not a declared ownership class",
                row.owner.as_str()
            );
        }
        for owner in [OperationOwner::Family, OperationOwner::BrokerGeneric] {
            assert!(
                BROKER_OPERATION_CATALOG
                    .iter()
                    .any(|row| row.owner == owner),
                "{} is in use but owns no committed row",
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
    fn wire_row_resolves_every_variant_it_names() {
        let (request, name) = (
            d2b_contracts_broker::broker_wire::BrokerRequest::PollChildReaped,
            "PollChildReaped",
        );
        assert_eq!(wire_variant_name(&request), name);
        assert_eq!(wire_row(&request).map(|row| row.operation), Some(name));
    }

    #[test]
    fn a_wire_variant_without_a_row_fails_the_gate() {
        // The wire enum grows and the rows do not: the new variant has no
        // committed row to resolve, which is the axis the row-derived views
        // cannot see.
        let mut views = live_views();
        views.wire.push("NoSuchVariant");
        let mismatches = audit(BROKER_OPERATION_CATALOG, &views);
        assert!(
            mismatches
                .iter()
                .any(|mismatch| mismatch.view == "wire"
                    && mismatch.detail.contains("NoSuchVariant")
                    && mismatch.detail.contains("no committed row")),
            "a wire variant with no committed row must fail: {mismatches:?}"
        );

        let rows: Vec<BrokerOperationRow> = BROKER_OPERATION_CATALOG
            .iter()
            .copied()
            .filter(|row| row.operation != "Hello")
            .collect();
        let mismatches = audit(&rows, &live_views());
        assert!(
            mismatches
                .iter()
                .any(|mismatch| mismatch.view == "wire"
                    && mismatch.detail.contains("Hello")
                    && mismatch.detail.contains("no committed row")),
            "a wire variant with no committed row must fail: {mismatches:?}"
        );
    }

    #[test]
    fn a_row_without_an_owner_or_a_recorded_justification_fails_the_gate() {
        // Every committed row is either provider-owned or explicitly
        // broker/transport-owned with the reason recorded: a row that drops
        // the reason is a row the broker commits without an owner the
        // operator can read.
        let mut rows = BROKER_OPERATION_CATALOG.to_vec();
        let non_provider = rows
            .iter_mut()
            .find(|row| row.owner != OperationOwner::Family)
            .expect("a committed row the broker or a transport concern owns");
        let operation = non_provider.operation;
        assert!(non_provider.justification.is_some());
        non_provider.justification = None;
        let mismatches = audit(&rows, &live_views());
        assert!(
            mismatches.iter().any(|mismatch| mismatch.view == "rows"
                && mismatch.detail.contains(operation)
                && mismatch.detail.contains("without a recorded justification")),
            "a non-provider row without a justification must fail: {mismatches:?}"
        );
    }

    #[test]
    fn a_row_inheriting_an_undeclared_wire_variant_fails_the_gate() {
        let mut rows = BROKER_OPERATION_CATALOG.to_vec();
        let named = rows
            .iter_mut()
            .find(|row| row.wire_variant == Some("Hello"))
            .expect("a committed row inherits the Hello variant");
        named.wire_variant = Some("NoSuchVariant");
        let mismatches = audit(&rows, &live_views());
        assert!(
            mismatches.iter().any(|mismatch| mismatch.view == "wire"
                && mismatch.detail.contains("NoSuchVariant")
                && mismatch.detail.contains("the wire enum does not declare it")),
            "a row inheriting an undeclared variant must fail: {mismatches:?}"
        );
    }

    #[test]
    fn a_join_over_an_unrequired_field_fails_the_gate() {
        let mut rows = BROKER_OPERATION_CATALOG.to_vec();
        rows[0].audit_join = Some(&["absent"]);
        let mismatches = audit(&rows, &live_views());
        assert!(
            mismatches
                .iter()
                .any(|mismatch| mismatch.view == "audit-join" && mismatch.detail.contains("absent")),
            "a join over a field the payload does not require must fail: {mismatches:?}"
        );
        rows[0].audit_join = Some(&[]);
        let mismatches = audit(&rows, &live_views());
        assert!(
            mismatches
                .iter()
                .any(|mismatch| mismatch.view == "audit-join"
                    && mismatch.detail.contains("empty join")),
            "an empty join must fail: {mismatches:?}"
        );
    }
}
