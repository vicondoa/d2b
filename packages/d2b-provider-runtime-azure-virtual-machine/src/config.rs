//! Bounded Azure VM configuration and Guest settings.

use std::fmt;

use d2b_contracts::v3::{ResourceRef, credential::OpaqueAzureRef};
use serde::{Deserialize, Serialize};

use crate::error::AzureVmError;

/// Maximum operator tags.
pub const MAX_AZURE_TAGS: usize = 50;
/// Maximum data disks.
pub const MAX_DATA_DISKS: usize = 16;

/// Azure disk SKU allowlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum DiskSku {
    /// Premium SSD.
    PremiumLrs,
    /// Standard SSD.
    StandardSsdLrs,
    /// Standard HDD.
    StandardLrs,
    /// Ultra SSD.
    UltraSsdLrs,
}

/// One provider-owned data disk intent.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DataDiskSpec {
    /// Azure LUN.
    pub lun: u8,
    /// Optional bounded class label.
    pub disk_class: Option<OpaqueAzureRef>,
    /// Size in GiB.
    pub size_gb: u32,
    /// Optional bounded display label.
    pub label: Option<String>,
}

impl DataDiskSpec {
    /// Validate one data disk intent.
    pub fn validate(&self) -> Result<(), AzureVmError> {
        if self.size_gb == 0 || self.size_gb > 32_767 {
            return Err(AzureVmError::InvalidConfiguration);
        }
        if self
            .label
            .as_ref()
            .is_some_and(|label| label.is_empty() || label.len() > 64 || !valid_label(label))
        {
            return Err(AzureVmError::InvalidConfiguration);
        }
        Ok(())
    }
}

impl fmt::Debug for DataDiskSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DataDiskSpec")
            .field("lun", &self.lun)
            .field("disk_class", &self.disk_class)
            .field("size_gb", &self.size_gb)
            .field("label", &self.label.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// How the one-time bootstrap PSK reaches the VM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum BootstrapPskDelivery {
    /// Use an ARM VM extension.
    VmExtension,
    /// Use cloud-init user data.
    UserData,
}

/// Provider root configuration.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AzureVmConfig {
    /// Optional tenant identifier for Entra-backed credentials.
    pub tenant_id: Option<OpaqueAzureRef>,
    /// Optional managed identity client identifier.
    pub client_id: Option<OpaqueAzureRef>,
    /// ARM Credential resource reference.
    pub arm_credential_ref: ResourceRef,
    /// Gateway Guest where controller and bootstrap service run.
    pub controller_execution_ref: ResourceRef,
    /// Optional gateway egress Network.
    pub network_ref: Option<ResourceRef>,
}

impl AzureVmConfig {
    /// Validate the root configuration and its gateway boundary.
    pub fn validate(&self) -> Result<(), AzureVmError> {
        if self.arm_credential_ref.resource_type().as_str() != "Credential"
            || self.controller_execution_ref.resource_type().as_str() != "Guest"
            || self
                .network_ref
                .as_ref()
                .is_some_and(|reference| reference.resource_type().as_str() != "Network")
        {
            return Err(AzureVmError::InvalidConfiguration);
        }
        Ok(())
    }

    /// Require a Credential scope to equal the gateway Guest exactly.
    pub fn validate_credential_scope(
        &self,
        execution_ref: &ResourceRef,
    ) -> Result<(), AzureVmError> {
        if execution_ref != &self.controller_execution_ref {
            return Err(AzureVmError::InvalidConfiguration);
        }
        Ok(())
    }
}

impl fmt::Debug for AzureVmConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureVmConfig")
            .field("tenant_id", &self.tenant_id)
            .field("client_id", &self.client_id)
            .field("arm_credential_ref", &"<redacted>")
            .field("controller_execution_ref", &"<redacted>")
            .field(
                "network_ref",
                &self.network_ref.as_ref().map(|_| "<configured>"),
            )
            .finish()
    }
}

