//! Host-mediated TPM, USBIP, FIDO, GPU, video, and mediated-device provider.

#![forbid(unsafe_code)]
#![allow(clippy::result_large_err)]

use std::{collections::BTreeMap, fmt, future::Future, sync::Arc, time::Duration};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{BoundedVec, EndpointRole},
    v2_identity::{ProviderId, ProviderType},
    v2_provider::{
        AdoptionRequest, AdoptionState, AuthorizedProviderScope, DeviceProvider, DeviceSelectorId,
        Fingerprint, Generation, HandleId, ImplementationId, MAX_PROVIDER_REGISTRY_ENTRIES,
        MutationReceipt, MutationState, ObservationReason, ObservedLifecycleState,
        OperationBinding, PROVIDER_SCHEMA_VERSION, PlanId, PlannedResourceClass, Provider,
        ProviderCallContext, ProviderCapability, ProviderCapabilitySet, ProviderContractError,
        ProviderDescriptor, ProviderFactoryKey, ProviderFailure, ProviderFailureKind,
        ProviderFuture, ProviderHandle, ProviderHandleKind, ProviderHealth, ProviderHealthReason,
        ProviderHealthState, ProviderMethod, ProviderObservation, ProviderOperationContext,
        ProviderOperationInput, ProviderOperationRequest, ProviderPlacement, ProviderPlan,
        ProviderRemediation, ProviderTarget, RetryClass,
    },
};
use d2b_host::{
    gpu_argv::{GpuArgvInput, generate_gpu_argv},
    swtpm_argv::{
        SwtpmArgvInput, SwtpmIoctlFlushInput, generate_swtpm_argv, generate_swtpm_ioctl_flush_argv,
    },
    usbip_argv::{UsbipArgvInput, UsbipSubcommand, generate_usbip_argv},
    video_argv::{VideoArgvInput, generate_video_argv},
};
use d2b_provider::{
    FactoryError, ProviderClock, ProviderFactory, ProviderInstance, SystemProviderClock,
};
use d2b_provider_toolkit::ProviderValues;

pub const MAX_DEVICE_SELECTORS: usize = 32;
pub const MAX_FIDO_CLIENT_PIN_CBOR_BYTES: usize = 1024;
pub const IMPLEMENTATION_ID: &str = "host-mediated";

const MAX_FIDO_CLIENT_PIN_MAP_ENTRIES: u64 = 16;
const MAX_FIDO_CBOR_CONTAINER_ITEMS: u64 = 32;
const MAX_FIDO_CBOR_DEPTH: u8 = 4;

pub fn implementation_id() -> ImplementationId {
    ImplementationId::parse(IMPLEMENTATION_ID)
        .unwrap_or_else(|_| unreachable!("static implementation id is valid"))
}

