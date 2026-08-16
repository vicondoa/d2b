//! Core-owned USBIP identity and bundle adapter.
//!
//! This module is deliberately independent from the public contracts crate:
//! `d2b-contracts` depends on `d2b-core` for canonical identity/error types.
//! The daemon bridges these Core projections to the typed USBIP effect ports.
//! It is the only place that derives physical and relay authority keys from
//! trusted bundle data.

use sha2::{Digest, Sha256};

use crate::bundle_resolver::BundleResolver;

/// Domain used for the Host-global physical USB backing digest.
pub const PHYSICAL_USB_BACKING_DOMAIN: &str = "d2b:physical-usb-backing/v1";
/// Domain used for the per-Network USBIP relay authority digest.
pub const USBIP_NETWORK_RELAY_DOMAIN: &str = "d2b:usbip-network-relay/v1";
/// Domain used for the host usbip module authority digest.
pub const USBIP_HOST_MODULE_DOMAIN: &str = "d2b:usbip-host-module/v1";

/// Core adapter failures.  No variant carries caller-controlled identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbipCoreAdapterError {
    /// The requested and declared Zones differ.
    WrongZone,
    /// The owning Zone did not opt into USBIP.
    ZoneNotOptedIn,
    /// The target VM is absent from the trusted manifest.
    VmNotFound,
    /// The target VM has no trusted environment.
    EnvironmentMissing,
    /// The bus id shape is not safe for bundle lookup.
    BusIdInvalid,
    /// The trusted bundle does not contain the exact bind and firewall rows.
    BundleIntentMissing,
    /// The installed bundle generation is unavailable.
    GenerationMissing,
    /// The host bundle does not prove the scoped anti-spoofing topology.
    AntiSpoofUnproven,
    /// A caller tried to use a Provider-private authority class.
    ProviderClassBypass,
}

impl UsbipCoreAdapterError {
    /// Return the stable identity-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::WrongZone => "wrong-zone",
            Self::ZoneNotOptedIn => "zone-not-opted-in",
            Self::VmNotFound => "usbip-vm-not-found",
            Self::EnvironmentMissing => "usbip-environment-missing",
            Self::BusIdInvalid => "usbip-bus-id-invalid",
            Self::BundleIntentMissing => "usbip-bundle-intent-missing",
            Self::GenerationMissing => "usbip-generation-missing",
            Self::AntiSpoofUnproven => "usbip-anti-spoof-unproven",
            Self::ProviderClassBypass => "usbip-provider-class-bypass",
        }
    }
}

impl core::fmt::Display for UsbipCoreAdapterError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for UsbipCoreAdapterError {}

/// Opaque 32-byte authority key produced by Core.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UsbipAuthorityKey([u8; 32]);

impl UsbipAuthorityKey {
    /// Borrow the key for a typed authority adapter.
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl core::fmt::Debug for UsbipAuthorityKey {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("UsbipAuthorityKey(<redacted>)")
    }
}

/// Exact trusted bundle projection for one USBIP Binding.
#[derive(Clone, PartialEq, Eq)]
pub struct UsbipBindingProjection {
    env: String,
    vm: String,
    firewall_intent_ref: String,
    bind_intent_ref: String,
    generation_id: String,
}

impl UsbipBindingProjection {
    /// Return the trusted environment route.
    pub fn env(&self) -> &str {
        &self.env
    }

    /// Return the trusted VM route.
    pub fn vm(&self) -> &str {
        &self.vm
    }

    /// Return the opaque firewall intent reference.
    pub fn firewall_intent_ref(&self) -> &str {
        &self.firewall_intent_ref
    }

    /// Return the opaque bind intent reference.
    pub fn bind_intent_ref(&self) -> &str {
        &self.bind_intent_ref
    }

    /// Return the installed-generation fence.
    pub fn generation_id(&self) -> &str {
        &self.generation_id
    }
}

impl core::fmt::Debug for UsbipBindingProjection {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("UsbipBindingProjection")
            .field("has_firewall_intent", &true)
            .field("has_bind_intent", &true)
            .field("has_generation", &true)
            .finish()
    }
}

/// Core adapter over a trusted, already-verified bundle.
#[derive(Clone)]
pub struct UsbipCoreAdapter {
    resolver: BundleResolver,
}

impl UsbipCoreAdapter {
    /// Construct the adapter from the daemon or broker's trusted resolver.
    pub fn new(resolver: BundleResolver) -> Self {
        Self { resolver }
    }

