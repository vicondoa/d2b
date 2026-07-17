use std::{
    collections::BTreeMap,
    fmt,
    fs::File,
    io::Read,
    sync::{Arc, Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use d2b_contracts::{
    v2_identity::ProviderId,
    v2_provider::{Generation, LeaseId, MAX_SAFE_JSON_INTEGER, SourceVersion},
};
use d2b_provider_credential_secret_service::{
    Oo7SecretServicePort, SecretServiceLeaseGrant, SecretServiceLeaseInspection,
    SecretServiceLeaseRef, SecretServiceLeaseRenewal, SecretServiceLeaseRequest,
    SecretServiceLeaseRevocation, SecretServiceLeaseState, SecretServicePortError,
    SecretServiceState,
};
use zeroize::Zeroizing;

use super::identity::{
    ClosedOutcome, OwnerBinding, SecretMetricEvent, SecretMetricSink, SecretOperation,
};

const MAX_SECRET_BYTES: usize = 64 * 1024;
const MAX_PORT_LEASES: usize = 256;
const ENTROPY_BYTES: usize = 16;

pub trait UserdClock: Send + Sync {
    fn now_unix_ms(&self) -> u64;
}

#[derive(Debug, Default)]
pub struct SystemUserdClock;

impl UserdClock for SystemUserdClock {
    fn now_unix_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| {
                u64::try_from(duration.as_millis())
                    .unwrap_or(MAX_SAFE_JSON_INTEGER)
                    .min(MAX_SAFE_JSON_INTEGER)
            })
    }
}

pub trait EntropySource: Send + Sync {
    fn fill(&self, destination: &mut [u8]) -> Result<(), UserSecretEntropyError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserSecretEntropyError;

#[derive(Debug, Default)]
pub struct OsEntropy;

impl EntropySource for OsEntropy {
    fn fill(&self, destination: &mut [u8]) -> Result<(), UserSecretEntropyError> {
        File::open("/dev/urandom")
            .and_then(|mut source| source.read_exact(destination))
            .map_err(|_| UserSecretEntropyError)
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct OwnedSecretSelector {
    owner: OwnerBinding,
    provider_id: ProviderId,
}

impl OwnedSecretSelector {
    pub fn new(owner: OwnerBinding, provider_id: ProviderId) -> Self {
        Self { owner, provider_id }
    }

    pub fn owner(&self) -> &OwnerBinding {
        &self.owner
    }

    pub fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }
}

impl fmt::Debug for OwnedSecretSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedSecretSelector")
            .field("owner", &"<redacted>")
            .field("provider_id", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct OwnedSecretMetadata {
    pub source_version: SourceVersion,
    pub rotation_generation: Generation,
    pub expires_at_unix_ms: u64,
}

impl fmt::Debug for OwnedSecretMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedSecretMetadata")
            .field("source_version", &"<redacted>")
            .field("rotation_generation", &self.rotation_generation)
            .field("expires_at_unix_ms", &self.expires_at_unix_ms)
            .finish()
    }
}

pub struct SecretMaterial(Zeroizing<Vec<u8>>);

impl SecretMaterial {
    pub fn new(value: Vec<u8>) -> Result<Self, SecretStoreError> {
        if value.is_empty() || value.len() > MAX_SECRET_BYTES {
            return Err(SecretStoreError::InvalidData);
        }
        Ok(Self(Zeroizing::new(value)))
    }

    pub fn expose<R>(&self, use_secret: impl FnOnce(&[u8]) -> R) -> R {
        use_secret(self.0.as_slice())
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn into_bytes(self) -> Zeroizing<Vec<u8>> {
        self.0
    }
}

impl fmt::Debug for SecretMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretMaterial(<redacted>)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretStoreError {
    Locked,
    NotFound,
    Duplicate,
    Denied,
    InvalidData,
    Unavailable,
}

#[async_trait]
pub trait SecretStore: Send + Sync {
    async fn state(&self) -> Result<SecretServiceState, SecretStoreError>;

    async fn put_owned(
        &self,
        selector: &OwnedSecretSelector,
        metadata: &OwnedSecretMetadata,
        secret: SecretMaterial,
    ) -> Result<(), SecretStoreError>;

