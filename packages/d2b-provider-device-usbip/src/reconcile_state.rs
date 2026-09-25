//! Provider-owned USBIP degraded-reason vocabulary.
//!
//! The daemon projects these closed reasons into the probe and status surfaces;
//! the module adds no broker or public wire operation. Host carrier, flow, and
//! sysfs reconciliation belong to the Provider's live lifecycle path instead.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

/// Maximum accepted lifecycle/correlation-id length for one reconcile attempt.
pub const USBIP_RECONCILE_CORRELATION_ID_MAX_LEN: usize = 48;

fn looks_like_trace_id(value: &str) -> bool {
    matches!(value.len(), 16 | 32) && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn usbip_vm_source_shape_is_valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && value
            .bytes()
            .enumerate()
            .all(|(idx, b)| b.is_ascii_lowercase() || b.is_ascii_digit() || (idx > 0 && b == b'-'))
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && !looks_like_trace_id(value)
}

fn project_usbip_vm_label(value: &str) -> &str {
    if usbip_vm_source_shape_is_valid(value) {
        value
    } else {
        "other"
    }
}

fn deserialize_optional_usbip_event_source_vm<'de, D>(
    deserializer: D,
) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    if let Some(vm) = value.as_deref()
        && !usbip_vm_source_shape_is_valid(vm)
    {
        return Err(D::Error::custom("invalid USB event source VM shape"));
    }
    Ok(value)
}
/// Policy failure detected before attempting host mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipPolicyFailure {
    /// USBIP support is disabled for this VM or environment.
    FeatureDisabled,
    /// Bundle has no USBIP bind/firewall intent for this busid.
    MissingBundleIntent,
    /// Device is not declared for the requested VM.
    DeviceNotDeclaredForVm,
    /// Device is not declared for the requested environment.
    DeviceNotDeclaredForEnv,
    /// Observed topology does not match the declared physical identity.
    TopologyMismatch,
    /// More than one physical device matches the declaration.
    AmbiguousPhysicalMatch,
    /// Caller is not authorized to mutate this claim.
    AuthorizationDenied,
}

/// Why a reconciliation row is degraded instead of converged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipDegradedReason {
    /// One or more policy checks failed.
    PolicyFailed(UsbipPolicyFailure),
    /// A desired device was not present at probe time.
    DeviceDepartedBeforeClaim,
    /// Device disappeared after the daemon/broker acquired the lock.
    DeviceDepartedAfterLock,
    /// Device disappeared during bind/import.
    DeviceDepartedDuringMutation,
    /// A device reappeared at the same logical busid with a different topology.
    DeviceReappearedWithDifferentTopology,
    /// Persisted lock is held by a different owner.
    LockHeldByOtherOwner,
    /// Persisted lock claim is stale or corrupt.
    InvalidPersistedLockClaim,
    /// Host carrier/backend is missing or unavailable.
    CarrierUnavailable,
    /// Host kernel bind is missing or points at an unexpected driver.
    HostBindUnavailable,
    /// Per-env proxy is missing, stale, or failed.
    ProxyUnavailable,
    /// Guest import has not converged.
    GuestImportUnavailable,
    /// Host has stale state for an undeclared/releasing claim.
    StaleHostState,
    /// Guest has stale state for an undeclared/releasing claim.
    StaleGuestState,
    /// Probe was incomplete; retry before mutating.
    ProbeIncomplete,
}

/// Closed public/status code for a degraded USB row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipDegradedReasonCode {
    /// One or more policy checks failed.
    PolicyFailed,
    /// A desired device was not present at probe time.
    DeviceDepartedBeforeClaim,
    /// Device disappeared after the daemon/broker acquired the lock.
    DeviceDepartedAfterLock,
    /// Device disappeared while host or guest state was changing.
    DeviceDepartedDuringMutation,
    /// A different device appeared at the expected location.
    DeviceReappearedWithDifferentTopology,
    /// Another owner currently holds the claim.
    LockHeldByOtherOwner,
    /// The broker-mediated claim is missing, stale, or invalid.
    InvalidPersistedLockClaim,
    /// The host USBIP carrier or backend is unavailable.
    CarrierUnavailable,
    /// The host device is not bound for USBIP export.
    HostBindUnavailable,
    /// The per-environment USBIP proxy is unavailable.
    ProxyUnavailable,
    /// The guest USBIP import has not converged.
    GuestImportUnavailable,
    /// Host USBIP state remains after the claim was removed.
    StaleHostState,
    /// Guest USBIP state remains after the claim was removed.
    StaleGuestState,
    /// Probing did not produce a reconciliation-safe identity.
    ProbeIncomplete,
}

