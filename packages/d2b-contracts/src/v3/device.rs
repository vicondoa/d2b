//! Device primitive ResourceType base spec.
//!
//! `Device` is the inventoried, exclusive-or-shared device arbitration
//! ResourceType. `deviceClass`, `arbitration`, `maxConcurrentClaims`, and the
//! `inventory.selector` discriminated union are Layer 2 base fields;
//! implementation-only device configuration belongs to the Layer 3
//! `spec.provider` envelope on the universal `ResourceSpec`.
//!
//! No raw device path appears in the spec. A physical device is selected by a
//! stable operator-defined label plus optional bounded filter fields, and the
//! Provider resolves the physical node privately.

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use super::{
    ResourceRef, ResourceUid, StatusMessage, Timestamp,
    execution_policy::{
        BoundedToken, PrimitiveSpecError, parsed_deserialize, redacted_debug, string_schema,
    },
};

/// The canonical ResourceType name for this module.
pub const DEVICE_RESOURCE_TYPE: &str = "Device";
/// Maximum simultaneous claimants on one Device.
pub const MAX_CONCURRENT_CLAIMS: u32 = 16;
/// Maximum bytes in one device serial filter.
pub const MAX_DEVICE_SERIAL_BYTES: usize = 128;
/// Maximum bytes in one PCI slot filter.
pub const MAX_PCI_SLOT_BYTES: usize = 31;
/// Maximum Device status holder references retained in one projection.
pub const MAX_DEVICE_HOLDER_REFS: usize = 64;
/// Maximum Device claim entries retained in one status projection.
pub const MAX_DEVICE_CLAIMS: usize = 64;
/// Maximum bytes in a redacted Provider diagnostic.
pub const MAX_DEVICE_DIAGNOSTIC_BYTES: usize = 4 * 1024;
/// Maximum physical-device authority descriptors in one Device admission.
pub const MAX_DEVICE_AUTHORITY_DESCRIPTORS: usize = 1;

/// The Device Provider's finalizer. The value is deliberately a closed
/// constant rather than a caller-provided string.
pub const DEVICE_TPM_FINALIZER: &str = "device-tpm.d2bus.org/state-preserved";
/// The Device Provider's USBIP finalizer.
pub const DEVICE_USBIP_FINALIZER: &str = "device-usbip.d2bus.org/attachment-released";
/// The Device Provider's security-key finalizer.
pub const DEVICE_SECURITY_KEY_FINALIZER: &str = "device-security-key.d2bus.org/lease-released";
/// The Device Provider's GPU finalizer.
pub const DEVICE_GPU_FINALIZER: &str = "device-gpu.d2bus.org/worker-stopped";

/// The single physical-device authority scope admitted by Device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceAuthorityScope {
    /// A host physical backing, never a raw path.
    PhysicalDevice,
}

/// Cardinality of one Core-derived physical backing authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceAuthorityCardinality {
    /// At most one authority may own a physical backing in a Zone.
    ZeroOrOne,
}

/// Requested physical-device arbitration declared by the Device resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DeviceAuthorityArbitration {
    /// A whole device or exclusive backing.
    Exclusive,
    /// A genuinely multiplexable engine, such as a render node.
    Shared,
}

/// A physical-device authority descriptor with no path, serial, bus ID, or
/// resource-name-derived identity.
///
/// Core creates this descriptor after resolving the trusted inventory selector.
/// Providers may retain the opaque key and compare it, but cannot construct a
/// physical authority from a host path.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceAuthorityDescriptor {
    authority_scope: DeviceAuthorityScope,
    authority_key: DeviceAuthorityKey,
    cardinality: DeviceAuthorityCardinality,
    arbitration: DeviceAuthorityArbitration,
}

impl DeviceAuthorityDescriptor {
    /// Construct one Core-derived descriptor.
    pub const fn new(
        authority_key: DeviceAuthorityKey,
        arbitration: DeviceAuthorityArbitration,
    ) -> Self {
        Self {
            authority_scope: DeviceAuthorityScope::PhysicalDevice,
            authority_key,
            cardinality: DeviceAuthorityCardinality::ZeroOrOne,
            arbitration,
        }
    }