pub fn factory_key() -> ProviderFactoryKey {
    ProviderFactoryKey {
        provider_type: ProviderType::Device,
        implementation_id: implementation_id(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeviceKind {
    Tpm,
    Usbip,
    FidoCtaphidUhid,
    Gpu,
    Video,
    Mediated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeviceCapabilityKind {
    Tpm2Stateful,
    UsbipExclusive,
    FidoCeremony,
    GpuCrossDomain,
    VideoDecode,
    MediatedDevice,
}

impl DeviceKind {
    const fn capability(self) -> DeviceCapabilityKind {
        match self {
            Self::Tpm => DeviceCapabilityKind::Tpm2Stateful,
            Self::Usbip => DeviceCapabilityKind::UsbipExclusive,
            Self::FidoCtaphidUhid => DeviceCapabilityKind::FidoCeremony,
            Self::Gpu => DeviceCapabilityKind::GpuCrossDomain,
            Self::Video => DeviceCapabilityKind::VideoDecode,
            Self::Mediated => DeviceCapabilityKind::MediatedDevice,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FidoCommandKind {
    MakeCredential,
    GetAssertion,
    GetNextAssertion,
    GetInfo,
    ClientPin,
    Selection,
    LargeBlobs,
    Reset,
    CredentialManagement,
    BioEnrollment,
    AuthenticatorConfiguration,
    Vendor,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FidoClientPinSubcommand {
    GetPinRetries,
    GetKeyAgreement,
    SetPin,
    ChangePin,
    GetPinToken,
    GetPinUvAuthTokenUsingUvWithPermissions,
    GetUvRetries,
    GetPinUvAuthTokenUsingPinWithPermissions,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FidoClientPinParseError {
    Empty,
    TooLarge,
    InvalidCbor,
    MissingSubcommand,
    DuplicateSubcommand,
}

struct BoundedCbor<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> BoundedCbor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn read_byte(&mut self) -> Result<u8, FidoClientPinParseError> {
        let byte = self
            .bytes
            .get(self.position)
            .copied()
            .ok_or(FidoClientPinParseError::InvalidCbor)?;
        self.position += 1;
        Ok(byte)
    }

    fn read_argument(&mut self, additional: u8) -> Result<u64, FidoClientPinParseError> {
        match additional {
            0..=23 => Ok(u64::from(additional)),
            24 => Ok(u64::from(self.read_byte()?)),
            25 => {
                let bytes = [self.read_byte()?, self.read_byte()?];
                Ok(u64::from(u16::from_be_bytes(bytes)))
            }
            26 => {
                let bytes = [
                    self.read_byte()?,
                    self.read_byte()?,
                    self.read_byte()?,
                    self.read_byte()?,
                ];
                Ok(u64::from(u32::from_be_bytes(bytes)))
            }
            27 => {
                let bytes = [
                    self.read_byte()?,
                    self.read_byte()?,
                    self.read_byte()?,
                    self.read_byte()?,
                    self.read_byte()?,
                    self.read_byte()?,
                    self.read_byte()?,
                    self.read_byte()?,
                ];
                Ok(u64::from_be_bytes(bytes))
            }
            _ => Err(FidoClientPinParseError::InvalidCbor),
        }
    }

    fn read_head(&mut self) -> Result<(u8, u64), FidoClientPinParseError> {
        let initial = self.read_byte()?;
        Ok((initial >> 5, self.read_argument(initial & 0x1f)?))
    }

    fn advance(&mut self, length: u64) -> Result<(), FidoClientPinParseError> {
        let length = usize::try_from(length).map_err(|_| FidoClientPinParseError::InvalidCbor)?;
        self.position = self
            .position
            .checked_add(length)
            .filter(|position| *position <= self.bytes.len())
            .ok_or(FidoClientPinParseError::InvalidCbor)?;
        Ok(())
    }

    fn skip_body(
        &mut self,
        major: u8,
        argument: u64,
        depth: u8,
    ) -> Result<(), FidoClientPinParseError> {
        if depth > MAX_FIDO_CBOR_DEPTH {
            return Err(FidoClientPinParseError::InvalidCbor);
        }
        match major {
            0 | 1 | 7 => Ok(()),
            2 | 3 => self.advance(argument),
            4 if argument <= MAX_FIDO_CBOR_CONTAINER_ITEMS => {
                for _ in 0..argument {
                    self.skip_item(depth + 1)?;
                }
                Ok(())
            }
            5 if argument <= MAX_FIDO_CBOR_CONTAINER_ITEMS => {
                for _ in 0..argument {
                    self.skip_item(depth + 1)?;
                    self.skip_item(depth + 1)?;
                }
                Ok(())
            }
            6 => self.skip_item(depth + 1),
            _ => Err(FidoClientPinParseError::InvalidCbor),
        }
    }

    fn skip_item(&mut self, depth: u8) -> Result<(), FidoClientPinParseError> {
        let (major, argument) = self.read_head()?;
        self.skip_body(major, argument, depth)
    }
}

pub fn parse_fido_client_pin_subcommand(
    request_cbor: &[u8],
) -> Result<FidoClientPinSubcommand, FidoClientPinParseError> {
    if request_cbor.is_empty() {
        return Err(FidoClientPinParseError::Empty);
    }
    if request_cbor.len() > MAX_FIDO_CLIENT_PIN_CBOR_BYTES {
        return Err(FidoClientPinParseError::TooLarge);
    }
    let mut cursor = BoundedCbor::new(request_cbor);
    let (major, entries) = cursor.read_head()?;
    if major != 5 || entries > MAX_FIDO_CLIENT_PIN_MAP_ENTRIES {
        return Err(FidoClientPinParseError::InvalidCbor);
    }
    let mut subcommand = None;
    for _ in 0..entries {
        let (key_major, key) = cursor.read_head()?;
        if key_major != 0 {
            cursor.skip_body(key_major, key, 1)?;
            cursor.skip_item(1)?;
            continue;
        }
        if key != 2 {
            cursor.skip_item(1)?;
            continue;
        }
        if subcommand.is_some() {
            return Err(FidoClientPinParseError::DuplicateSubcommand);
        }
        let (value_major, value) = cursor.read_head()?;
        if value_major != 0 {
            cursor.skip_body(value_major, value, 1)?;
            return Err(FidoClientPinParseError::InvalidCbor);
        }
        subcommand = Some(match value {
            1 => FidoClientPinSubcommand::GetPinRetries,
            2 => FidoClientPinSubcommand::GetKeyAgreement,
            3 => FidoClientPinSubcommand::SetPin,
            4 => FidoClientPinSubcommand::ChangePin,
            5 => FidoClientPinSubcommand::GetPinToken,
            6 => FidoClientPinSubcommand::GetPinUvAuthTokenUsingUvWithPermissions,
            7 => FidoClientPinSubcommand::GetUvRetries,
            9 => FidoClientPinSubcommand::GetPinUvAuthTokenUsingPinWithPermissions,
            _ => FidoClientPinSubcommand::Unknown,
        });
    }
    if cursor.position != request_cbor.len() {
        return Err(FidoClientPinParseError::InvalidCbor);
    }
    subcommand.ok_or(FidoClientPinParseError::MissingSubcommand)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FidoCeremonyApproval {
    ApprovedTrustedSource,
    Missing,
    UntrustedSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FidoPolicyDecision {
    AllowReadOnly,
    AllowApprovedCeremony,
    DenyApprovalRequired,
    DenyDestructive,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct FidoPolicyIntent {
    require_trusted_ceremony_approval: bool,
    deny_closed_destructive_set: bool,
}

impl fmt::Debug for FidoPolicyIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FidoPolicyIntent")
            .field(
                "require_trusted_ceremony_approval",
                &self.require_trusted_ceremony_approval,
            )
            .field(
                "deny_closed_destructive_set",
                &self.deny_closed_destructive_set,
            )
            .finish()
    }
}

impl Default for FidoPolicyIntent {
    fn default() -> Self {
        Self::canonical()
    }
}

impl FidoPolicyIntent {
    pub const fn canonical() -> Self {
        Self {
            require_trusted_ceremony_approval: true,
            deny_closed_destructive_set: true,
        }
    }

    pub const fn decide(
        self,
        command: FidoCommandKind,
        approval: FidoCeremonyApproval,
    ) -> FidoPolicyDecision {
        match command {
            FidoCommandKind::GetInfo => FidoPolicyDecision::AllowReadOnly,
            FidoCommandKind::MakeCredential
            | FidoCommandKind::GetAssertion
            | FidoCommandKind::GetNextAssertion
            | FidoCommandKind::Selection => {
                if matches!(approval, FidoCeremonyApproval::ApprovedTrustedSource) {
                    FidoPolicyDecision::AllowApprovedCeremony
                } else {
                    FidoPolicyDecision::DenyApprovalRequired
                }
            }
            FidoCommandKind::ClientPin
            | FidoCommandKind::LargeBlobs
            | FidoCommandKind::Reset
            | FidoCommandKind::CredentialManagement
            | FidoCommandKind::BioEnrollment
            | FidoCommandKind::AuthenticatorConfiguration
            | FidoCommandKind::Vendor
            | FidoCommandKind::Unknown => FidoPolicyDecision::DenyDestructive,
        }
    }

    pub fn decide_client_pin(
        self,
        request_cbor: &[u8],
        approval: FidoCeremonyApproval,
    ) -> FidoPolicyDecision {
        match parse_fido_client_pin_subcommand(request_cbor) {
            Ok(
                FidoClientPinSubcommand::GetPinRetries
                | FidoClientPinSubcommand::GetKeyAgreement
                | FidoClientPinSubcommand::GetUvRetries,
            ) => FidoPolicyDecision::AllowReadOnly,
            Ok(
                FidoClientPinSubcommand::GetPinToken
                | FidoClientPinSubcommand::GetPinUvAuthTokenUsingUvWithPermissions
                | FidoClientPinSubcommand::GetPinUvAuthTokenUsingPinWithPermissions,
            ) if matches!(approval, FidoCeremonyApproval::ApprovedTrustedSource) => {
                FidoPolicyDecision::AllowApprovedCeremony
            }
            Ok(
                FidoClientPinSubcommand::GetPinToken
                | FidoClientPinSubcommand::GetPinUvAuthTokenUsingUvWithPermissions
                | FidoClientPinSubcommand::GetPinUvAuthTokenUsingPinWithPermissions,
            ) => FidoPolicyDecision::DenyApprovalRequired,
            Ok(
                FidoClientPinSubcommand::SetPin
                | FidoClientPinSubcommand::ChangePin
                | FidoClientPinSubcommand::Unknown,
            )
            | Err(_) => FidoPolicyDecision::DenyDestructive,
        }
    }
}

#[derive(Clone)]
pub enum DeviceEffectPreparation {
    Tpm {
        sidecar: SwtpmArgvInput,
        flush: SwtpmIoctlFlushInput,
    },
    Usbip(UsbipArgvInput),
    Fido(FidoPolicyIntent),
    Gpu(GpuArgvInput),
    Video(VideoArgvInput),
    Mediated,
}

impl fmt::Debug for DeviceEffectPreparation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Tpm { .. } => "DeviceEffectPreparation::Tpm(<redacted>)",
            Self::Usbip(_) => "DeviceEffectPreparation::Usbip(<redacted>)",
            Self::Fido(_) => "DeviceEffectPreparation::Fido",
            Self::Gpu(_) => "DeviceEffectPreparation::Gpu(<redacted>)",
            Self::Video(_) => "DeviceEffectPreparation::Video(<redacted>)",
            Self::Mediated => "DeviceEffectPreparation::Mediated",
        })
    }
}

#[derive(Clone)]
pub struct DeviceSelectorDefinition {
    selector_id: DeviceSelectorId,
    kind: DeviceKind,
    capability: DeviceCapabilityKind,
    preparation: DeviceEffectPreparation,
}

impl fmt::Debug for DeviceSelectorDefinition {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceSelectorDefinition")
            .field("selector_id", &"<redacted>")
            .field("kind", &self.kind)
            .field("capability", &self.capability)
            .field("preparation", &self.preparation)
            .finish()
    }
}

impl DeviceSelectorDefinition {
    pub fn tpm(
        selector_id: DeviceSelectorId,
        sidecar: SwtpmArgvInput,
        flush: SwtpmIoctlFlushInput,
    ) -> Self {
        Self {
            selector_id,
            kind: DeviceKind::Tpm,
            capability: DeviceCapabilityKind::Tpm2Stateful,
            preparation: DeviceEffectPreparation::Tpm { sidecar, flush },
        }
    }

    pub fn usbip(selector_id: DeviceSelectorId, input: UsbipArgvInput) -> Self {
        Self {
            selector_id,
            kind: DeviceKind::Usbip,
            capability: DeviceCapabilityKind::UsbipExclusive,
            preparation: DeviceEffectPreparation::Usbip(input),
        }
    }

    pub fn fido(selector_id: DeviceSelectorId) -> Self {
        Self {
            selector_id,
            kind: DeviceKind::FidoCtaphidUhid,
            capability: DeviceCapabilityKind::FidoCeremony,
            preparation: DeviceEffectPreparation::Fido(FidoPolicyIntent::canonical()),
        }
    }

    pub fn gpu(selector_id: DeviceSelectorId, input: GpuArgvInput) -> Self {
        Self {
            selector_id,
            kind: DeviceKind::Gpu,
            capability: DeviceCapabilityKind::GpuCrossDomain,
            preparation: DeviceEffectPreparation::Gpu(input),
        }
    }

    pub fn video(selector_id: DeviceSelectorId, input: VideoArgvInput) -> Self {
        Self {
            selector_id,
            kind: DeviceKind::Video,
            capability: DeviceCapabilityKind::VideoDecode,
            preparation: DeviceEffectPreparation::Video(input),
        }
    }

    pub fn mediated(selector_id: DeviceSelectorId) -> Self {
        Self {
            selector_id,
            kind: DeviceKind::Mediated,
            capability: DeviceCapabilityKind::MediatedDevice,
            preparation: DeviceEffectPreparation::Mediated,
        }
    }

    pub fn selector_id(&self) -> &DeviceSelectorId {
        &self.selector_id
    }

    pub const fn kind(&self) -> DeviceKind {
        self.kind
    }

    pub const fn capability(&self) -> DeviceCapabilityKind {
        self.capability
    }

    fn validate(&self) -> bool {
        if self.capability != self.kind.capability() {
            return false;
        }
        match (&self.kind, &self.preparation) {
            (DeviceKind::Tpm, DeviceEffectPreparation::Tpm { sidecar, flush }) => {
                sidecar.extra_args.is_empty()
                    && sidecar.vm_name == flush.vm_name
                    && sidecar.ctrl_socket_path == flush.ctrl_socket_path
                    && generate_swtpm_argv(sidecar).is_ok()
                    && generate_swtpm_ioctl_flush_argv(flush).is_ok()
            }
            (DeviceKind::Usbip, DeviceEffectPreparation::Usbip(input)) => {
                generate_usbip_argv(input, UsbipSubcommand::Bind).is_ok()
                    && generate_usbip_argv(input, UsbipSubcommand::Unbind).is_ok()
            }
            (DeviceKind::FidoCtaphidUhid, DeviceEffectPreparation::Fido(policy)) => {
                *policy == FidoPolicyIntent::canonical()
            }
            (DeviceKind::Gpu, DeviceEffectPreparation::Gpu(input)) => {
                input.extra_args.is_empty() && generate_gpu_argv(input).is_ok()
            }
            (DeviceKind::Video, DeviceEffectPreparation::Video(input)) => {
                generate_video_argv(input).is_ok()
            }
            (DeviceKind::Mediated, DeviceEffectPreparation::Mediated) => true,
            _ => false,
        }
    }

    fn semantic(&self) -> DeviceSemanticSelector {
        DeviceSemanticSelector {
            selector_id: self.selector_id.clone(),
            kind: self.kind,
            capability: self.capability,
            preparation: self.preparation.clone(),
        }
    }
}

#[derive(Clone)]
pub struct DeviceSemanticSelector {
    selector_id: DeviceSelectorId,
    kind: DeviceKind,
    capability: DeviceCapabilityKind,
    preparation: DeviceEffectPreparation,
}

impl fmt::Debug for DeviceSemanticSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceSemanticSelector")
            .field("selector_id", &"<redacted>")
            .field("kind", &self.kind)
            .field("capability", &self.capability)
            .field("preparation", &self.preparation)
            .finish()
    }
}

impl DeviceSemanticSelector {
    pub fn selector_id(&self) -> &DeviceSelectorId {
        &self.selector_id
    }

    pub const fn kind(&self) -> DeviceKind {
        self.kind
    }

    pub const fn capability(&self) -> DeviceCapabilityKind {
        self.capability
    }

    pub fn preparation(&self) -> &DeviceEffectPreparation {
        &self.preparation
    }

    pub const fn fido_policy(&self) -> Option<FidoPolicyIntent> {
        match self.preparation {
            DeviceEffectPreparation::Fido(policy) => Some(policy),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct DeviceCall {
    operation: ProviderOperationContext,
    peer_role: EndpointRole,
    monotonic_deadline_remaining_ms: u32,
}

impl fmt::Debug for DeviceCall {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceCall")
            .field("method", &self.operation.method)
            .field("provider_generation", &self.operation.provider_generation)
            .field("peer_role", &self.peer_role)
            .field(
                "monotonic_deadline_remaining_ms",
                &self.monotonic_deadline_remaining_ms,
            )
            .finish_non_exhaustive()
    }
}

impl DeviceCall {
    pub fn operation(&self) -> &ProviderOperationContext {
        &self.operation
    }

    pub fn binding(&self) -> OperationBinding {
        self.operation.binding()
    }

    pub fn scope(&self) -> &AuthorizedProviderScope {
        &self.operation.scope
    }

    pub const fn monotonic_deadline_remaining_ms(&self) -> u32 {
        self.monotonic_deadline_remaining_ms
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevicePortError {
    Denied,
    Unavailable,
    Stale,
    Ambiguous,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeviceDispatch {
    Read,
    Mutation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevicePlanOutcome {
    pub plan_id: PlanId,
    pub expires_at_unix_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAttachOutcome {
    pub handle_id: HandleId,
    pub resource_generation: Generation,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DeviceInspection {
    pub handle: Option<ProviderHandle>,
    pub lifecycle: ObservedLifecycleState,
    pub reason: ObservationReason,
    pub health_state: ProviderHealthState,
    pub health_reason: ProviderHealthReason,
    pub remediation: ProviderRemediation,
}

impl fmt::Debug for DeviceInspection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceInspection")
            .field("has_handle", &self.handle.is_some())
            .field("lifecycle", &self.lifecycle)
            .field("reason", &self.reason)
            .field("health_state", &self.health_state)
            .field("health_reason", &self.health_reason)
            .field("remediation", &self.remediation)
            .finish()
    }
}

impl DeviceInspection {
    pub fn ready(handle: Option<ProviderHandle>) -> Self {
        Self {
            handle,
            lifecycle: ObservedLifecycleState::Ready,
            reason: ObservationReason::None,
            health_state: ProviderHealthState::Healthy,
            health_reason: ProviderHealthReason::None,
            remediation: ProviderRemediation::None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceAdoptionOutcome {
    Adopted,
    Rejected(ObservationReason),
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceHealth {
    pub state: ProviderHealthState,
    pub reason: ProviderHealthReason,
    pub remediation: ProviderRemediation,
}

impl DeviceHealth {
    pub const fn healthy() -> Self {
        Self {
            state: ProviderHealthState::Healthy,
            reason: ProviderHealthReason::None,
            remediation: ProviderRemediation::None,
        }
    }
}

#[async_trait]
pub trait DeviceEffectPort: Send + Sync {
    async fn plan_attach(
        &self,
        context: DeviceCall,
        selector: DeviceSemanticSelector,
    ) -> Result<DevicePlanOutcome, DevicePortError>;

    async fn attach(
        &self,
        context: DeviceCall,
        plan: ProviderPlan,
    ) -> Result<DeviceAttachOutcome, DevicePortError>;

    async fn detach(
        &self,
        context: DeviceCall,
        target: ProviderTarget,
    ) -> Result<MutationState, DevicePortError>;
}

#[async_trait]
pub trait DeviceQueryPort: Send + Sync {
    async fn health(&self, context: DeviceCall) -> Result<DeviceHealth, DevicePortError>;

    async fn inspect(
        &self,
        context: DeviceCall,
        target: ProviderTarget,
    ) -> Result<DeviceInspection, DevicePortError>;

    async fn adopt(
        &self,
        context: DeviceCall,
        request: AdoptionRequest,
    ) -> Result<DeviceAdoptionOutcome, DevicePortError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceProviderBuildError {
    Contract(ProviderContractError),
    WrongProviderType,
    WrongImplementation,
    WrongPlacement,
    CapabilityMismatch,
    EmptySelectorSet,
    TooManySelectors,
    DuplicateSelector,
    InvalidSelector,
    EmptyFactory,
    TooManyFactoryEntries,
    DuplicateProvider,
}

impl From<ProviderContractError> for DeviceProviderBuildError {
    fn from(value: ProviderContractError) -> Self {
        Self::Contract(value)
    }
}

fn validate_device_descriptor(
    descriptor: &ProviderDescriptor,
) -> Result<(), DeviceProviderBuildError> {
    descriptor.validate()?;
    if descriptor.provider_type() != ProviderType::Device {
        return Err(DeviceProviderBuildError::WrongProviderType);
    }
    if descriptor.implementation_id != implementation_id() {
        return Err(DeviceProviderBuildError::WrongImplementation);
    }
    if !matches!(
        descriptor.placement,
        ProviderPlacement::TrustedFirstPartyInProcess { .. }
    ) {
        return Err(DeviceProviderBuildError::WrongPlacement);
    }
    if descriptor.capabilities != live_device_capabilities()? {
        return Err(DeviceProviderBuildError::CapabilityMismatch);
    }
    Ok(())
}

fn validate_device_selectors(
    selectors: &[DeviceSelectorDefinition],
) -> Result<(), DeviceProviderBuildError> {
    if selectors.is_empty() {
        return Err(DeviceProviderBuildError::EmptySelectorSet);
    }
    if selectors.len() > MAX_DEVICE_SELECTORS {
        return Err(DeviceProviderBuildError::TooManySelectors);
    }
    let mut indexed = BTreeMap::new();
    for selector in selectors {
        if !selector.validate() {
            return Err(DeviceProviderBuildError::InvalidSelector);
        }
        if indexed.insert(selector.selector_id.clone(), ()).is_some() {
            return Err(DeviceProviderBuildError::DuplicateSelector);
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct HostMediatedDeviceFactoryEntry {
    provider_id: ProviderId,
    configuration_schema_fingerprint: Fingerprint,
    configured_scope_digest: Fingerprint,
    placement: ProviderPlacement,
    selectors: Arc<Vec<DeviceSelectorDefinition>>,
    effects: Arc<dyn DeviceEffectPort>,
    queries: Arc<dyn DeviceQueryPort>,
}

pub type FactoryEntry = HostMediatedDeviceFactoryEntry;

impl fmt::Debug for HostMediatedDeviceFactoryEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostMediatedDeviceFactoryEntry")
            .field("provider_id", &"<redacted>")
            .field("selector_count", &self.selectors.len())
            .field("placement", &self.placement)
            .finish_non_exhaustive()
    }
}

impl HostMediatedDeviceFactoryEntry {
    pub fn new(
        descriptor: &ProviderDescriptor,
        selectors: Vec<DeviceSelectorDefinition>,
        effects: Arc<dyn DeviceEffectPort>,
        queries: Arc<dyn DeviceQueryPort>,
    ) -> Result<Self, DeviceProviderBuildError> {
        validate_device_descriptor(descriptor)?;
        validate_device_selectors(&selectors)?;
        Ok(Self {
            provider_id: descriptor.provider_id.clone(),
            configuration_schema_fingerprint: descriptor.configuration_schema_fingerprint.clone(),
            configured_scope_digest: descriptor.configured_scope_digest.clone(),
            placement: descriptor.placement.clone(),
            selectors: Arc::new(selectors),
            effects,
            queries,
        })
    }

    fn matches(&self, descriptor: &ProviderDescriptor) -> bool {
        self.provider_id == descriptor.provider_id
            && self.configuration_schema_fingerprint == descriptor.configuration_schema_fingerprint
            && self.configured_scope_digest == descriptor.configured_scope_digest
            && self.placement == descriptor.placement
    }
}

#[derive(Clone)]
pub struct HostMediatedDeviceFactory {
    entries: Arc<BTreeMap<ProviderId, HostMediatedDeviceFactoryEntry>>,
    clock: Arc<dyn ProviderClock>,
}

pub type Factory = HostMediatedDeviceFactory;

impl fmt::Debug for HostMediatedDeviceFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostMediatedDeviceFactory")
            .field("key", &factory_key())
            .field("entry_count", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl HostMediatedDeviceFactory {
    pub fn new(
        entries: Vec<HostMediatedDeviceFactoryEntry>,
    ) -> Result<Self, DeviceProviderBuildError> {
        Self::with_clock(entries, Arc::new(SystemProviderClock))
    }

    pub fn with_clock(
        entries: Vec<HostMediatedDeviceFactoryEntry>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, DeviceProviderBuildError> {
        if entries.is_empty() {
            return Err(DeviceProviderBuildError::EmptyFactory);
        }
        if entries.len() > MAX_PROVIDER_REGISTRY_ENTRIES {
            return Err(DeviceProviderBuildError::TooManyFactoryEntries);
        }
        let mut indexed = BTreeMap::new();
        for entry in entries {
            if indexed.insert(entry.provider_id.clone(), entry).is_some() {
                return Err(DeviceProviderBuildError::DuplicateProvider);
            }
        }
        Ok(Self {
            entries: Arc::new(indexed),
            clock,
        })
    }
}

impl ProviderFactory for HostMediatedDeviceFactory {
    fn construct(&self, descriptor: &ProviderDescriptor) -> Result<ProviderInstance, FactoryError> {
        if descriptor.provider_type() != ProviderType::Device
            || descriptor.implementation_id != implementation_id()
        {
            return Err(FactoryError::Rejected);
        }
        let entry = self
            .entries
            .get(&descriptor.provider_id)
            .filter(|entry| entry.matches(descriptor))
            .ok_or(FactoryError::Rejected)?;
        HostMediatedDeviceProvider::with_clock(
            descriptor.clone(),
            entry.selectors.as_ref().clone(),
            entry.effects.clone(),
            entry.queries.clone(),
            self.clock.clone(),
        )
        .map(|provider| ProviderInstance::Device(Arc::new(provider)))
        .map_err(|_| FactoryError::Rejected)
    }
}

#[derive(Clone)]
pub struct HostMediatedDeviceProvider {
    descriptor: ProviderDescriptor,
    selectors: Arc<BTreeMap<DeviceSelectorId, DeviceSelectorDefinition>>,
    effects: Arc<dyn DeviceEffectPort>,
    queries: Arc<dyn DeviceQueryPort>,
    clock: Arc<dyn ProviderClock>,
}

impl fmt::Debug for HostMediatedDeviceProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostMediatedDeviceProvider")
            .field("provider_generation", &self.descriptor.registry_generation)
            .field("selector_count", &self.selectors.len())
            .finish_non_exhaustive()
    }
}

impl HostMediatedDeviceProvider {
    pub fn new(
        descriptor: ProviderDescriptor,
        selectors: Vec<DeviceSelectorDefinition>,
        effects: Arc<dyn DeviceEffectPort>,
        queries: Arc<dyn DeviceQueryPort>,
    ) -> Result<Self, DeviceProviderBuildError> {
        Self::with_clock(
            descriptor,
            selectors,
            effects,
            queries,
            Arc::new(SystemProviderClock),
        )
    }

    pub fn with_clock(
        descriptor: ProviderDescriptor,
        selectors: Vec<DeviceSelectorDefinition>,
        effects: Arc<dyn DeviceEffectPort>,
        queries: Arc<dyn DeviceQueryPort>,
        clock: Arc<dyn ProviderClock>,
    ) -> Result<Self, DeviceProviderBuildError> {
        validate_device_descriptor(&descriptor)?;
        validate_device_selectors(&selectors)?;
        let mut indexed = BTreeMap::new();
        for selector in selectors {
            indexed.insert(selector.selector_id.clone(), selector);
        }
        Ok(Self {
            descriptor,
            selectors: Arc::new(indexed),
            effects,
            queries,
            clock,
        })
    }

    fn expected_peer_role(&self) -> EndpointRole {
        match &self.descriptor.placement {
            ProviderPlacement::TrustedFirstPartyInProcess {
                controller_role, ..
            } => *controller_role,
            ProviderPlacement::ProviderAgent { endpoint_role, .. }
            | ProviderPlacement::UserAgent { endpoint_role, .. } => *endpoint_role,
        }
    }

    fn now_unix_ms(&self) -> u64 {
        self.clock
            .now_unix_ms()
            .min(d2b_contracts::v2_provider::MAX_SAFE_JSON_INTEGER)
    }

    fn failure(
        &self,
        operation: &ProviderOperationContext,
        kind: ProviderFailureKind,
        retry: RetryClass,
        reason: ProviderHealthReason,
        remediation: ProviderRemediation,
    ) -> ProviderFailure {
        let mut binding = operation.binding();
        binding.provider_id = self.descriptor.provider_id.clone();
        binding.provider_generation = self.descriptor.registry_generation;
        ProviderFailure {
            kind,
            retry,
            provider_type: ProviderType::Device,
            binding,
            correlation_id: operation.correlation_id.clone(),
            occurred_at_unix_ms: self.now_unix_ms(),
            reason,
            remediation,
        }
    }

    fn invalid_request(&self, operation: &ProviderOperationContext) -> ProviderFailure {
        self.failure(
            operation,
            ProviderFailureKind::InvalidRequest,
            RetryClass::Never,
            ProviderHealthReason::ConfigurationMismatch,
            ProviderRemediation::RepairConfiguration,
        )
    }

    fn validate_call(
        &self,
        context: &ProviderCallContext<'_>,
        operation: &ProviderOperationContext,
        expected: ProviderMethod,
    ) -> Result<DeviceCall, ProviderFailure> {
        if context.cancelled {
            return Err(self.failure(
                operation,
                ProviderFailureKind::Cancelled,
                RetryClass::Never,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::RetryBounded,
            ));
        }
        if context.monotonic_deadline_remaining_ms == 0 {
            return Err(self.failure(
                operation,
                ProviderFailureKind::DeadlineExpired,
                RetryClass::SameOperation,
                ProviderHealthReason::HealthTimeout,
                ProviderRemediation::RetryBounded,
            ));
        }
        if context.operation != operation
            || context.peer_role != self.expected_peer_role()
            || context.validate().is_err()
            || operation
                .validate(&self.descriptor, self.now_unix_ms())
                .is_err()
            || operation.method != expected
        {
            return Err(self.invalid_request(operation));
        }
        Ok(DeviceCall {
            operation: operation.clone(),
            peer_role: context.peer_role,
            monotonic_deadline_remaining_ms: context.monotonic_deadline_remaining_ms,
        })
    }

    fn validate_request(
        &self,
        context: &ProviderCallContext<'_>,
        request: &ProviderOperationRequest,
        expected: ProviderMethod,
    ) -> Result<DeviceCall, ProviderFailure> {
        let call = self.validate_call(context, &request.context, expected)?;
        if request
            .validate_method(&self.descriptor, self.now_unix_ms(), expected)
            .is_err()
        {
            return Err(self.invalid_request(&request.context));
        }
        Ok(call)
    }

    async fn invoke<T, F>(
        &self,
        operation: &ProviderOperationContext,
        deadline_ms: u32,
        dispatch: DeviceDispatch,
        future: F,
    ) -> Result<T, ProviderFailure>
    where
        F: Future<Output = Result<T, DevicePortError>> + Send,
    {
        match tokio::time::timeout(Duration::from_millis(u64::from(deadline_ms)), future).await {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) => Err(self.port_failure(operation, error)),
            Err(_) => Err(match dispatch {
                DeviceDispatch::Read => self.failure(
                    operation,
                    ProviderFailureKind::DeadlineExpired,
                    RetryClass::SameOperation,
                    ProviderHealthReason::HealthTimeout,
                    ProviderRemediation::RetryBounded,
                ),
                DeviceDispatch::Mutation => self.failure(
                    operation,
                    ProviderFailureKind::AmbiguousMutation,
                    RetryClass::AfterObservation,
                    ProviderHealthReason::AdoptionAmbiguous,
                    ProviderRemediation::InspectProvider,
                ),
            }),
        }
    }

    fn port_failure(
        &self,
        operation: &ProviderOperationContext,
        error: DevicePortError,
    ) -> ProviderFailure {
        match error {
            DevicePortError::Denied => self.failure(
                operation,
                ProviderFailureKind::CapabilityDenied,
                RetryClass::Never,
                ProviderHealthReason::CapabilityMismatch,
                ProviderRemediation::RepairConfiguration,
            ),
            DevicePortError::Unavailable => self.failure(
                operation,
                ProviderFailureKind::Unavailable,
                RetryClass::SameOperation,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::RetryBounded,
            ),
            DevicePortError::Stale => self.failure(
                operation,
                ProviderFailureKind::AdoptionRejected,
                RetryClass::Never,
                ProviderHealthReason::GenerationMismatch,
                ProviderRemediation::ReplaceGeneration,
            ),
            DevicePortError::Ambiguous => self.failure(
                operation,
                ProviderFailureKind::AmbiguousMutation,
                RetryClass::AfterObservation,
                ProviderHealthReason::AdoptionAmbiguous,
                ProviderRemediation::InspectProvider,
            ),
            DevicePortError::Cancelled => self.failure(
                operation,
                ProviderFailureKind::Cancelled,
                RetryClass::Never,
                ProviderHealthReason::ProviderDegraded,
                ProviderRemediation::RetryBounded,
            ),
        }
    }

    fn values(
        &self,
        operation: &ProviderOperationContext,
    ) -> Result<ProviderValues, ProviderFailure> {
        ProviderValues::new(&self.descriptor, self.now_unix_ms())
            .map_err(|_| self.invalid_request(operation))
    }

    fn validate_plan(
        &self,
        operation: &ProviderOperationContext,
        plan: &ProviderPlan,
    ) -> Result<(), ProviderFailure> {
        let now = self.now_unix_ms();
        if plan.schema_version != PROVIDER_SCHEMA_VERSION
            || plan.method != ProviderMethod::DevicePlanAttach
            || plan.binding.provider_id != self.descriptor.provider_id
            || plan.binding.provider_generation != self.descriptor.registry_generation
            || plan.configuration_fingerprint != self.descriptor.configuration_schema_fingerprint
            || plan.realm_id != *operation.scope.realm_id()
            || plan.workload_id.as_ref() != operation.scope.workload_id()
            || plan.created_at_unix_ms > now
            || plan.expires_at_unix_ms <= now
            || plan.resources.as_slice() != [PlannedResourceClass::DeviceAttachment]
        {
            Err(self.invalid_request(operation))
        } else {
            Ok(())
        }
    }

    fn validate_inspection(
        &self,
        operation: &ProviderOperationContext,
        target: &ProviderTarget,
        inspection: &DeviceInspection,
    ) -> Result<(), ProviderFailure> {
        let Some(handle) = inspection.handle.as_ref() else {
            return if matches!(target, ProviderTarget::Workload { .. }) {
                Ok(())
            } else {
                Err(self.invalid_request(operation))
            };
        };
        if handle.validate().is_err()
            || handle.kind != ProviderHandleKind::Device
            || handle.provider_id != self.descriptor.provider_id
            || handle.provider_generation != self.descriptor.registry_generation
            || handle.configuration_fingerprint != self.descriptor.configuration_schema_fingerprint
            || !target_matches_handle(target, handle)
        {
            Err(self.invalid_request(operation))
        } else {
            Ok(())
        }
    }
}

pub fn live_device_capabilities() -> Result<ProviderCapabilitySet, ProviderContractError> {
    ProviderCapabilitySet::new(vec![
        ProviderCapability(ProviderMethod::DevicePlanAttach),
        ProviderCapability(ProviderMethod::DeviceAttach),
        ProviderCapability(ProviderMethod::DeviceInspect),
        ProviderCapability(ProviderMethod::DeviceAdopt),
        ProviderCapability(ProviderMethod::DeviceDetach),
    ])
}

impl Provider for HostMediatedDeviceProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        self.descriptor.clone()
    }

    fn health<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
    ) -> ProviderFuture<'a, ProviderHealth> {
        Box::pin(async move {
            let call = self.validate_call(context, context.operation, context.operation.method)?;
            let deadline = call.monotonic_deadline_remaining_ms();
            let health = self
                .invoke(
                    context.operation,
                    deadline,
                    DeviceDispatch::Read,
                    self.queries.health(call),
                )
                .await?;
            self.values(context.operation)?
                .health(health.state, health.reason, health.remediation)
                .map_err(|_| self.invalid_request(context.operation))
        })
    }
}

impl DeviceProvider for HostMediatedDeviceProvider {
    fn capabilities(&self) -> ProviderCapabilitySet {
        self.descriptor.capabilities.clone()
    }

    fn plan_attach<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderPlan> {
        Box::pin(async move {
            let call = self.validate_request(context, request, ProviderMethod::DevicePlanAttach)?;
            if !matches!(request.target, ProviderTarget::Workload { .. }) {
                return Err(self.invalid_request(&request.context));
            }
            let ProviderOperationInput::DeviceSelector { device_selector_id } = &request.input
            else {
                return Err(self.invalid_request(&request.context));
            };
            let selector = self
                .selectors
                .get(device_selector_id)
                .ok_or_else(|| self.invalid_request(&request.context))?
                .semantic();
            let deadline = call.monotonic_deadline_remaining_ms();
            let outcome = self
                .invoke(
                    &request.context,
                    deadline,
                    DeviceDispatch::Read,
                    self.effects.plan_attach(call, selector),
                )
                .await?;
            let resources = BoundedVec::new(vec![PlannedResourceClass::DeviceAttachment])
                .map_err(|_| self.invalid_request(&request.context))?;
            self.values(&request.context)?
                .plan(
                    request,
                    outcome.plan_id,
                    outcome.expires_at_unix_ms,
                    resources,
                )
                .map_err(|_| self.invalid_request(&request.context))
        })
    }

    fn attach<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        plan: &'a ProviderPlan,
    ) -> ProviderFuture<'a, ProviderHandle> {
        Box::pin(async move {
            let call =
                self.validate_call(context, context.operation, ProviderMethod::DeviceAttach)?;
            self.validate_plan(context.operation, plan)?;
            let deadline = call.monotonic_deadline_remaining_ms();
            let outcome = self
                .invoke(
                    context.operation,
                    deadline,
                    DeviceDispatch::Mutation,
                    self.effects.attach(call, plan.clone()),
                )
                .await?;
            let values = self.values(context.operation)?;
            values
                .handle_from_plan(
                    plan,
                    outcome.handle_id,
                    values.provider_owner(&plan.realm_id),
                    outcome.resource_generation,
                    None,
                )
                .map_err(|_| self.invalid_request(context.operation))
        })
    }

    fn inspect<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(async move {
            let call = self.validate_request(context, request, ProviderMethod::DeviceInspect)?;
            if matches!(request.target, ProviderTarget::Realm { .. }) {
                return Err(self.invalid_request(&request.context));
            }
            let deadline = call.monotonic_deadline_remaining_ms();
            let inspection = self
                .invoke(
                    &request.context,
                    deadline,
                    DeviceDispatch::Read,
                    self.queries.inspect(call, request.target.clone()),
                )
                .await?;
            self.validate_inspection(&request.context, &request.target, &inspection)?;
            self.values(&request.context)?
                .observation(
                    &request.context,
                    inspection.handle.as_ref(),
                    inspection.lifecycle,
                    AdoptionState::NotAttempted,
                    inspection.reason,
                    inspection.health_state,
                    inspection.health_reason,
                    inspection.remediation,
                )
                .map_err(|_| self.invalid_request(&request.context))
        })
    }

    fn adopt<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a AdoptionRequest,
    ) -> ProviderFuture<'a, ProviderObservation> {
        Box::pin(async move {
            let call =
                self.validate_call(context, &request.context, ProviderMethod::DeviceAdopt)?;
            if request
                .validate(&self.descriptor, self.now_unix_ms())
                .is_err()
                || request.handle.kind != ProviderHandleKind::Device
            {
                return Err(self.invalid_request(&request.context));
            }
            let deadline = call.monotonic_deadline_remaining_ms();
            let outcome = self
                .invoke(
                    &request.context,
                    deadline,
                    DeviceDispatch::Read,
                    self.queries.adopt(call, request.clone()),
                )
                .await?;
            let values = self.values(&request.context)?;
            let result = match outcome {
                DeviceAdoptionOutcome::Adopted => values.observation(
                    &request.context,
                    Some(&request.handle),
                    ObservedLifecycleState::Ready,
                    AdoptionState::Adopted,
                    ObservationReason::None,
                    ProviderHealthState::Healthy,
                    ProviderHealthReason::None,
                    ProviderRemediation::None,
                ),
                DeviceAdoptionOutcome::Rejected(reason)
                    if !matches!(
                        reason,
                        ObservationReason::None | ObservationReason::MultipleCandidates
                    ) =>
                {
                    values.observation(
                        &request.context,
                        Some(&request.handle),
                        ObservedLifecycleState::Unknown,
                        AdoptionState::Rejected,
                        reason,
                        ProviderHealthState::Healthy,
                        ProviderHealthReason::None,
                        ProviderRemediation::None,
                    )
                }
                DeviceAdoptionOutcome::Ambiguous => values.observation(
                    &request.context,
                    Some(&request.handle),
                    ObservedLifecycleState::Quarantined,
                    AdoptionState::Ambiguous,
                    ObservationReason::MultipleCandidates,
                    ProviderHealthState::Failed,
                    ProviderHealthReason::AdoptionAmbiguous,
                    ProviderRemediation::OperatorInteraction,
                ),
                DeviceAdoptionOutcome::Rejected(_) => {
                    return Err(self.invalid_request(&request.context));
                }
            };
            result.map_err(|_| self.invalid_request(&request.context))
        })
    }

    fn detach<'a>(
        &'a self,
        context: &'a ProviderCallContext<'a>,
        request: &'a ProviderOperationRequest,
    ) -> ProviderFuture<'a, MutationReceipt> {
        Box::pin(async move {
            let call = self.validate_request(context, request, ProviderMethod::DeviceDetach)?;
            if !matches!(request.target, ProviderTarget::Handle { .. }) {
                return Err(self.invalid_request(&request.context));
            }
            let deadline = call.monotonic_deadline_remaining_ms();
            let state = self
                .invoke(
                    &request.context,
                    deadline,
                    DeviceDispatch::Mutation,
                    self.effects.detach(call, request.target.clone()),
                )
                .await?;
            self.values(&request.context)?
                .receipt(&request.context, state)
                .map_err(|_| self.invalid_request(&request.context))
        })
    }
}

fn target_matches_handle(target: &ProviderTarget, handle: &ProviderHandle) -> bool {
    match target {
        ProviderTarget::Realm { .. } => false,
        ProviderTarget::Workload {
            realm_id,
            workload_id,
        } => &handle.realm_id == realm_id && handle.workload_id.as_ref() == Some(workload_id),
        ProviderTarget::Handle {
            realm_id,
            workload_id,
            handle_id,
            handle_generation,
        } => {
            &handle.realm_id == realm_id
                && &handle.workload_id == workload_id
                && &handle.handle_id == handle_id
                && &handle.resource_generation == handle_generation
        }
    }
}

#[cfg(test)]
mod tests;