impl UsbipDegradedReasonCode {
    /// Return the stable telemetry label.
    pub const fn telemetry_label(self) -> &'static str {
        match self {
            Self::PolicyFailed => "policy-failed",
            Self::DeviceDepartedBeforeClaim => "device-departed-before-claim",
            Self::DeviceDepartedAfterLock => "device-departed-after-lock",
            Self::DeviceDepartedDuringMutation => "device-departed-during-mutation",
            Self::DeviceReappearedWithDifferentTopology => "device-reappeared-different-topology",
            Self::LockHeldByOtherOwner => "lock-held-by-other-owner",
            Self::InvalidPersistedLockClaim => "invalid-persisted-lock-claim",
            Self::CarrierUnavailable => "carrier-unavailable",
            Self::HostBindUnavailable => "host-bind-unavailable",
            Self::ProxyUnavailable => "proxy-unavailable",
            Self::GuestImportUnavailable => "guest-import-unavailable",
            Self::StaleHostState => "stale-host-state",
            Self::StaleGuestState => "stale-guest-state",
            Self::ProbeIncomplete => "probe-incomplete",
        }
    }
}

impl UsbipPolicyFailure {
    /// Return the stable telemetry label.
    pub const fn telemetry_label(&self) -> &'static str {
        match self {
            Self::FeatureDisabled => "feature-disabled",
            Self::MissingBundleIntent => "missing-bundle-intent",
            Self::DeviceNotDeclaredForVm => "device-not-declared-for-vm",
            Self::DeviceNotDeclaredForEnv => "device-not-declared-for-env",
            Self::TopologyMismatch => "topology-mismatch",
            Self::AmbiguousPhysicalMatch => "ambiguous-physical-match",
            Self::AuthorizationDenied => "authorization-denied",
        }
    }
}

/// Bounded telemetry/log labels projected from a degraded reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbipTelemetryLabels {
    /// Stable degraded-reason label.
    pub reason: &'static str,
    /// Stable policy-failure label, or `"none"`.
    pub policy: &'static str,
}

/// Closed USB event type used for dedupe/rate-limit buckets and metric label
/// projection. Raw operation names, trace IDs, and process IDs must never create
/// additional buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipEventType {
    /// A reconciliation row became or stayed degraded.
    Degraded,
    /// A reconciliation state transition occurred.
    StateTransition,
    /// A repeated degraded summary was suppressed.
    SuppressedSummary,
    /// Any other bounded event class.
    Other,
}

impl UsbipEventType {
    /// Return the stable telemetry label.
    pub const fn telemetry_label(self) -> &'static str {
        match self {
            Self::Degraded => "degraded",
            Self::StateTransition => "state-transition",
            Self::SuppressedSummary => "suppressed-summary",
            Self::Other => "other",
        }
    }
}

/// Bounded source component for USB event buckets. Partition by VM/component
/// class, never by process ID, bus ID, sysfs path, trace ID, or serial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsbipEventSourceKind {
    /// The guest VM the claim serves.
    Vm,
    /// The host carrier reporting USB topology.
    Host,
    /// The guest-side USBIP import surface.
    Guest,
    /// The USBIP claim broker.
    Broker,
    /// The reconciliation loop itself.
    Reconciler,
    /// Any other bounded source class.
    Other,
}

impl UsbipEventSourceKind {
    /// Return the stable telemetry label.
    pub const fn telemetry_label(self) -> &'static str {
        match self {
            Self::Vm => "vm",
            Self::Host => "host",
            Self::Guest => "guest",
            Self::Broker => "broker",
            Self::Reconciler => "reconciler",
            Self::Other => "other",
        }
    }
}