    /// Return the fixed physical-device scope.
    pub const fn authority_scope(&self) -> DeviceAuthorityScope {
        self.authority_scope
    }

    /// Borrow the opaque authority key.
    pub const fn authority_key(&self) -> &DeviceAuthorityKey {
        &self.authority_key
    }

    /// Return the fixed cardinality.
    pub const fn cardinality(&self) -> DeviceAuthorityCardinality {
        self.cardinality
    }

    /// Return the requested arbitration.
    pub const fn arbitration(&self) -> DeviceAuthorityArbitration {
        self.arbitration
    }
}

redacted_debug!(DeviceAuthorityDescriptor);

/// Core-derived opaque identity for one physical device backing.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct DeviceAuthorityKey([u8; 32]);

impl DeviceAuthorityKey {
    /// Construct a key only at the trusted Core adapter boundary.
    pub const fn from_core(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrow the key for equality at another trusted adapter boundary.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

redacted_debug!(DeviceAuthorityKey);

impl<'de> Deserialize<'de> for DeviceAuthorityKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let bytes = <[u8; 32]>::deserialize(deserializer)?;
        if bytes == [0; 32] {
            return Err(serde::de::Error::custom(
                PrimitiveSpecError::MissingRequiredField,
            ));
        }
        Ok(Self(bytes))
    }
}

impl JsonSchema for DeviceAuthorityKey {
    fn schema_name() -> String {
        "DeviceAuthorityKey".to_owned()
    }

    fn json_schema(_gen: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        schemars::schema::Schema::Object(schemars::schema::SchemaObject {
            instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
                schemars::schema::InstanceType::Array,
            ))),
            array: Some(Box::new(schemars::schema::ArrayValidation {
                min_items: Some(32),
                max_items: Some(32),
                items: Some(schemars::schema::SingleOrVec::Single(Box::new(
                    schemars::schema::Schema::Object(schemars::schema::SchemaObject {
                        instance_type: Some(schemars::schema::SingleOrVec::Single(Box::new(
                            schemars::schema::InstanceType::Integer,
                        ))),
                        ..Default::default()
                    }),
                ))),
                ..Default::default()
            })),
            ..Default::default()
        })
    }
}

/// Whether a device exists in host inventory or is created by its Provider.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum DeviceClass {
    Physical,
    Emulated,
}

/// Whether the device may be held by more than one claimant.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum DeviceArbitration {
    Exclusive,
    Shared,
}

/// A validated four-digit lower-case hexadecimal USB vendor or product ID.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct HexId(String);

impl HexId {
    /// Parse exactly four lower-case ASCII hexadecimal digits.
    pub fn parse(value: impl Into<String>) -> Result<Self, PrimitiveSpecError> {
        let value = value.into();
        if value.len() == 4
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            Ok(Self(value))
        } else {
            Err(PrimitiveSpecError::InvalidToken)
        }
    }

    /// Borrow the canonical identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

redacted_debug!(HexId);
parsed_deserialize!(HexId);
string_schema!(HexId, 4, 4);

/// A validated bounded device serial or slot filter.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct DeviceFilterText(String);

impl DeviceFilterText {
    /// Parse bounded printable ASCII with no control character or separator.
    pub fn parse(value: impl Into<String>) -> Result<Self, PrimitiveSpecError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_DEVICE_SERIAL_BYTES
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'/' | b'\\'))
        {
            return Err(PrimitiveSpecError::InvalidText);
        }
        Ok(Self(value))
    }

    /// Borrow the filter text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

redacted_debug!(DeviceFilterText);
parsed_deserialize!(DeviceFilterText);
string_schema!(DeviceFilterText, 1, MAX_DEVICE_SERIAL_BYTES);