    async fn metadata(
        &self,
        selector: &OwnedSecretSelector,
    ) -> Result<OwnedSecretMetadata, SecretStoreError>;

    async fn read_owned(
        &self,
        selector: &OwnedSecretSelector,
    ) -> Result<(OwnedSecretMetadata, SecretMaterial), SecretStoreError>;

    async fn delete_owned(&self, selector: &OwnedSecretSelector) -> Result<bool, SecretStoreError>;
}

#[cfg(feature = "secret-service")]
mod oo7_store {
    use std::{collections::BTreeMap, sync::Arc};

    use oo7::dbus::{Collection, Error as Oo7Error, Service, ServiceError};

    use super::{
        OwnedSecretMetadata, OwnedSecretSelector, SecretMaterial, SecretStore, SecretStoreError,
    };
    use async_trait::async_trait;
    use d2b_contracts::v2_provider::{Generation, SourceVersion};

    const SCHEMA_ATTRIBUTE: &str = "xdg:schema";
    const SCHEMA_VALUE: &str = "org.d2b.UserCredential.v2";
    const OWNER_ATTRIBUTE: &str = "d2b:owner";
    const OWNER_VALUE: &str = "d2b-userd";
    const SELECTOR_VERSION_ATTRIBUTE: &str = "d2b:selector-version";
    const SELECTOR_VERSION: &str = "1";
    const UID_ATTRIBUTE: &str = "d2b:uid";
    const REALM_ATTRIBUTE: &str = "d2b:realm-id";
    const PROVIDER_ATTRIBUTE: &str = "d2b:provider-id";
    const SOURCE_VERSION_ATTRIBUTE: &str = "d2b:source-version";
    const ROTATION_ATTRIBUTE: &str = "d2b:rotation-generation";
    const EXPIRY_ATTRIBUTE: &str = "d2b:expires-at-unix-ms";

    pub struct Oo7SecretStore {
        owner: super::OwnerBinding,
        collection: Arc<Collection>,
    }

    impl std::fmt::Debug for Oo7SecretStore {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .debug_struct("Oo7SecretStore")
                .field("owner", &"<redacted>")
                .finish_non_exhaustive()
        }
    }

    impl Oo7SecretStore {
        pub async fn connect(owner: super::OwnerBinding) -> Result<Self, SecretStoreError> {
            let service = Service::new().await.map_err(map_oo7_error)?;
            let collection = service
                .with_alias(Service::DEFAULT_COLLECTION)
                .await
                .map_err(map_oo7_error)?
                .ok_or(SecretStoreError::Unavailable)?;
            if collection.is_locked().await.map_err(map_oo7_error)? {
                return Err(SecretStoreError::Locked);
            }
            Ok(Self {
                owner,
                collection: Arc::new(collection),
            })
        }

        fn authorize_selector(
            &self,
            selector: &OwnedSecretSelector,
        ) -> Result<(), SecretStoreError> {
            if selector.owner() != &self.owner {
                return Err(SecretStoreError::Denied);
            }
            Ok(())
        }

        fn selector_attributes(selector: &OwnedSecretSelector) -> BTreeMap<String, String> {
            BTreeMap::from([
                (SCHEMA_ATTRIBUTE.to_owned(), SCHEMA_VALUE.to_owned()),
                (OWNER_ATTRIBUTE.to_owned(), OWNER_VALUE.to_owned()),
                (
                    SELECTOR_VERSION_ATTRIBUTE.to_owned(),
                    SELECTOR_VERSION.to_owned(),
                ),
                (UID_ATTRIBUTE.to_owned(), selector.owner().uid().to_string()),
                (
                    REALM_ATTRIBUTE.to_owned(),
                    selector.owner().realm_id().as_str().to_owned(),
                ),
                (
                    PROVIDER_ATTRIBUTE.to_owned(),
                    selector.provider_id().as_str().to_owned(),
                ),
            ])
        }