/// Bounded source projection for USB structured events and dedupe buckets.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipEventSource {
    /// Bounded source kind.
    pub kind: UsbipEventSourceKind,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_optional_usbip_event_source_vm"
    )]
    /// Projected, bounded VM label when the source is a VM.
    pub vm: Option<String>,
}

impl UsbipEventSource {
    /// Build a VM-bucketed event source from an arbitrary VM label.
    pub fn vm(vm: impl AsRef<str>) -> Self {
        Self {
            kind: UsbipEventSourceKind::Vm,
            vm: Some(project_usbip_vm_label(vm.as_ref()).to_owned()),
        }
    }

    /// Build a component-bucketed event source without a VM label.
    pub fn component(kind: UsbipEventSourceKind) -> Self {
        Self { kind, vm: None }
    }

    /// Project the bounded telemetry label pair.
    pub fn telemetry_labels(&self) -> UsbipEventSourceLabels<'_> {
        UsbipEventSourceLabels {
            source_kind: self.kind.telemetry_label(),
            vm: self.metric_vm_label(),
        }
    }

    fn metric_vm_label(&self) -> &'static str {
        match self.vm.as_deref() {
            None => "none",
            Some("other") => "other",
            Some(_) => "present",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Bounded telemetry label pair for one event source.
pub struct UsbipEventSourceLabels<'a> {
    /// Stable source-kind label.
    pub source_kind: &'static str,
    /// Projected VM label, or `"none"`.
    pub vm: &'a str,
}

/// Bounded correlation identifier for one reconcile attempt. This is safe in
/// structured USB events but must never be projected into metric labels.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UsbipReconcileCorrelationId(String);

impl UsbipReconcileCorrelationId {
    /// Validate abounded correlation id, returning `None` when it is empty,
    /// overlong, contains an unsupported character, or looks like a trace id.
    pub fn new(value: impl AsRef<str>) -> Option<Self> {
        let value = value.as_ref();
        let valid = !value.is_empty()
            && value.len() <= USBIP_RECONCILE_CORRELATION_ID_MAX_LEN
            && value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
            && !looks_like_trace_id(value);
        valid.then(|| Self(value.to_owned()))
    }

    /// Borrow the raw correlation id.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for UsbipReconcileCorrelationId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for UsbipReconcileCorrelationId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(&value).ok_or_else(|| D::Error::custom("invalid USB reconcile correlation id"))
    }
}

/// Bucketed lifecycle/correlation context attached to structured USB events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipReconcileAttemptContext {
    /// Correlation id for this reconcile attempt.
    pub correlation_id: UsbipReconcileCorrelationId,
}

impl UsbipDegradedReason {
    /// Project the closed status code.
    pub fn code(&self) -> UsbipDegradedReasonCode {
        match self {
            Self::PolicyFailed(_) => UsbipDegradedReasonCode::PolicyFailed,
            Self::DeviceDepartedBeforeClaim => UsbipDegradedReasonCode::DeviceDepartedBeforeClaim,
            Self::DeviceDepartedAfterLock => UsbipDegradedReasonCode::DeviceDepartedAfterLock,
            Self::DeviceDepartedDuringMutation => {
                UsbipDegradedReasonCode::DeviceDepartedDuringMutation
            }
            Self::DeviceReappearedWithDifferentTopology => {
                UsbipDegradedReasonCode::DeviceReappearedWithDifferentTopology
            }
            Self::LockHeldByOtherOwner => UsbipDegradedReasonCode::LockHeldByOtherOwner,
            Self::InvalidPersistedLockClaim => UsbipDegradedReasonCode::InvalidPersistedLockClaim,
            Self::CarrierUnavailable => UsbipDegradedReasonCode::CarrierUnavailable,
            Self::HostBindUnavailable => UsbipDegradedReasonCode::HostBindUnavailable,
            Self::ProxyUnavailable => UsbipDegradedReasonCode::ProxyUnavailable,
            Self::GuestImportUnavailable => UsbipDegradedReasonCode::GuestImportUnavailable,
            Self::StaleHostState => UsbipDegradedReasonCode::StaleHostState,
            Self::StaleGuestState => UsbipDegradedReasonCode::StaleGuestState,
            Self::ProbeIncomplete => UsbipDegradedReasonCode::ProbeIncomplete,
        }
    }