/// Per-Guest Azure VM settings.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AzureVmGuestSettings {
    /// Azure subscription.
    pub subscription_id: OpaqueAzureRef,
    /// Resource group.
    pub resource_group: OpaqueAzureRef,
    /// Azure region.
    pub region: OpaqueAzureRef,
    /// VM size.
    pub vm_size: OpaqueAzureRef,
    /// Image reference.
    pub image_ref: OpaqueAzureRef,
    /// OS disk SKU.
    pub disk_sku: DiskSku,
    /// Optional OS disk size.
    pub os_disk_size_gb: Option<u32>,
    /// ARM admin username; not an SSH authority.
    pub admin_user: String,
    /// Optional VNet subscription.
    pub vnet_subscription_id: Option<OpaqueAzureRef>,
    /// Optional VNet resource group.
    pub vnet_resource_group: Option<OpaqueAzureRef>,
    /// VNet name.
    pub vnet_name: OpaqueAzureRef,
    /// Subnet name.
    pub subnet_name: OpaqueAzureRef,
    /// Whether a public IP is requested.
    pub assign_public_ip: bool,
    /// Provider-owned data disks.
    pub data_disks: Vec<DataDiskSpec>,
    /// Bootstrap delivery mode.
    pub bootstrap_psk_delivery: BootstrapPskDelivery,
    /// Bootstrap deadline in milliseconds.
    pub bootstrap_deadline_ms: u64,
    /// Whether the VM hosts a child Zone.
    pub child_zone_hosting: bool,
    /// Operator tags.
    pub azure_tags: Vec<(String, String)>,
}

impl AzureVmGuestSettings {
    /// Validate all bounds and closed sets.
    pub fn validate(&self) -> Result<(), AzureVmError> {
        if self
            .os_disk_size_gb
            .is_some_and(|size| !(30..=4095).contains(&size))
            || self.admin_user.is_empty()
            || self.admin_user.len() > 64
            || !valid_admin_user(&self.admin_user)
            || self.data_disks.len() > MAX_DATA_DISKS
            || !(60_000..=3_600_000).contains(&self.bootstrap_deadline_ms)
            || self.azure_tags.len() > MAX_AZURE_TAGS
        {
            return Err(AzureVmError::InvalidConfiguration);
        }
        let mut luns = [false; 64];
        for disk in &self.data_disks {
            disk.validate()?;
            let slot = usize::from(disk.lun);
            if slot >= luns.len() || luns[slot] {
                return Err(AzureVmError::InvalidConfiguration);
            }
            luns[slot] = true;
        }
        for (key, value) in &self.azure_tags {
            if key.is_empty() || key.len() > 512 || value.len() > 256 || key.starts_with("d2b:") {
                return Err(AzureVmError::InvalidConfiguration);
            }
        }
        Ok(())
    }
}

impl fmt::Debug for AzureVmGuestSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AzureVmGuestSettings")
            .field("subscription_id", &self.subscription_id)
            .field("resource_group", &self.resource_group)
            .field("region", &self.region)
            .field("vm_size", &self.vm_size)
            .field("image_ref", &self.image_ref)
            .field("disk_sku", &self.disk_sku)
            .field("os_disk_size_gb", &self.os_disk_size_gb)
            .field("admin_user", &"<redacted>")
            .field("vnet_subscription_id", &self.vnet_subscription_id)
            .field("vnet_resource_group", &self.vnet_resource_group)
            .field("vnet_name", &self.vnet_name)
            .field("subnet_name", &self.subnet_name)
            .field("assign_public_ip", &self.assign_public_ip)
            .field("data_disks", &self.data_disks.len())
            .field("bootstrap_psk_delivery", &self.bootstrap_psk_delivery)
            .field("bootstrap_deadline_ms", &self.bootstrap_deadline_ms)
            .field("child_zone_hosting", &self.child_zone_hosting)
            .field("azure_tags", &self.azure_tags.len())
            .finish()
    }
}

fn valid_admin_user(value: &str) -> bool {
    let mut chars = value.bytes();
    matches!(chars.next(), Some(b'a'..=b'z') | Some(b'_'))
        && chars.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
}

fn valid_label(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}