        fn all_attributes(
            selector: &OwnedSecretSelector,
            metadata: &OwnedSecretMetadata,
        ) -> BTreeMap<String, String> {
            let mut attributes = Self::selector_attributes(selector);
            attributes.insert(
                SOURCE_VERSION_ATTRIBUTE.to_owned(),
                metadata.source_version.as_str().to_owned(),
            );
            attributes.insert(
                ROTATION_ATTRIBUTE.to_owned(),
                metadata.rotation_generation.get().to_string(),
            );
            attributes.insert(
                EXPIRY_ATTRIBUTE.to_owned(),
                metadata.expires_at_unix_ms.to_string(),
            );
            attributes
        }

        fn parse_metadata(
            attributes: &std::collections::HashMap<String, String>,
            selector: &OwnedSecretSelector,
        ) -> Result<OwnedSecretMetadata, SecretStoreError> {
            for (key, expected) in Self::selector_attributes(selector) {
                if attributes.get(&key) != Some(&expected) {
                    return Err(SecretStoreError::Denied);
                }
            }
            let source_version = attributes
                .get(SOURCE_VERSION_ATTRIBUTE)
                .cloned()
                .ok_or(SecretStoreError::InvalidData)
                .and_then(|value| {
                    SourceVersion::parse(value).map_err(|_| SecretStoreError::InvalidData)
                })?;
            let rotation_generation = attributes
                .get(ROTATION_ATTRIBUTE)
                .and_then(|value| value.parse::<u64>().ok())
                .and_then(|value| Generation::new(value).ok())
                .ok_or(SecretStoreError::InvalidData)?;
            let expires_at_unix_ms = attributes
                .get(EXPIRY_ATTRIBUTE)
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|value| *value > 0)
                .ok_or(SecretStoreError::InvalidData)?;
            Ok(OwnedSecretMetadata {
                source_version,
                rotation_generation,
                expires_at_unix_ms,
            })
        }

        async fn one_item(
            &self,
            selector: &OwnedSecretSelector,
        ) -> Result<oo7::dbus::Item, SecretStoreError> {
            self.authorize_selector(selector)?;
            if self.collection.is_locked().await.map_err(map_oo7_error)? {
                return Err(SecretStoreError::Locked);
            }
            let mut items = self
                .collection
                .search_items(&Self::selector_attributes(selector))
                .await
                .map_err(map_oo7_error)?;
            match items.len() {
                0 => Err(SecretStoreError::NotFound),
                1 => Ok(items.remove(0)),
                _ => Err(SecretStoreError::Duplicate),
            }
        }
    }

    fn map_oo7_error(error: Oo7Error) -> SecretStoreError {
        match error {
            Oo7Error::Service(ServiceError::IsLocked(_)) => SecretStoreError::Locked,
            Oo7Error::Service(ServiceError::NoSuchObject(_))
            | Oo7Error::Deleted
            | Oo7Error::NotFound(_) => SecretStoreError::NotFound,
            Oo7Error::Dismissed => SecretStoreError::Denied,
            _ => SecretStoreError::Unavailable,
        }
    }

    #[async_trait]
    impl SecretStore for Oo7SecretStore {
        async fn state(
            &self,
        ) -> Result<d2b_provider_credential_secret_service::SecretServiceState, SecretStoreError>
        {
            self.collection
                .is_locked()
                .await
                .map(|locked| {
                    if locked {
                        d2b_provider_credential_secret_service::SecretServiceState::Locked
                    } else {
                        d2b_provider_credential_secret_service::SecretServiceState::Unlocked
                    }
                })
                .map_err(map_oo7_error)
        }

        async fn put_owned(
            &self,
            selector: &OwnedSecretSelector,
            metadata: &OwnedSecretMetadata,
            secret: SecretMaterial,
        ) -> Result<(), SecretStoreError> {
            self.authorize_selector(selector)?;
            if self.collection.is_locked().await.map_err(map_oo7_error)? {
                return Err(SecretStoreError::Locked);
            }
            let attributes = Self::all_attributes(selector, metadata);
            self.collection
                .create_item(
                    "d2b credential",
                    &attributes,
                    secret.expose(|bytes| oo7::Secret::blob(bytes)),
                    true,
                    None,
                )
                .await
                .map(|_| ())
                .map_err(map_oo7_error)
        }

        async fn metadata(
            &self,
            selector: &OwnedSecretSelector,
        ) -> Result<OwnedSecretMetadata, SecretStoreError> {
            let item = self.one_item(selector).await?;
            let attributes = item.attributes().await.map_err(map_oo7_error)?;
            Self::parse_metadata(&attributes, selector)
        }

