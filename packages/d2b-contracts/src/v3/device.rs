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

use super::execution_policy::{
    BoundedToken, PrimitiveSpecError, parsed_deserialize, redacted_debug, string_schema,
};

/// The canonical ResourceType name for this module.
pub const DEVICE_RESOURCE_TYPE: &str = "Device";
/// Maximum simultaneous claimants on one Device.
pub const MAX_CONCURRENT_CLAIMS: u32 = 16;
/// Maximum bytes in one device serial filter.
pub const MAX_DEVICE_SERIAL_BYTES: usize = 128;
/// Maximum bytes in one PCI slot filter.
pub const MAX_PCI_SLOT_BYTES: usize = 31;

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