/// The closed inventory selector union, discriminated on `busClass`.
///
/// An emulated device carries no selector; a physical device always names a
/// stable operator-defined `label`.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", tag = "busClass")]
pub enum InventorySelector {
    /// A USB device.
    #[serde(rename = "usb")]
    Usb {
        label: BoundedToken,
        vendor_id: Option<HexId>,
        product_id: Option<HexId>,
        serial: Option<DeviceFilterText>,
    },
    /// A HID or hidraw device.
    #[serde(rename = "hidraw")]
    Hidraw {
        label: BoundedToken,
        vendor_id: Option<HexId>,
        product_id: Option<HexId>,
        serial: Option<DeviceFilterText>,
    },
    /// A DRM or GPU device.
    #[serde(rename = "drm")]
    Drm {
        label: BoundedToken,
        pci_slot: Option<DeviceFilterText>,
    },
    /// A non-GPU PCI device.
    #[serde(rename = "pci")]
    Pci {
        label: BoundedToken,
        slot: Option<DeviceFilterText>,
    },
    /// A physical TPM kernel device.
    #[serde(rename = "tpm")]
    Tpm { label: BoundedToken, index: u8 },
}

redacted_debug!(InventorySelector);

impl<'de> Deserialize<'de> for InventorySelector {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// The union of every declared selector field.
        ///
        /// Serde does not support `deny_unknown_fields` on an internally
        /// tagged enum, so the flat union is parsed strictly and every field
        /// that does not belong to the selected `busClass` variant is
        /// rejected explicitly.
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            bus_class: BusClass,
            label: BoundedToken,
            #[serde(default)]
            vendor_id: Option<HexId>,
            #[serde(default)]
            product_id: Option<HexId>,
            #[serde(default)]
            serial: Option<DeviceFilterText>,
            #[serde(default)]
            pci_slot: Option<DeviceFilterText>,
            #[serde(default)]
            slot: Option<DeviceFilterText>,
            #[serde(default)]
            index: Option<u8>,
        }
        let wire = Wire::deserialize(deserializer)?;
        let reject = |present: bool| {
            if present {
                Err(serde::de::Error::custom(
                    PrimitiveSpecError::ConflictingFields,
                ))
            } else {
                Ok(())
            }
        };
        let bounded_slot = |slot: Option<DeviceFilterText>| match slot {
            Some(slot) if slot.as_str().len() > MAX_PCI_SLOT_BYTES => {
                Err(serde::de::Error::custom(PrimitiveSpecError::InvalidText))
            }
            other => Ok(other),
        };
        match wire.bus_class {
            BusClass::Usb | BusClass::Hidraw => {
                reject(wire.pci_slot.is_some() || wire.slot.is_some() || wire.index.is_some())?;
                let label = wire.label;
                let vendor_id = wire.vendor_id;
                let product_id = wire.product_id;
                let serial = wire.serial;
                Ok(if wire.bus_class == BusClass::Usb {
                    Self::Usb {
                        label,
                        vendor_id,
                        product_id,
                        serial,
                    }
                } else {
                    Self::Hidraw {
                        label,
                        vendor_id,
                        product_id,
                        serial,
                    }
                })
            }
            BusClass::Drm => {
                reject(
                    wire.vendor_id.is_some()
                        || wire.product_id.is_some()
                        || wire.serial.is_some()
                        || wire.slot.is_some()
                        || wire.index.is_some(),
                )?;
                Ok(Self::Drm {
                    label: wire.label,
                    pci_slot: bounded_slot(wire.pci_slot)?,
                })
            }
            BusClass::Pci => {
                reject(
                    wire.vendor_id.is_some()
                        || wire.product_id.is_some()
                        || wire.serial.is_some()
                        || wire.pci_slot.is_some()
                        || wire.index.is_some(),
                )?;
                Ok(Self::Pci {
                    label: wire.label,
                    slot: bounded_slot(wire.slot)?,
                })
            }
            BusClass::Tpm => {
                reject(
                    wire.vendor_id.is_some()
                        || wire.product_id.is_some()
                        || wire.serial.is_some()
                        || wire.pci_slot.is_some()
                        || wire.slot.is_some(),
                )?;
                Ok(Self::Tpm {
                    label: wire.label,
                    index: wire.index.unwrap_or(0),
                })
            }
        }
    }
}

/// The closed inventory bus-class discriminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum BusClass {
    Usb,
    Hidraw,
    Drm,
    Pci,
    Tpm,
}

/// The physical or emulated device selector.
#[derive(Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct InventorySpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    selector: Option<InventorySelector>,
}