    /// Validate Zone opt-in before any bundle lookup or effect preparation.
    pub fn validate_zone(
        requested_zone: &str,
        declared_zone: &str,
        zone_opted_in: bool,
    ) -> Result<(), UsbipCoreAdapterError> {
        if requested_zone != declared_zone {
            return Err(UsbipCoreAdapterError::WrongZone);
        }
        if !zone_opted_in {
            return Err(UsbipCoreAdapterError::ZoneNotOptedIn);
        }
        Ok(())
    }

    /// Resolve one Binding only after Zone opt-in and exact Zone agreement.
    ///
    /// All checks happen before a caller can obtain an intent reference or
    /// invoke a broker effect.
    pub fn resolve_binding(
        &self,
        requested_zone: &str,
        declared_zone: &str,
        zone_opted_in: bool,
        vm: &str,
        bus_id: &str,
    ) -> Result<UsbipBindingProjection, UsbipCoreAdapterError> {
        Self::validate_zone(requested_zone, declared_zone, zone_opted_in)?;
        if !valid_bus_id(bus_id) {
            return Err(UsbipCoreAdapterError::BusIdInvalid);
        }

        let entry = self
            .resolver
            .manifest
            .vms
            .get(vm)
            .ok_or(UsbipCoreAdapterError::VmNotFound)?;
        let env = entry
            .env
            .as_deref()
            .ok_or(UsbipCoreAdapterError::EnvironmentMissing)?;

        let firewall_intent_ref = crate::bundle_resolver::intent_id_usbip_firewall(env, bus_id);
        let bind_intent_ref = crate::bundle_resolver::intent_id_usbip_bind(env, vm, bus_id);
        if self
            .resolver
            .find_usbip_firewall_intent(&firewall_intent_ref)
            .is_none()
            || self
                .resolver
                .find_usbip_bind_intent(&bind_intent_ref)
                .is_none()
        {
            return Err(UsbipCoreAdapterError::BundleIntentMissing);
        }

        let net = self
            .resolver
            .host
            .environments
            .iter()
            .find(|candidate| candidate.env == env)
            .ok_or(UsbipCoreAdapterError::EnvironmentMissing)?;
        if net.host_uplink_ip.is_none() || net.net_uplink_ip.is_none() {
            return Err(UsbipCoreAdapterError::AntiSpoofUnproven);
        }
        let generation_id = self
            .resolver
            .installed_generation_identity()
            .ok_or(UsbipCoreAdapterError::GenerationMissing)?
            .as_str()
            .to_owned();

        Ok(UsbipBindingProjection {
            env: env.to_owned(),
            vm: vm.to_owned(),
            firewall_intent_ref,
            bind_intent_ref,
            generation_id,
        })
    }

    /// Derive the exact physical backing key shared by USBIP and security-key
    /// Providers for one Core-observed physical identity.
    pub fn physical_usb_backing_key(identity: &[u8]) -> UsbipAuthorityKey {
        UsbipAuthorityKey(hash(PHYSICAL_USB_BACKING_DOMAIN, identity))
    }

    /// Derive the one shared relay authority key for a Network.
    pub fn network_relay_key(network_identity: &[u8]) -> UsbipAuthorityKey {
        UsbipAuthorityKey(hash(USBIP_NETWORK_RELAY_DOMAIN, network_identity))
    }

    /// Derive the one Host usbip module authority key.
    pub fn host_module_key() -> UsbipAuthorityKey {
        UsbipAuthorityKey(hash(USBIP_HOST_MODULE_DOMAIN, b"usbip-host"))
    }

    /// Reject a provider-private authority class from a public projection.
    pub fn validate_provider_class(provider_class: &str) -> Result<(), UsbipCoreAdapterError> {
        match provider_class {
            "device-usbip" | "device-security-key" => Ok(()),
            _ => Err(UsbipCoreAdapterError::ProviderClassBypass),
        }
    }
}

fn hash(domain: &str, identity: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain.as_bytes());
    hasher.update([0]);
    hasher.update((identity.len() as u64).to_be_bytes());
    hasher.update(identity);
    hasher.finalize().into()
}

fn valid_bus_id(bus_id: &str) -> bool {
    if bus_id.is_empty() || bus_id.len() > 31 {
        return false;
    }
    let segment_ok = |segment: &str| {
        !segment.is_empty()
            && segment.bytes().all(|byte| byte.is_ascii_digit())
            && !(segment.len() > 1 && segment.starts_with('0'))
    };
    match bus_id.split_once('-') {
        None => segment_ok(bus_id),
        Some((bus, ports)) if segment_ok(bus) => ports.split('.').all(segment_ok),
        Some(_) => false,
    }
}