        async fn read_owned(
            &self,
            selector: &OwnedSecretSelector,
        ) -> Result<(OwnedSecretMetadata, SecretMaterial), SecretStoreError> {
            let item = self.one_item(selector).await?;
            let attributes = item.attributes().await.map_err(map_oo7_error)?;
            let metadata = Self::parse_metadata(&attributes, selector)?;
            let secret = item.secret().await.map_err(map_oo7_error)?;
            let material = SecretMaterial::new(secret.as_bytes().to_vec())?;
            Ok((metadata, material))
        }

        async fn delete_owned(
            &self,
            selector: &OwnedSecretSelector,
        ) -> Result<bool, SecretStoreError> {
            self.authorize_selector(selector)?;
            if self.collection.is_locked().await.map_err(map_oo7_error)? {
                return Err(SecretStoreError::Locked);
            }
            let items = self
                .collection
                .search_items(&Self::selector_attributes(selector))
                .await
                .map_err(map_oo7_error)?;
            if items.len() > 1 {
                return Err(SecretStoreError::Duplicate);
            }
            let Some(item) = items.into_iter().next() else {
                return Ok(false);
            };
            let attributes = item.attributes().await.map_err(map_oo7_error)?;
            Self::parse_metadata(&attributes, selector)?;
            item.delete(None).await.map_err(map_oo7_error)?;
            Ok(true)
        }
    }
}

#[cfg(feature = "secret-service")]
pub use oo7_store::Oo7SecretStore;

#[derive(Clone)]
struct PortLeaseRecord {
    request: SecretServiceLeaseRequest,
    metadata: OwnedSecretMetadata,
    expires_at_unix_ms: u64,
    state: SecretServiceLeaseState,
    revoked_at_unix_ms: Option<u64>,
}

pub struct UserdSecretServicePort {
    owner: OwnerBinding,
    store: Arc<dyn SecretStore>,
    entropy: Arc<dyn EntropySource>,
    clock: Arc<dyn UserdClock>,
    metrics: Arc<dyn SecretMetricSink>,
    leases: Mutex<BTreeMap<LeaseId, PortLeaseRecord>>,
}

impl fmt::Debug for UserdSecretServicePort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UserdSecretServicePort")
            .field("owner", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl UserdSecretServicePort {
    pub fn new(
        owner: OwnerBinding,
        store: Arc<dyn SecretStore>,
        metrics: Arc<dyn SecretMetricSink>,
    ) -> Self {
        Self::new_with_runtime(
            owner,
            store,
            Arc::new(OsEntropy),
            Arc::new(SystemUserdClock),
            metrics,
        )
    }

    pub fn new_with_runtime(
        owner: OwnerBinding,
        store: Arc<dyn SecretStore>,
        entropy: Arc<dyn EntropySource>,
        clock: Arc<dyn UserdClock>,
        metrics: Arc<dyn SecretMetricSink>,
    ) -> Self {
        Self {
            owner,
            store,
            entropy,
            clock,
            metrics,
            leases: Mutex::new(BTreeMap::new()),
        }
    }

    fn now(&self) -> u64 {
        self.clock.now_unix_ms().min(MAX_SAFE_JSON_INTEGER)
    }

    fn lock_leases(
        &self,
    ) -> Result<MutexGuard<'_, BTreeMap<LeaseId, PortLeaseRecord>>, SecretServicePortError> {
        self.leases
            .lock()
            .map_err(|_| SecretServicePortError::Unavailable)
    }

    fn validate_placement(
        &self,
        placement: &d2b_contracts::v2_provider::CredentialPlacementBinding,
    ) -> Result<(), SecretServicePortError> {
        if !self.owner.owns_placement(placement) {
            return Err(SecretServicePortError::Denied);
        }
        Ok(())
    }

    fn map_store_error(error: SecretStoreError) -> SecretServicePortError {
        match error {
            SecretStoreError::Locked => SecretServicePortError::Locked,
            SecretStoreError::Denied => SecretServicePortError::Denied,
            SecretStoreError::NotFound => SecretServicePortError::LeaseRevoked,
            SecretStoreError::Duplicate | SecretStoreError::InvalidData => {
                SecretServicePortError::CompletionUnknown
            }
            SecretStoreError::Unavailable => SecretServicePortError::Unavailable,
        }
    }

    fn next_lease_id(
        &self,
        leases: &BTreeMap<LeaseId, PortLeaseRecord>,
    ) -> Result<LeaseId, SecretServicePortError> {
        for _ in 0..4 {
            let mut entropy = [0_u8; ENTROPY_BYTES];
            self.entropy
                .fill(&mut entropy)
                .map_err(|_| SecretServicePortError::Unavailable)?;
            let mut value = String::with_capacity(1 + ENTROPY_BYTES * 2);
            value.push('l');
            append_hex(&mut value, &entropy);
            let id = LeaseId::parse(value).map_err(|_| SecretServicePortError::Unavailable)?;
            if !leases.contains_key(&id) {
                return Ok(id);
            }
        }
        Err(SecretServicePortError::Unavailable)
    }

    fn record(&self, operation: SecretOperation, outcome: ClosedOutcome) {
        self.metrics
            .record(SecretMetricEvent { operation, outcome });
    }

    fn reference_matches(record: &PortLeaseRecord, lease: &SecretServiceLeaseRef) -> bool {
        lease.acquired_by == record.request.operation
            && lease.credential_provider_id == record.request.credential_provider_id
            && lease.credential_provider_generation == record.request.credential_provider_generation
            && lease.consumer_provider_id == record.request.consumer_provider_id
            && lease.consumer_provider_generation == record.request.consumer_provider_generation
            && lease.placement_binding == record.request.placement_binding
            && lease.allowed_operations == record.request.allowed_operations
            && lease.source_version == record.metadata.source_version
            && lease.rotation_generation == record.metadata.rotation_generation
    }
}