impl InventorySpec {
    /// Construct an inventory selector wrapper.
    pub const fn new(selector: Option<InventorySelector>) -> Self {
        Self { selector }
    }

    /// Borrow the selector.
    pub const fn selector(&self) -> Option<&InventorySelector> {
        self.selector.as_ref()
    }
}

redacted_debug!(InventorySpec);

impl<'de> Deserialize<'de> for InventorySpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            selector: Option<InventorySelector>,
        }
        Ok(Self::new(Wire::deserialize(deserializer)?.selector))
    }
}

/// The Device ResourceType base spec.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSpec {
    device_class: DeviceClass,
    arbitration: DeviceArbitration,
    max_concurrent_claims: u32,
    inventory: InventorySpec,
}

impl DeviceSpec {
    /// Construct a Device base spec after checking the arbitration and
    /// selector invariants.
    pub fn new(
        device_class: DeviceClass,
        arbitration: DeviceArbitration,
        max_concurrent_claims: u32,
        inventory: InventorySpec,
    ) -> Result<Self, PrimitiveSpecError> {
        if !(1..=MAX_CONCURRENT_CLAIMS).contains(&max_concurrent_claims) {
            return Err(PrimitiveSpecError::OutOfRange);
        }
        if arbitration == DeviceArbitration::Exclusive && max_concurrent_claims != 1 {
            return Err(PrimitiveSpecError::ConflictingFields);
        }
        match (device_class, inventory.selector()) {
            (DeviceClass::Emulated, Some(_)) => return Err(PrimitiveSpecError::ConflictingFields),
            (DeviceClass::Physical, None) => {
                return Err(PrimitiveSpecError::MissingRequiredField);
            }
            _ => {}
        }
        Ok(Self {
            device_class,
            arbitration,
            max_concurrent_claims,
            inventory,
        })
    }

    /// Construct the canonical minimal exclusive emulated Device base spec.
    pub fn emulated_exclusive() -> Self {
        Self::new(
            DeviceClass::Emulated,
            DeviceArbitration::Exclusive,
            1,
            InventorySpec::default(),
        )
        .expect("the minimal emulated Device spec is always valid")
    }

    /// Return the device class.
    pub const fn device_class(&self) -> DeviceClass {
        self.device_class
    }

    /// Return the arbitration mode.
    pub const fn arbitration(&self) -> DeviceArbitration {
        self.arbitration
    }

    /// Return the simultaneous claimant ceiling.
    pub const fn max_concurrent_claims(&self) -> u32 {
        self.max_concurrent_claims
    }

    /// Borrow the inventory selector.
    pub const fn inventory(&self) -> &InventorySpec {
        &self.inventory
    }
}

redacted_debug!(DeviceSpec);

impl<'de> Deserialize<'de> for DeviceSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            device_class: DeviceClass,
            arbitration: DeviceArbitration,
            #[serde(default = "one")]
            max_concurrent_claims: u32,
            inventory: InventorySpec,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.device_class,
            wire.arbitration,
            wire.max_concurrent_claims,
            wire.inventory,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Whether a Device is currently healthy enough for a claimant.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum DeviceHealth {
    /// Probe or emulation is healthy.
    Healthy,
    /// The Device remains usable with an impaired condition.
    Degraded,
    /// The Device is known not to be usable.
    Failed,
    /// The controller cannot currently prove the Device state.
    Unknown,
}

/// Claim mode recorded in the common Device status layer.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceClaimKind {
    /// The claimant owns the whole Device.
    Exclusive,
    /// The claimant consumes a read-only shared capability.
    ReadShared,
    /// The selected Provider owns the claim protocol.
    ProviderManaged,
}

/// One bounded Device claim status entry.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceClaim {
    holder_ref: ResourceRef,
    claim: DeviceClaimKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    passthrough: Option<BoundedToken>,
    claimed_at: Timestamp,
    health: DeviceHealth,
}

impl DeviceClaim {
    /// Construct one common claim entry.
    pub fn new(
        holder_ref: ResourceRef,
        claim: DeviceClaimKind,
        passthrough: Option<BoundedToken>,
        claimed_at: Timestamp,
        health: DeviceHealth,
    ) -> Result<Self, PrimitiveSpecError> {
        if !matches!(
            holder_ref.resource_type().as_str(),
            "Host" | "Guest" | "Process"
        ) {
            return Err(PrimitiveSpecError::WrongResourceType);
        }
        Ok(Self {
            holder_ref,
            claim,
            passthrough,
            claimed_at,
            health,
        })
    }