    /// Project the bounded telemetry label pair.
    pub fn telemetry_labels(&self) -> UsbipTelemetryLabels {
        UsbipTelemetryLabels {
            reason: self.code().telemetry_label(),
            policy: match self {
                Self::PolicyFailed(policy) => policy.telemetry_label(),
                _ => "none",
            },
        }
    }

    /// Return the bounded human summary.
    pub fn summary(&self) -> &'static str {
        match self.code() {
            UsbipDegradedReasonCode::PolicyFailed => "USB policy does not allow this claim",
            UsbipDegradedReasonCode::DeviceDepartedBeforeClaim => {
                "the USB device was not present before claiming"
            }
            UsbipDegradedReasonCode::DeviceDepartedAfterLock => {
                "the USB device disappeared after broker claim acquisition"
            }
            UsbipDegradedReasonCode::DeviceDepartedDuringMutation => {
                "the USB device disappeared while host or guest state was changing"
            }
            UsbipDegradedReasonCode::DeviceReappearedWithDifferentTopology => {
                "a different USB device appeared at the expected location"
            }
            UsbipDegradedReasonCode::LockHeldByOtherOwner => {
                "another owner currently holds the USB claim"
            }
            UsbipDegradedReasonCode::InvalidPersistedLockClaim => {
                "the broker-mediated USB claim is missing, stale, or invalid"
            }
            UsbipDegradedReasonCode::CarrierUnavailable => {
                "the host USBIP carrier or backend is unavailable"
            }
            UsbipDegradedReasonCode::HostBindUnavailable => {
                "the host USB device is not bound for USBIP export"
            }
            UsbipDegradedReasonCode::ProxyUnavailable => {
                "the per-environment USBIP proxy is unavailable"
            }
            UsbipDegradedReasonCode::GuestImportUnavailable => {
                "the guest USBIP import has not converged"
            }
            UsbipDegradedReasonCode::StaleHostState => {
                "host USBIP state remains after the claim was removed"
            }
            UsbipDegradedReasonCode::StaleGuestState => {
                "guest USBIP state remains after the claim was removed"
            }
            UsbipDegradedReasonCode::ProbeIncomplete => {
                "USB probing did not produce a reconciliation-safe identity"
            }
        }
    }

    /// Return the bounded remediation guidance.
    pub fn remediation(&self) -> &'static str {
        match self.code() {
            UsbipDegradedReasonCode::PolicyFailed => {
                "fix the USBIP declaration or caller authorization, rebuild the bundle, and retry the USB lifecycle verb"
            }
            UsbipDegradedReasonCode::DeviceDepartedBeforeClaim
            | UsbipDegradedReasonCode::DeviceDepartedAfterLock
            | UsbipDegradedReasonCode::DeviceDepartedDuringMutation => {
                "reconnect the physical device, wait for the host to observe it, then rerun the USB probe or lifecycle verb"
            }
            UsbipDegradedReasonCode::DeviceReappearedWithDifferentTopology => {
                "verify the physical device identity, update the declaration if intentional, and retry after the probe is stable"
            }
            UsbipDegradedReasonCode::LockHeldByOtherOwner => {
                "stop or detach the owning VM/environment before retrying this USB claim"
            }
            UsbipDegradedReasonCode::InvalidPersistedLockClaim => {
                "run the USB reconciler after confirming no active owner still uses the device; remove only broker-owned stale claim state"
            }
            UsbipDegradedReasonCode::CarrierUnavailable => {
                "ensure the usbip-host kernel module and per-environment backend are available, then retry"
            }
            UsbipDegradedReasonCode::HostBindUnavailable => {
                "for attach/start, rerun the USB lifecycle verb so the broker can bind the device to usbip-host; for detach/stop cleanup refused before unbind, stop the VM so USBIP streams drain or retry once a single targeted stream can be proven"
            }
            UsbipDegradedReasonCode::ProxyUnavailable => {
                "restart or reconcile the per-environment USBIP proxy before guest attach"
            }
            UsbipDegradedReasonCode::GuestImportUnavailable => {
                "check component-session USBIP capability and retry the attach after host export is healthy"
            }
            UsbipDegradedReasonCode::StaleHostState => {
                "rerun USB detach/reconcile to drain host export and proxy state for the removed claim"
            }
            UsbipDegradedReasonCode::StaleGuestState => {
                "rerun USB detach/reconcile so the target-local Process removes stale imported-device state"
            }
            UsbipDegradedReasonCode::ProbeIncomplete => {
                "retry the USB probe; if it repeats, verify the declaration has a stable physical selector"
            }
        }
    }

    /// Project the structured, redacted public reason detail.
    pub fn to_public_reason(&self) -> UsbipPublicDegradedReason {
        UsbipPublicDegradedReason {
            code: self.code(),
            policy_failure: match self {
                Self::PolicyFailed(policy) => Some(*policy),
                _ => None,
            },
            summary: self.summary().to_owned(),
            remediation: self.remediation().to_owned(),
        }
    }
}