#[async_trait]
impl Oo7SecretServicePort for UserdSecretServicePort {
    async fn state(&self) -> Result<SecretServiceState, SecretServicePortError> {
        let result = self.store.state().await.map_err(Self::map_store_error);
        self.record(
            SecretOperation::Status,
            if result.is_ok() {
                ClosedOutcome::Succeeded
            } else {
                ClosedOutcome::Failed
            },
        );
        result
    }

    async fn issue_lease(
        &self,
        request: &SecretServiceLeaseRequest,
    ) -> Result<SecretServiceLeaseGrant, SecretServicePortError> {
        self.validate_placement(&request.placement_binding)?;
        let now = self.now();
        if request.requested_expiry_unix_ms <= now {
            return Err(SecretServicePortError::DeadlineExpired);
        }
        let selector =
            OwnedSecretSelector::new(self.owner.clone(), request.credential_provider_id.clone());
        let metadata = self
            .store
            .metadata(&selector)
            .await
            .map_err(Self::map_store_error)?;
        if metadata.expires_at_unix_ms <= now {
            return Err(SecretServicePortError::LeaseExpired);
        }
        let expires_at_unix_ms = request
            .requested_expiry_unix_ms
            .min(metadata.expires_at_unix_ms);
        let mut leases = self.lock_leases()?;
        leases.retain(|_, lease| {
            lease.state == SecretServiceLeaseState::Active && lease.expires_at_unix_ms > now
        });
        if leases.len() >= MAX_PORT_LEASES {
            return Err(SecretServicePortError::Unavailable);
        }
        let lease_id = self.next_lease_id(&leases)?;
        leases.insert(
            lease_id.clone(),
            PortLeaseRecord {
                request: request.clone(),
                metadata: metadata.clone(),
                expires_at_unix_ms,
                state: SecretServiceLeaseState::Active,
                revoked_at_unix_ms: None,
            },
        );
        self.record(SecretOperation::AcquireLease, ClosedOutcome::Succeeded);
        Ok(SecretServiceLeaseGrant {
            lease_id,
            source_version: metadata.source_version,
            rotation_generation: metadata.rotation_generation,
            expires_at_unix_ms,
        })
    }

