use std::{error::Error, fmt};

use d2b_contracts::{
    v2_identity::{ProviderId, RealmId},
    v2_provider::{
        Fingerprint, Generation, ImplementationId, MAX_SAFE_JSON_INTEGER, OperationBinding, PlanId,
        PrincipalRef, ProviderDescriptor,
    },
};

pub const MAX_CHECK_FINDINGS: usize = 64;
pub const MAX_PLAN_FINDINGS: usize = 32;
pub const MAX_FINDING_DIAGNOSTICS: usize = 8;
pub const MAX_REPORT_DIAGNOSTICS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostModelError {
    BoundExceeded,
    DuplicateEntry,
    InvalidBinding,
    InvalidTransition,
    InvalidTime,
}

impl fmt::Display for HostModelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::BoundExceeded => "host substrate model bound exceeded",
            Self::DuplicateEntry => "host substrate model contains a duplicate entry",
            Self::InvalidBinding => "host substrate model binding is invalid",
            Self::InvalidTransition => "host substrate model transition is invalid",
            Self::InvalidTime => "host substrate model time range is invalid",
        })
    }
}

impl Error for HostModelError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HostSubstrateKind {
    NixOs,
    GenericLinux,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostCheckProfile {
    NixOsFullHost,
    GenericLinuxFullHost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostSubstrateConfiguration {
    substrate: HostSubstrateKind,
    check_profile: HostCheckProfile,
}

impl HostSubstrateConfiguration {
    pub const fn nixos() -> Self {
        Self {
            substrate: HostSubstrateKind::NixOs,
            check_profile: HostCheckProfile::NixOsFullHost,
        }
    }

    pub const fn generic_linux() -> Self {
        Self {
            substrate: HostSubstrateKind::GenericLinux,
            check_profile: HostCheckProfile::GenericLinuxFullHost,
        }
    }

    pub const fn substrate(self) -> HostSubstrateKind {
        self.substrate
    }

    pub const fn check_profile(self) -> HostCheckProfile {
        self.check_profile
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct HostDescriptorBinding {
    pub provider_id: ProviderId,
    pub implementation_id: ImplementationId,
    pub registry_generation: Generation,
    pub configuration_fingerprint: Fingerprint,
    pub configured_scope_digest: Fingerprint,
}

impl fmt::Debug for HostDescriptorBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostDescriptorBinding")
            .field("implementation_id", &self.implementation_id)
            .field("registry_generation", &self.registry_generation)
            .finish_non_exhaustive()
    }
}

impl HostDescriptorBinding {
    pub fn from_descriptor(descriptor: &ProviderDescriptor) -> Self {
        Self {
            provider_id: descriptor.provider_id.clone(),
            implementation_id: descriptor.implementation_id.clone(),
            registry_generation: descriptor.registry_generation,
            configuration_fingerprint: descriptor.configuration_schema_fingerprint.clone(),
            configured_scope_digest: descriptor.configured_scope_digest.clone(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct HostOperationOwner {
    pub realm_id: RealmId,
    pub principal: PrincipalRef,
}

impl fmt::Debug for HostOperationOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostOperationOwner")
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HostCapability {
    CgroupV2,
    UserNamespaces,
    VhostAcceleration,
    DeviceAccess,
}

impl HostCapability {
    pub const ALL: [Self; 4] = [
        Self::CgroupV2,
        Self::UserNamespaces,
        Self::VhostAcceleration,
        Self::DeviceAccess,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HostEvidenceSource {
    KernelApiProbe,
    KernelModuleProbe,
    DeviceAccessProbe,
    DelegationProbe,
    DeclarativeConfiguration,
    DaemonPreflight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostSupportStatus {
    Confirmed(HostEvidenceSource),
    Unknown,
    NotApplicable,
    Unsupported,
}

impl HostSupportStatus {
    pub const fn is_confirmed(self) -> bool {
        matches!(self, Self::Confirmed(_))
    }

    pub const fn is_explicit(self) -> bool {
        !matches!(self, Self::Unknown)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostSupportEntry {
    pub capability: HostCapability,
    pub status: HostSupportStatus,
}

impl HostSupportEntry {
    pub const fn new(capability: HostCapability, status: HostSupportStatus) -> Self {
        Self { capability, status }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HostSupportEvidence {
    entries: Vec<HostSupportEntry>,
}

impl HostSupportEvidence {
    pub fn new(mut entries: Vec<HostSupportEntry>) -> Result<Self, HostModelError> {
        if entries.len() > HostCapability::ALL.len() {
            return Err(HostModelError::BoundExceeded);
        }
        entries.sort_by_key(|entry| entry.capability);
        if entries
            .windows(2)
            .any(|pair| pair[0].capability == pair[1].capability)
        {
            return Err(HostModelError::DuplicateEntry);
        }
        Ok(Self { entries })
    }

    pub fn status(&self, capability: HostCapability) -> HostSupportStatus {
        self.entries
            .iter()
            .find(|entry| entry.capability == capability)
            .map_or(HostSupportStatus::Unknown, |entry| entry.status)
    }

    pub fn entries(&self) -> &[HostSupportEntry] {
        &self.entries
    }

    pub fn confirmed_capabilities(&self) -> Vec<HostCapability> {
        HostCapability::ALL
            .into_iter()
            .filter(|capability| self.status(*capability).is_confirmed())
            .collect()
    }

    pub fn has_unknown(&self) -> bool {
        HostCapability::ALL
            .into_iter()
            .any(|capability| self.status(capability) == HostSupportStatus::Unknown)
    }

    pub fn has_unsupported(&self) -> bool {
        HostCapability::ALL
            .into_iter()
            .any(|capability| self.status(capability) == HostSupportStatus::Unsupported)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HostCheckKind {
    KernelVersion,
    CpuVirtualization,
    CgroupV2,
    UserNamespaces,
    KernelModules,
    DeviceAccess,
    NetworkPolicy,
    SysctlPolicy,
    RunnerParity,
    StateOwnership,
    HostIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HostKernelModule {
    KvmIntel,
    KvmAmd,
    VhostNet,
    Tun,
    VirtioFs,
    BridgeNetfilter,
    Udmabuf,
    UsbipHost,
    TpmVtpmProxy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HostFindingKind {
    CheckUnavailable(HostCheckKind),
    CheckFailed(HostCheckKind),
    CapabilityUnsupported(HostCapability),
    KernelModuleMissing(HostKernelModule),
    ConfigurationDrift(HostCheckKind),
    OwnershipDrift,
    FirewallConflict,
    HostIdentityDrift,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostFindingSeverity {
    Advisory,
    Degraded,
    Blocking,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostRemediationClass {
    DaemonAuthorized,
    NixOsConfiguration,
    OperatorAction,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HostDiagnostic {
    EvidenceMissing,
    ProbeUnavailable,
    VersionTooOld,
    RequiredComponentMissing,
    ConfigurationMismatch,
    OwnershipMismatch,
    ConflictingOwner,
    AuthorizationRequired,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostFinding {
    kind: HostFindingKind,
    severity: HostFindingSeverity,
    remediation: HostRemediationClass,
    diagnostics: Vec<HostDiagnostic>,
    affected_count: u16,
}

impl HostFinding {
    pub fn new(
        kind: HostFindingKind,
        severity: HostFindingSeverity,
        remediation: HostRemediationClass,
        mut diagnostics: Vec<HostDiagnostic>,
        affected_count: u16,
    ) -> Result<Self, HostModelError> {
        if diagnostics.len() > MAX_FINDING_DIAGNOSTICS {
            return Err(HostModelError::BoundExceeded);
        }
        if affected_count == 0 {
            return Err(HostModelError::InvalidBinding);
        }
        diagnostics.sort_unstable();
        if diagnostics.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(HostModelError::DuplicateEntry);
        }
        Ok(Self {
            kind,
            severity,
            remediation,
            diagnostics,
            affected_count,
        })
    }

    pub const fn kind(&self) -> HostFindingKind {
        self.kind
    }

    pub const fn severity(&self) -> HostFindingSeverity {
        self.severity
    }

    pub const fn remediation(&self) -> HostRemediationClass {
        self.remediation
    }

    pub fn diagnostics(&self) -> &[HostDiagnostic] {
        &self.diagnostics
    }

    pub const fn affected_count(&self) -> u16 {
        self.affected_count
    }

    fn validate(&self) -> Result<(), HostModelError> {
        if self.diagnostics.len() > MAX_FINDING_DIAGNOSTICS {
            return Err(HostModelError::BoundExceeded);
        }
        if self.affected_count == 0 {
            return Err(HostModelError::InvalidBinding);
        }
        if self.diagnostics.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(HostModelError::DuplicateEntry);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostCheckSummary {
    pub advisory: u16,
    pub degraded: u16,
    pub blocking: u16,
}

#[derive(Clone, PartialEq, Eq)]
pub struct HostCheckReport {
    configuration: HostSubstrateConfiguration,
    descriptor: HostDescriptorBinding,
    owner: HostOperationOwner,
    operation: OperationBinding,
    observed_at_unix_ms: u64,
    report_fingerprint: Fingerprint,
    support: HostSupportEvidence,
    findings: Vec<HostFinding>,
}

impl fmt::Debug for HostCheckReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostCheckReport")
            .field("configuration", &self.configuration)
            .field("descriptor", &self.descriptor)
            .field("observed_at_unix_ms", &self.observed_at_unix_ms)
            .field("support_entries", &self.support.entries().len())
            .field("finding_count", &self.findings.len())
            .finish_non_exhaustive()
    }
}

impl HostCheckReport {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        configuration: HostSubstrateConfiguration,
        descriptor: HostDescriptorBinding,
        owner: HostOperationOwner,
        operation: OperationBinding,
        observed_at_unix_ms: u64,
        report_fingerprint: Fingerprint,
        support: HostSupportEvidence,
        mut findings: Vec<HostFinding>,
    ) -> Result<Self, HostModelError> {
        findings.sort_by_key(|finding| finding.kind);
        let report = Self {
            configuration,
            descriptor,
            owner,
            operation,
            observed_at_unix_ms,
            report_fingerprint,
            support,
            findings,
        };
        report.validate()?;
        Ok(report)
    }

    pub const fn configuration(&self) -> HostSubstrateConfiguration {
        self.configuration
    }

    pub fn descriptor(&self) -> &HostDescriptorBinding {
        &self.descriptor
    }

    pub fn owner(&self) -> &HostOperationOwner {
        &self.owner
    }

    pub fn operation(&self) -> &OperationBinding {
        &self.operation
    }

    pub const fn observed_at_unix_ms(&self) -> u64 {
        self.observed_at_unix_ms
    }

    pub fn report_fingerprint(&self) -> &Fingerprint {
        &self.report_fingerprint
    }

    pub fn support(&self) -> &HostSupportEvidence {
        &self.support
    }

    pub fn findings(&self) -> &[HostFinding] {
        &self.findings
    }

    pub fn summary(&self) -> HostCheckSummary {
        let mut summary = HostCheckSummary {
            advisory: 0,
            degraded: 0,
            blocking: 0,
        };
        for finding in &self.findings {
            let count = match finding.severity {
                HostFindingSeverity::Advisory => &mut summary.advisory,
                HostFindingSeverity::Degraded => &mut summary.degraded,
                HostFindingSeverity::Blocking => &mut summary.blocking,
            };
            *count = count.saturating_add(1);
        }
        summary
    }

    pub fn validate(&self) -> Result<(), HostModelError> {
        if self.observed_at_unix_ms > MAX_SAFE_JSON_INTEGER {
            return Err(HostModelError::InvalidTime);
        }
        validate_findings(&self.findings, MAX_CHECK_FINDINGS, MAX_REPORT_DIAGNOSTICS)
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HostRemediationId(PlanId);

impl fmt::Debug for HostRemediationId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("HostRemediationId")
            .field(&"<redacted>")
            .finish()
    }
}

impl HostRemediationId {
    pub fn parse(value: impl Into<String>) -> Result<Self, HostModelError> {
        PlanId::parse(value)
            .map(Self)
            .map_err(|_| HostModelError::InvalidBinding)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn as_plan_id(&self) -> PlanId {
        self.0.clone()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostRemediationPlanDisposition {
    Authorized,
    NotApplicable,
}

#[derive(Clone, PartialEq, Eq)]
pub struct HostRemediationPlan {
    remediation_id: HostRemediationId,
    disposition: HostRemediationPlanDisposition,
    configuration: HostSubstrateConfiguration,
    descriptor: HostDescriptorBinding,
    owner: HostOperationOwner,
    operation: OperationBinding,
    report_fingerprint: Fingerprint,
    findings: Vec<HostFinding>,
    created_at_unix_ms: u64,
    expires_at_unix_ms: u64,
}

impl fmt::Debug for HostRemediationPlan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostRemediationPlan")
            .field("disposition", &self.disposition)
            .field("configuration", &self.configuration)
            .field("descriptor", &self.descriptor)
            .field("finding_count", &self.findings.len())
            .field("created_at_unix_ms", &self.created_at_unix_ms)
            .field("expires_at_unix_ms", &self.expires_at_unix_ms)
            .finish_non_exhaustive()
    }
}

impl HostRemediationPlan {
    #[allow(clippy::too_many_arguments)]
    pub fn authorized(
        remediation_id: HostRemediationId,
        configuration: HostSubstrateConfiguration,
        descriptor: HostDescriptorBinding,
        owner: HostOperationOwner,
        operation: OperationBinding,
        report_fingerprint: Fingerprint,
        findings: Vec<HostFinding>,
        created_at_unix_ms: u64,
        expires_at_unix_ms: u64,
    ) -> Result<Self, HostModelError> {
        Self::new(
            remediation_id,
            HostRemediationPlanDisposition::Authorized,
            configuration,
            descriptor,
            owner,
            operation,
            report_fingerprint,
            findings,
            created_at_unix_ms,
            expires_at_unix_ms,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn not_applicable(
        remediation_id: HostRemediationId,
        configuration: HostSubstrateConfiguration,
        descriptor: HostDescriptorBinding,
        owner: HostOperationOwner,
        operation: OperationBinding,
        report_fingerprint: Fingerprint,
        findings: Vec<HostFinding>,
        created_at_unix_ms: u64,
        expires_at_unix_ms: u64,
    ) -> Result<Self, HostModelError> {
        Self::new(
            remediation_id,
            HostRemediationPlanDisposition::NotApplicable,
            configuration,
            descriptor,
            owner,
            operation,
            report_fingerprint,
            findings,
            created_at_unix_ms,
            expires_at_unix_ms,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        remediation_id: HostRemediationId,
        disposition: HostRemediationPlanDisposition,
        configuration: HostSubstrateConfiguration,
        descriptor: HostDescriptorBinding,
        owner: HostOperationOwner,
        operation: OperationBinding,
        report_fingerprint: Fingerprint,
        mut findings: Vec<HostFinding>,
        created_at_unix_ms: u64,
        expires_at_unix_ms: u64,
    ) -> Result<Self, HostModelError> {
        findings.sort_by_key(|finding| finding.kind);
        let plan = Self {
            remediation_id,
            disposition,
            configuration,
            descriptor,
            owner,
            operation,
            report_fingerprint,
            findings,
            created_at_unix_ms,
            expires_at_unix_ms,
        };
        plan.validate()?;
        Ok(plan)
    }

    pub fn remediation_id(&self) -> &HostRemediationId {
        &self.remediation_id
    }

    pub const fn disposition(&self) -> HostRemediationPlanDisposition {
        self.disposition
    }

    pub const fn configuration(&self) -> HostSubstrateConfiguration {
        self.configuration
    }

    pub fn descriptor(&self) -> &HostDescriptorBinding {
        &self.descriptor
    }

    pub fn owner(&self) -> &HostOperationOwner {
        &self.owner
    }

    pub fn operation(&self) -> &OperationBinding {
        &self.operation
    }

    pub fn report_fingerprint(&self) -> &Fingerprint {
        &self.report_fingerprint
    }

    pub fn findings(&self) -> &[HostFinding] {
        &self.findings
    }

    pub const fn created_at_unix_ms(&self) -> u64 {
        self.created_at_unix_ms
    }

    pub const fn expires_at_unix_ms(&self) -> u64 {
        self.expires_at_unix_ms
    }

    pub fn validate(&self) -> Result<(), HostModelError> {
        if self.created_at_unix_ms > MAX_SAFE_JSON_INTEGER
            || self.expires_at_unix_ms > MAX_SAFE_JSON_INTEGER
            || self.expires_at_unix_ms <= self.created_at_unix_ms
        {
            return Err(HostModelError::InvalidTime);
        }
        validate_findings(&self.findings, MAX_PLAN_FINDINGS, MAX_REPORT_DIAGNOSTICS)?;
        let has_daemon_remediation = self
            .findings
            .iter()
            .any(|finding| finding.remediation == HostRemediationClass::DaemonAuthorized);
        match self.disposition {
            HostRemediationPlanDisposition::Authorized if !has_daemon_remediation => {
                Err(HostModelError::InvalidTransition)
            }
            HostRemediationPlanDisposition::NotApplicable if has_daemon_remediation => {
                Err(HostModelError::InvalidTransition)
            }
            _ => Ok(()),
        }
    }
}

fn validate_findings(
    findings: &[HostFinding],
    max_findings: usize,
    max_diagnostics: usize,
) -> Result<(), HostModelError> {
    if findings.len() > max_findings {
        return Err(HostModelError::BoundExceeded);
    }
    let mut kinds = Vec::with_capacity(findings.len());
    let mut diagnostic_count = 0usize;
    for finding in findings {
        finding.validate()?;
        kinds.push(finding.kind);
        diagnostic_count = diagnostic_count
            .checked_add(finding.diagnostics.len())
            .ok_or(HostModelError::BoundExceeded)?;
    }
    if diagnostic_count > max_diagnostics {
        return Err(HostModelError::BoundExceeded);
    }
    kinds.sort_unstable();
    if kinds.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(HostModelError::DuplicateEntry);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostApplyOutcome {
    Applied,
    AlreadyApplied,
    NotApplicable,
    CancelledBeforeMutation,
    CompletionAmbiguous,
}

#[derive(Clone, PartialEq, Eq)]
pub struct HostApplyInspection {
    remediation_id: HostRemediationId,
    outcome: HostApplyOutcome,
    observed_at_unix_ms: u64,
}

impl fmt::Debug for HostApplyInspection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostApplyInspection")
            .field("outcome", &self.outcome)
            .field("observed_at_unix_ms", &self.observed_at_unix_ms)
            .finish_non_exhaustive()
    }
}

impl HostApplyInspection {
    pub(crate) fn new(
        remediation_id: HostRemediationId,
        outcome: HostApplyOutcome,
        observed_at_unix_ms: u64,
    ) -> Self {
        Self {
            remediation_id,
            outcome,
            observed_at_unix_ms,
        }
    }

    pub fn remediation_id(&self) -> &HostRemediationId {
        &self.remediation_id
    }

    pub const fn outcome(&self) -> HostApplyOutcome {
        self.outcome
    }

    pub const fn observed_at_unix_ms(&self) -> u64 {
        self.observed_at_unix_ms
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostSubstrateState {
    Unknown,
    Checked,
    RemediationPlanned,
    Ready,
    CompletionAmbiguous,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSubstrateInspection {
    configuration: HostSubstrateConfiguration,
    descriptor: HostDescriptorBinding,
    state: HostSubstrateState,
    report: Option<HostCheckReport>,
    plan: Option<HostRemediationPlan>,
    apply: Option<HostApplyInspection>,
}

impl HostSubstrateInspection {
    pub(crate) fn new(
        configuration: HostSubstrateConfiguration,
        descriptor: HostDescriptorBinding,
        report: Option<HostCheckReport>,
        plan: Option<HostRemediationPlan>,
        apply: Option<HostApplyInspection>,
    ) -> Self {
        let report_is_ready = report.as_ref().is_some_and(|report| {
            !report.support().has_unknown()
                && !report.support().has_unsupported()
                && report
                    .findings()
                    .iter()
                    .all(|finding| finding.severity() == HostFindingSeverity::Advisory)
        });
        let remediation_is_authorized = plan
            .as_ref()
            .is_some_and(|plan| plan.disposition() == HostRemediationPlanDisposition::Authorized);
        let state = match apply.as_ref().map(HostApplyInspection::outcome) {
            Some(HostApplyOutcome::CompletionAmbiguous) => HostSubstrateState::CompletionAmbiguous,
            Some(HostApplyOutcome::Applied | HostApplyOutcome::AlreadyApplied) => {
                HostSubstrateState::Ready
            }
            Some(HostApplyOutcome::CancelledBeforeMutation) => {
                if remediation_is_authorized {
                    HostSubstrateState::RemediationPlanned
                } else if report_is_ready {
                    HostSubstrateState::Ready
                } else {
                    report
                        .as_ref()
                        .map_or(HostSubstrateState::Unknown, |_| HostSubstrateState::Checked)
                }
            }
            Some(HostApplyOutcome::NotApplicable) if report_is_ready => HostSubstrateState::Ready,
            Some(HostApplyOutcome::NotApplicable) if report.is_some() => {
                HostSubstrateState::Checked
            }
            Some(HostApplyOutcome::NotApplicable) => HostSubstrateState::Unknown,
            None if remediation_is_authorized => HostSubstrateState::RemediationPlanned,
            None if report_is_ready => HostSubstrateState::Ready,
            None if report.is_some() => HostSubstrateState::Checked,
            None => HostSubstrateState::Unknown,
        };
        Self {
            configuration,
            descriptor,
            state,
            report,
            plan,
            apply,
        }
    }

    pub const fn configuration(&self) -> HostSubstrateConfiguration {
        self.configuration
    }

    pub fn descriptor(&self) -> &HostDescriptorBinding {
        &self.descriptor
    }

    pub const fn state(&self) -> HostSubstrateState {
        self.state
    }

    pub fn report(&self) -> Option<&HostCheckReport> {
        self.report.as_ref()
    }

    pub fn plan(&self) -> Option<&HostRemediationPlan> {
        self.plan.as_ref()
    }

    pub fn apply(&self) -> Option<&HostApplyInspection> {
        self.apply.as_ref()
    }
}