    /// Borrow the claimant reference.
    pub const fn holder_ref(&self) -> &ResourceRef {
        &self.holder_ref
    }

    /// Return the common claim mode.
    pub const fn claim(&self) -> DeviceClaimKind {
        self.claim
    }

    /// Borrow the optional provider-specific passthrough kind.
    pub const fn passthrough(&self) -> Option<&BoundedToken> {
        self.passthrough.as_ref()
    }

    /// Return the claim health.
    pub const fn health(&self) -> DeviceHealth {
        self.health
    }
}

redacted_debug!(DeviceClaim);

impl<'de> Deserialize<'de> for DeviceClaim {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            holder_ref: ResourceRef,
            claim: DeviceClaimKind,
            #[serde(default)]
            passthrough: Option<BoundedToken>,
            claimed_at: Timestamp,
            health: DeviceHealth,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.holder_ref,
            wire.claim,
            wire.passthrough,
            wire.claimed_at,
            wire.health,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// The common Device-specific status resource layer.
///
/// This object is placed in universal `status.resource`; it deliberately does
/// not duplicate `observedGeneration`, `phase`, `conditions`, or `update`.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceStatusResource {
    present: Option<bool>,
    health: DeviceHealth,
    holder_refs: Vec<ResourceRef>,
    claims: Vec<DeviceClaim>,
    provisioned_at: Option<Timestamp>,
    last_probed_at: Option<Timestamp>,
    provider_diagnostic: Option<StatusMessage>,
}

impl DeviceStatusResource {
    /// Construct and bound the common Device status projection.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        present: Option<bool>,
        health: DeviceHealth,
        mut holder_refs: Vec<ResourceRef>,
        claims: Vec<DeviceClaim>,
        provisioned_at: Option<Timestamp>,
        last_probed_at: Option<Timestamp>,
        provider_diagnostic: Option<StatusMessage>,
    ) -> Result<Self, PrimitiveSpecError> {
        if holder_refs.len() > MAX_DEVICE_HOLDER_REFS || claims.len() > MAX_DEVICE_CLAIMS {
            return Err(PrimitiveSpecError::TooManyEntries);
        }
        if holder_refs.iter().any(|reference| {
            !matches!(
                reference.resource_type().as_str(),
                "Host" | "Guest" | "Process"
            )
        }) {
            return Err(PrimitiveSpecError::WrongResourceType);
        }
        holder_refs.sort();
        let original_len = holder_refs.len();
        holder_refs.dedup();
        if holder_refs.len() != original_len {
            return Err(PrimitiveSpecError::DuplicateEntry);
        }
        Ok(Self {
            present,
            health,
            holder_refs,
            claims,
            provisioned_at,
            last_probed_at,
            provider_diagnostic,
        })
    }

    /// Return physical presence, or `None` before the first probe.
    pub const fn present(&self) -> Option<bool> {
        self.present
    }

    /// Return common Device health.
    pub const fn health(&self) -> DeviceHealth {
        self.health
    }

    /// Borrow the ordered holder references.
    pub fn holder_refs(&self) -> &[ResourceRef] {
        &self.holder_refs
    }

    /// Borrow the bounded claim entries.
    pub fn claims(&self) -> &[DeviceClaim] {
        &self.claims
    }
}

redacted_debug!(DeviceStatusResource);

impl<'de> Deserialize<'de> for DeviceStatusResource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Wire {
            #[serde(default)]
            present: Option<bool>,
            health: DeviceHealth,
            holder_refs: Vec<ResourceRef>,
            claims: Vec<DeviceClaim>,
            #[serde(default)]
            provisioned_at: Option<Timestamp>,
            #[serde(default)]
            last_probed_at: Option<Timestamp>,
            #[serde(default)]
            provider_diagnostic: Option<StatusMessage>,
        }
        let wire = Wire::deserialize(deserializer)?;
        Self::new(
            wire.present,
            wire.health,
            wire.holder_refs,
            wire.claims,
            wire.provisioned_at,
            wire.last_probed_at,
            wire.provider_diagnostic,
        )
        .map_err(serde::de::Error::custom)
    }
}