    async fn inspect_lease(
        &self,
        lease: &SecretServiceLeaseRef,
    ) -> Result<SecretServiceLeaseInspection, SecretServicePortError> {
        self.validate_placement(&lease.placement_binding)?;
        let selector =
            OwnedSecretSelector::new(self.owner.clone(), lease.credential_provider_id.clone());
        let current_metadata = self
            .store
            .metadata(&selector)
            .await
            .map_err(Self::map_store_error)?;
        let now = self.now();
        let mut leases = self.lock_leases()?;
        let record = leases
            .get_mut(&lease.lease_id)
            .ok_or(SecretServicePortError::LeaseRevoked)?;
        if !Self::reference_matches(record, lease) {
            return Err(SecretServicePortError::Denied);
        }
        if record.state == SecretServiceLeaseState::Active && current_metadata != record.metadata {
            record.state = SecretServiceLeaseState::Revoked;
            record.revoked_at_unix_ms = Some(now);
        }
        if record.state == SecretServiceLeaseState::Active && now >= record.expires_at_unix_ms {
            record.state = SecretServiceLeaseState::Expired;
        }
        let result = SecretServiceLeaseInspection {
            state: record.state,
            source_version: record.metadata.source_version.clone(),
            rotation_generation: record.metadata.rotation_generation,
            expires_at_unix_ms: record.expires_at_unix_ms,
            revoked_at_unix_ms: record.revoked_at_unix_ms,
        };
        self.record(SecretOperation::InspectLease, ClosedOutcome::Succeeded);
        Ok(result)
    }

    async fn refresh_lease(
        &self,
        lease: &SecretServiceLeaseRef,
    ) -> Result<SecretServiceLeaseRenewal, SecretServicePortError> {
        self.validate_placement(&lease.placement_binding)?;
        let selector =
            OwnedSecretSelector::new(self.owner.clone(), lease.credential_provider_id.clone());
        let metadata = self
            .store
            .metadata(&selector)
            .await
            .map_err(Self::map_store_error)?;
        let now = self.now();
        let mut leases = self.lock_leases()?;
        let record = leases
            .get_mut(&lease.lease_id)
            .ok_or(SecretServicePortError::LeaseRevoked)?;
        if !Self::reference_matches(record, lease) {
            return Err(SecretServicePortError::Denied);
        }
        if record.state != SecretServiceLeaseState::Active || record.expires_at_unix_ms <= now {
            record.state = SecretServiceLeaseState::Expired;
            return Err(SecretServicePortError::LeaseExpired);
        }
        let expires_at_unix_ms = lease
            .requested_expiry_unix_ms
            .min(metadata.expires_at_unix_ms);
        if expires_at_unix_ms <= now {
            return Err(SecretServicePortError::LeaseExpired);
        }
        record.metadata = metadata.clone();
        record.expires_at_unix_ms = expires_at_unix_ms;
        self.record(SecretOperation::RefreshLease, ClosedOutcome::Succeeded);
        Ok(SecretServiceLeaseRenewal {
            source_version: metadata.source_version,
            rotation_generation: metadata.rotation_generation,
            expires_at_unix_ms,
        })
    }

    async fn revoke_lease(
        &self,
        lease: &SecretServiceLeaseRef,
    ) -> Result<SecretServiceLeaseRevocation, SecretServicePortError> {
        self.validate_placement(&lease.placement_binding)?;
        let selector =
            OwnedSecretSelector::new(self.owner.clone(), lease.credential_provider_id.clone());
        let current_metadata = self
            .store
            .metadata(&selector)
            .await
            .map_err(Self::map_store_error)?;
        let now = self.now();
        let mut leases = self.lock_leases()?;
        let record = leases
            .get_mut(&lease.lease_id)
            .ok_or(SecretServicePortError::LeaseRevoked)?;
        if !Self::reference_matches(record, lease) {
            return Err(SecretServicePortError::Denied);
        }
        if current_metadata != record.metadata {
            return Err(SecretServicePortError::LeaseRevoked);
        }
        if let Some(revoked_at_unix_ms) = record.revoked_at_unix_ms {
            self.record(SecretOperation::RevokeLease, ClosedOutcome::AlreadyApplied);
            return Ok(SecretServiceLeaseRevocation::AlreadyRevoked { revoked_at_unix_ms });
        }
        record.state = SecretServiceLeaseState::Revoked;
        record.revoked_at_unix_ms = Some(now);
        self.record(SecretOperation::RevokeLease, ClosedOutcome::Succeeded);
        Ok(SecretServiceLeaseRevocation::Revoked {
            revoked_at_unix_ms: now,
        })
    }
}

