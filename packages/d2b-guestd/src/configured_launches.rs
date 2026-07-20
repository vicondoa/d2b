//! Runtime adaptation for the shared configured-launch catalog.

use std::{collections::BTreeMap, fmt};

use d2b_contracts::v2_guest_configured_launches::{
    GuestConfiguredLaunchesV1, MAX_GUEST_CONFIGURED_LAUNCHES_BYTES,
};
use sha2::{Digest, Sha256};

use crate::service_v2::{GuestSessionError, SystemdGuestCredentialSource};

pub const CONFIGURED_LAUNCH_CREDENTIAL: &str = "d2b-guest-configured-launches-v2";

#[derive(Clone)]
pub struct ConfiguredLaunchInventory {
    realm_id: String,
    workload_id: String,
    workload_digest: [u8; 32],
    entries: BTreeMap<String, Vec<String>>,
}

impl fmt::Debug for ConfiguredLaunchInventory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConfiguredLaunchInventory")
            .field("identity", &"<redacted>")
            .field("entries", &self.entries.len())
            .field("content", &"<redacted>")
            .finish()
    }
}

impl ConfiguredLaunchInventory {
    pub fn load(
        source: &SystemdGuestCredentialSource,
        expected_sha256: [u8; 32],
        expected_workload_id: &str,
    ) -> Result<Self, GuestSessionError> {
        if expected_sha256 == [0; 32] {
            return Err(GuestSessionError::InvalidConfiguration);
        }
        let credential = source.load_named(
            CONFIGURED_LAUNCH_CREDENTIAL,
            MAX_GUEST_CONFIGURED_LAUNCHES_BYTES as u64,
        )?;
        if Sha256::digest(credential.expose()).as_slice() != expected_sha256 {
            return Err(GuestSessionError::InvalidConfiguration);
        }
        let decoded = GuestConfiguredLaunchesV1::decode(credential.expose())
            .map_err(|_| GuestSessionError::InvalidConfiguration)?;
        if decoded.workload_id().as_str() != expected_workload_id {
            return Err(GuestSessionError::InvalidConfiguration);
        }
        let entries = decoded
            .entries()
            .iter()
            .map(|entry| {
                (
                    entry.item_id().as_str().to_owned(),
                    entry.argv().as_slice().to_vec(),
                )
            })
            .collect();
        Ok(Self {
            realm_id: decoded.realm_id().as_str().to_owned(),
            workload_id: decoded.workload_id().as_str().to_owned(),
            workload_digest: *decoded.workload_digest(),
            entries,
        })
    }

    pub fn realm_id(&self) -> &str {
        &self.realm_id
    }

    pub fn workload_id(&self) -> &str {
        &self.workload_id
    }

    pub const fn workload_digest(&self) -> &[u8; 32] {
        &self.workload_digest
    }

    pub fn into_entries(self) -> BTreeMap<String, Vec<String>> {
        self.entries
    }
}