/// Stable Device-specific error codes.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceErrorCode {
    /// A physical selector did not resolve during a probe.
    DeviceNotFound,
    /// An exclusive Device already has a claimant.
    DeviceClaimConflict,
    /// The maximum number of claims was reached.
    DeviceClaimMaxExceeded,
    /// The requested claim did not match Device arbitration.
    DeviceArbitrationViolation,
    /// Provider provisioning of an emulated Device failed.
    DeviceProvisionFailed,
    /// Core could not open or pass the physical device effect.
    DeviceBrokerInaccessible,
    /// A tamper marker or state identity did not match.
    DeviceStateIntegrityFailure,
    /// A security-key lease exceeded its bounded lifetime.
    DeviceSessionTimeout,
    /// An operator or owner cancelled a security-key session.
    DeviceSessionCancelled,
    /// A USB or security-key authority already owns the physical backing.
    PhysicalUsbBackingConflict,
    /// An owned Process is not Ready.
    DeviceWorkerFailed,
    /// A GPU or video wire contract differs from the pinned value.
    DeviceWireContractMismatch,
}

impl DeviceErrorCode {
    /// Return the exact lower-kebab wire spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DeviceNotFound => "device-not-found",
            Self::DeviceClaimConflict => "device-claim-conflict",
            Self::DeviceClaimMaxExceeded => "device-claim-max-exceeded",
            Self::DeviceArbitrationViolation => "device-arbitration-violation",
            Self::DeviceProvisionFailed => "device-provision-failed",
            Self::DeviceBrokerInaccessible => "device-broker-inaccessible",
            Self::DeviceStateIntegrityFailure => "device-state-integrity-failure",
            Self::DeviceSessionTimeout => "device-session-timeout",
            Self::DeviceSessionCancelled => "device-session-cancelled",
            Self::PhysicalUsbBackingConflict => "physical-usb-backing-conflict",
            Self::DeviceWorkerFailed => "device-worker-failed",
            Self::DeviceWireContractMismatch => "device-wire-contract-mismatch",
        }
    }
}

/// Device RBAC verbs and subresources.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceResourceVerb {
    /// Read one Device.
    Get,
    /// Enumerate Devices in a Zone.
    List,
    /// Subscribe to Device changes.
    Watch,
    /// Create a Device.
    Create,
    /// Change desired Device spec.
    UpdateSpec,
    /// Delete a Device.
    Delete,
    /// Write only the Provider-owned status layer.
    UpdateStatus,
    /// Add or remove the Provider's typed finalizer.
    UpdateFinalizers,
}

impl DeviceResourceVerb {
    /// Return the exact lower-kebab authorization spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::List => "list",
            Self::Watch => "watch",
            Self::Create => "create",
            Self::UpdateSpec => "update-spec",
            Self::Delete => "delete",
            Self::UpdateStatus => "update-status",
            Self::UpdateFinalizers => "update-finalizers",
        }
    }
}

/// Closed Device effect operation classes used by Core's adapter.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceEffectOperation {
    /// Harden or verify a TPM state directory and marker.
    PrepareStateDir,
    /// Spawn one swtpm, USBIP, GPU, or video worker.
    SpawnRunner,
    /// Open one exact security-key hidraw device.
    SecurityKeyOpenDevice,
    /// Apply the activation-only security-key udev projection.
    SecurityKeyApplyUdevRules,
    /// Apply or remove one USBIP network projection.
    ApplyNftablesProjection,
    /// Open GPU device grants before worker clone.
    OpenDevice,
}

/// Conservative broker effect limits for Device Providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeviceEffectLimits {
    security_key_open_max_concurrent: u16,
    security_key_open_fd_quota: u8,
    security_key_udev_activation_only: bool,
    nftables_projection_batch_limit: u16,
    spawn_runner_tpm_per_device: u16,
    spawn_runner_gpu_per_device: u16,
    open_device_gpu_fd_quota: u8,
}