/// Structured, redacted status/probe reason detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UsbipPublicDegradedReason {
    /// Closed status code.
    pub code: UsbipDegradedReasonCode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Policy failure detail when the code is policy-failed.
    pub policy_failure: Option<UsbipPolicyFailure>,
    /// Bounded human summary.
    pub summary: String,
    /// Bounded remediation guidance.
    pub remediation: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_source_round_trips_a_vm_source_payload() {
        let payload = r#"{"kind":"vm","vm":"workload-a"}"#;
        let source: UsbipEventSource = serde_json::from_str(payload).unwrap();
        assert_eq!(source, UsbipEventSource::vm("workload-a"));
        assert_eq!(serde_json::to_string(&source).unwrap(), payload);
    }

    #[test]
    fn event_source_round_trips_a_component_source_payload_without_vm() {
        let payload = r#"{"kind":"host"}"#;
        let source: UsbipEventSource = serde_json::from_str(payload).unwrap();
        assert_eq!(source, UsbipEventSource::component(UsbipEventSourceKind::Host));
        assert_eq!(serde_json::to_string(&source).unwrap(), payload);
    }

    #[test]
    fn reconcile_attempt_context_round_trips_a_correlation_id_payload() {
        let payload = r#"{"correlationId":"reconcile-2026-09-25-01"}"#;
        let context: UsbipReconcileAttemptContext = serde_json::from_str(payload).unwrap();
        assert_eq!(
            context,
            UsbipReconcileAttemptContext {
                correlation_id: UsbipReconcileCorrelationId::new("reconcile-2026-09-25-01").unwrap(),
            }
        );
        assert_eq!(serde_json::to_string(&context).unwrap(), payload);
    }

    #[test]
    fn public_degraded_reason_round_trips_a_policy_failure_payload() {
        let payload = r#"{"code":"policy-failed","policyFailure":"feature-disabled","summary":"USB policy does not allow this claim","remediation":"fix the USBIP declaration or caller authorization, rebuild the bundle, and retry the USB lifecycle verb"}"#;
        let reason: UsbipPublicDegradedReason = serde_json::from_str(payload).unwrap();
        assert_eq!(
            reason,
            UsbipDegradedReason::PolicyFailed(UsbipPolicyFailure::FeatureDisabled)
                .to_public_reason()
        );
        assert_eq!(serde_json::to_string(&reason).unwrap(), payload);
    }

    #[test]
    fn public_degraded_reason_round_trips_a_non_policy_payload_without_policy_failure() {
        let payload = r#"{"code":"probe-incomplete","summary":"USB probing did not produce a reconciliation-safe identity","remediation":"retry the USB probe; if it repeats, verify the declaration has a stable physical selector"}"#;
        let reason: UsbipPublicDegradedReason = serde_json::from_str(payload).unwrap();
        assert_eq!(
            reason,
            UsbipDegradedReason::ProbeIncomplete.to_public_reason()
        );
        assert_eq!(serde_json::to_string(&reason).unwrap(), payload);
    }
}