fn append_hex(destination: &mut String, value: &[u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in value {
        destination.push(char::from(HEX[usize::from(byte >> 4)]));
        destination.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use d2b_contracts::v2_identity::{
        RealmId, RealmPath, RoleId, RoleKind, WorkloadId, WorkloadName,
    };

    #[derive(Default)]
    pub struct FakeClock(pub Mutex<u64>);

    impl UserdClock for FakeClock {
        fn now_unix_ms(&self) -> u64 {
            *self.0.lock().expect("clock")
        }
    }

    #[derive(Default)]
    pub struct FixedEntropy(pub Mutex<u8>);

    impl EntropySource for FixedEntropy {
        fn fill(&self, destination: &mut [u8]) -> Result<(), UserSecretEntropyError> {
            let mut byte = self.0.lock().expect("entropy");
            destination.fill(*byte);
            *byte = byte.wrapping_add(1);
            Ok(())
        }
    }

    #[derive(Default)]
    pub struct MemoryStore {
        pub values: Mutex<BTreeMap<String, (OwnedSecretMetadata, Vec<u8>)>>,
        pub locked: Mutex<bool>,
    }

    #[async_trait]
    impl SecretStore for MemoryStore {
        async fn state(&self) -> Result<SecretServiceState, SecretStoreError> {
            Ok(if *self.locked.lock().expect("locked") {
                SecretServiceState::Locked
            } else {
                SecretServiceState::Unlocked
            })
        }

        async fn put_owned(
            &self,
            selector: &OwnedSecretSelector,
            metadata: &OwnedSecretMetadata,
            secret: SecretMaterial,
        ) -> Result<(), SecretStoreError> {
            let bytes = secret.expose(<[u8]>::to_vec);
            self.values.lock().expect("values").insert(
                selector.provider_id().as_str().to_owned(),
                (metadata.clone(), bytes),
            );
            Ok(())
        }

        async fn metadata(
            &self,
            selector: &OwnedSecretSelector,
        ) -> Result<OwnedSecretMetadata, SecretStoreError> {
            if *self.locked.lock().expect("locked") {
                return Err(SecretStoreError::Locked);
            }
            self.values
                .lock()
                .expect("values")
                .get(selector.provider_id().as_str())
                .map(|(metadata, _)| metadata.clone())
                .ok_or(SecretStoreError::NotFound)
        }

        async fn read_owned(
            &self,
            selector: &OwnedSecretSelector,
        ) -> Result<(OwnedSecretMetadata, SecretMaterial), SecretStoreError> {
            let (metadata, secret) = self
                .values
                .lock()
                .expect("values")
                .get(selector.provider_id().as_str())
                .cloned()
                .ok_or(SecretStoreError::NotFound)?;
            Ok((metadata, SecretMaterial::new(secret)?))
        }

        async fn delete_owned(
            &self,
            selector: &OwnedSecretSelector,
        ) -> Result<bool, SecretStoreError> {
            if *self.locked.lock().expect("locked") {
                return Err(SecretStoreError::Locked);
            }
            Ok(self
                .values
                .lock()
                .expect("values")
                .remove(selector.provider_id().as_str())
                .is_some())
        }
    }

    pub fn owner() -> OwnerBinding {
        let realm = RealmId::derive(&RealmPath::root());
        let workload = WorkloadId::derive(&realm, &WorkloadName::parse("userd").expect("workload"));
        OwnerBinding::new(
            1000,
            realm.clone(),
            RoleId::derive(&realm, &workload, RoleKind::WaylandProxy),
            Generation::new(3).expect("generation"),
        )
    }

    #[test]
    fn secret_material_debug_never_exposes_bytes() {
        let material = SecretMaterial::new(b"highly-sensitive".to_vec()).expect("material");
        let debug = format!("{material:?}");
        assert_eq!(debug, "SecretMaterial(<redacted>)");
        assert!(!debug.contains("sensitive"));
    }
}