impl DeviceEffectLimits {
    /// The frozen conservative limits from the Device security model.
    pub const fn frozen() -> Self {
        Self {
            security_key_open_max_concurrent: 1,
            security_key_open_fd_quota: 1,
            security_key_udev_activation_only: true,
            nftables_projection_batch_limit: 1,
            spawn_runner_tpm_per_device: 1,
            spawn_runner_gpu_per_device: 1,
            open_device_gpu_fd_quota: 8,
        }
    }

    /// Maximum concurrent hidraw opens for one Device.
    pub const fn security_key_open_max_concurrent(self) -> u16 {
        self.security_key_open_max_concurrent
    }

    /// FD quota for one security-key open.
    pub const fn security_key_open_fd_quota(self) -> u8 {
        self.security_key_open_fd_quota
    }

    /// Whether udev application is activation-only.
    pub const fn security_key_udev_activation_only(self) -> bool {
        self.security_key_udev_activation_only
    }

    /// Maximum nftables projection batch size.
    pub const fn nftables_projection_batch_limit(self) -> u16 {
        self.nftables_projection_batch_limit
    }

    /// Maximum GPU open-device fd quota per spawn.
    pub const fn open_device_gpu_fd_quota(self) -> u8 {
        self.open_device_gpu_fd_quota
    }
}

/// Named effect-limit constants for adapters that do not need the aggregate.
pub const DEVICE_SECURITY_KEY_OPEN_MAX_CONCURRENT: u16 = 1;
/// Security-key open returns at most one fd.
pub const DEVICE_SECURITY_KEY_OPEN_FD_QUOTA: u8 = 1;
/// One bounded projection batch is admitted per acquisition or release.
pub const DEVICE_NFTABLES_PROJECTION_BATCH_LIMIT: u16 = 1;
/// One swtpm runner launch is admitted per Device/start cycle.
pub const DEVICE_TPM_SPAWN_PER_DEVICE: u16 = 1;
/// One GPU/video worker set is admitted per Device.
pub const DEVICE_GPU_SPAWN_PER_DEVICE: u16 = 1;
/// GPU worker launches may receive at most eight opened device fds.
pub const DEVICE_GPU_OPEN_DEVICE_FD_QUOTA: u8 = 8;

/// Closed semantic Device metric operation labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceMetricOperation {
    Reconcile,
    Probe,
    Claim,
    Finalize,
    Effect,
}

impl DeviceMetricOperation {
    /// Return the fixed metric label value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reconcile => "reconcile",
            Self::Probe => "probe",
            Self::Claim => "claim",
            Self::Finalize => "finalize",
            Self::Effect => "effect",
        }
    }
}

/// Closed semantic Device metric outcome labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DeviceMetricOutcome {
    Success,
    Retry,
    Blocked,
}

impl DeviceMetricOutcome {
    /// Return the fixed metric label value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Retry => "retry",
            Self::Blocked => "blocked",
        }
    }
}

/// Fixed Device metric labels. Zone, resource, UID, selector, and backing
/// identity never occur in this struct; `d2b.zone` and `d2b.provider` belong
/// only to the OTEL resource-attribute set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceMetricLabels {
    /// Fixed Provider family label.
    pub provider: &'static str,
    /// Fixed semantic component label.
    pub component: &'static str,
    /// Closed operation label.
    pub operation: &'static str,
    /// Closed outcome label.
    pub outcome: &'static str,
    /// Closed error label or `none`.
    pub error: &'static str,
}

impl DeviceMetricLabels {
    /// Construct labels using only closed semantic values.
    pub const fn new(
        operation: DeviceMetricOperation,
        outcome: DeviceMetricOutcome,
        error: Option<DeviceErrorCode>,
    ) -> Self {
        Self {
            provider: "device",
            component: "controller",
            operation: operation.as_str(),
            outcome: outcome.as_str(),
            error: match error {
                Some(error) => error.as_str(),
                None => "none",
            },
        }
    }
}

/// OTEL resource attributes retained for Device spans. These are attributes,
/// not metric labels, and their keys are fixed by the telemetry contract.
pub const DEVICE_OTEL_RESOURCE_ATTRIBUTES: [&str; 2] = ["d2b.zone", "d2b.provider"];

/// Device telemetry contract version.
pub const DEVICE_TELEMETRY_CONTRACT_VERSION: &str = "device-telemetry/v1";

const fn one() -> u32 {
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v3::{execution_policy::to_base_object, resource_schema::canonical_json_bytes};

    const MINIMAL_DEVICE_SPEC: &[u8] =
        br#"{"arbitration":"exclusive","deviceClass":"emulated","inventory":{},"maxConcurrentClaims":1}"#;

    #[test]
    fn schema_vector_pins_the_minimal_emulated_device_base_spec() {
        let spec = DeviceSpec::emulated_exclusive();
        assert_eq!(canonical_json_bytes(&spec).unwrap(), MINIMAL_DEVICE_SPEC);
        let parsed: DeviceSpec = serde_json::from_slice(MINIMAL_DEVICE_SPEC).unwrap();
        assert_eq!(parsed, spec);
        let base = to_base_object(&spec).unwrap();
        for reserved in ["providerRef", "updatePolicy", "provider", "settings"] {
            assert!(base.get(reserved).is_none());
        }
    }

    #[test]
    fn exclusive_arbitration_pins_a_single_claimant() {
        assert_eq!(
            DeviceSpec::new(
                DeviceClass::Emulated,
                DeviceArbitration::Exclusive,
                2,
                InventorySpec::default(),
            ),
            Err(PrimitiveSpecError::ConflictingFields)
        );
        assert_eq!(
            DeviceSpec::new(
                DeviceClass::Emulated,
                DeviceArbitration::Shared,
                17,
                InventorySpec::default(),
            ),
            Err(PrimitiveSpecError::OutOfRange)
        );
        assert!(
            DeviceSpec::new(
                DeviceClass::Emulated,
                DeviceArbitration::Shared,
                4,
                InventorySpec::default(),
            )
            .is_ok()
        );
    }

    #[test]
    fn emulated_devices_carry_no_selector_and_physical_devices_require_one() {
        let usb = InventorySpec::new(Some(InventorySelector::Usb {
            label: BoundedToken::parse("yubikey-work").unwrap(),
            vendor_id: Some(HexId::parse("1050").unwrap()),
            product_id: Some(HexId::parse("0407").unwrap()),
            serial: None,
        }));
        assert_eq!(
            DeviceSpec::new(
                DeviceClass::Emulated,
                DeviceArbitration::Exclusive,
                1,
                usb.clone()
            ),
            Err(PrimitiveSpecError::ConflictingFields)
        );
        assert_eq!(
            DeviceSpec::new(
                DeviceClass::Physical,
                DeviceArbitration::Exclusive,
                1,
                InventorySpec::default(),
            ),
            Err(PrimitiveSpecError::MissingRequiredField)
        );
        assert!(
            DeviceSpec::new(DeviceClass::Physical, DeviceArbitration::Exclusive, 1, usb).is_ok()
        );
    }

    #[test]
    fn the_selector_union_is_closed_and_carries_no_raw_device_path() {
        for rejected in [
            br#"{"busClass":"serial","label":"x"}"#.as_slice(),
            br#"{"busClass":"usb","label":"x","path":"/dev/bus/usb/001/002"}"#,
            br#"{"busClass":"usb","label":"x","pciSlot":"0000:01:00.0"}"#,
            br#"{"busClass":"drm","label":"x","vendorId":"1050"}"#,
            br#"{"label":"x"}"#,
        ] {
            assert!(serde_json::from_slice::<InventorySelector>(rejected).is_err());
        }
        assert!(
            serde_json::from_slice::<InventorySelector>(
                br#"{"busClass":"drm","label":"host-gpu","pciSlot":"0000:01:00.0"}"#
            )
            .is_ok()
        );
        assert!(HexId::parse("1050").is_ok());
        assert!(HexId::parse("1A50").is_err());
        assert!(HexId::parse("105").is_err());
        assert!(DeviceFilterText::parse("../../dev/null").is_err());
    }

    #[test]
    fn diagnostics_stay_redacted() {
        assert_eq!(
            format!("{:?}", DeviceSpec::emulated_exclusive()),
            "DeviceSpec(<redacted>)"
        );
    }
}
