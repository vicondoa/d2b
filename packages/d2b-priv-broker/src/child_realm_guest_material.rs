use std::{collections::BTreeMap, fmt, path::PathBuf, sync::Arc};

use async_trait::async_trait;
use d2b_contracts::{
    v2_component_session::{BootstrapPskBinding, GuestBootstrapPsk, OperationId},
    v2_guest_configured_launches::GuestConfiguredLaunchesV1,
    v2_services::{
        broker::{
            AllocateRequest, AllocateResponse, SpawnRealmChildrenRequest,
            SpawnRealmChildrenResponse,
        },
        common::{ServiceRequest, ServiceResponse},
    },
};
use d2b_host::realm_broker_bootstrap::{
    RealmBrokerChildAuthority, RealmBrokerGuestRuntimeBootstrap,
};
use d2b_session::OwnedAttachment;
use zeroize::Zeroizing;

use crate::{
    guest_material_audit::FileGuestMaterialAuditSink,
    guest_material_authority::{FileBootstrapReplayLedger, RealmGuestSessionAuthorityConnector},
    guest_session_material::{
        FilesystemGuestMaterialStore, GuestBootstrapAuthority, GuestMaterialBundle,
        GuestMaterialBundlePort, GuestMaterialClock, GuestMaterialError, GuestMaterialStore,
        GuestMaterialTarget, GuestSessionAuthority, GuestSessionAuthorityPort,
        GuestSessionMaterialBroker, RealmBoundGuestMaterialDispatch, SystemGuestMaterialClock,
        guest_material_descriptor_policy_resolver,
    },
    runtime::RunError,
    service_v2::{
        BrokerCallContext, BrokerMethod, BrokerOperationHandler, BrokerReply,
        BrokerRuntimeDispatch, BrokerServiceFailure,
    },
};

pub(crate) struct ChildRealmBrokerHandler {
    dispatch: Arc<RealmBoundGuestMaterialDispatch<ClosedChildFallback>>,
}

impl fmt::Debug for ChildRealmBrokerHandler {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ChildRealmBrokerHandler(REDACTED)")
    }
}

#[async_trait]
impl BrokerOperationHandler for ChildRealmBrokerHandler {
    async fn handle(
        &self,
        method: BrokerMethod,
        request: ServiceRequest,
        attachments: Vec<OwnedAttachment>,
        context: &BrokerCallContext,
    ) -> Result<BrokerReply<ServiceResponse>, BrokerServiceFailure> {
        let result = self
            .dispatch
            .dispatch(method, request, attachments, context)
            .await;
        if let Err(error) = result.as_ref() {
            tracing::warn!(error = %error, "child realm guest-material operation failed closed");
        }
        result
    }

    async fn allocate(
        &self,
        _: AllocateRequest,
        _: &BrokerCallContext,
    ) -> Result<BrokerReply<AllocateResponse>, BrokerServiceFailure> {
        Err(BrokerServiceFailure::PermissionDenied)
    }

    async fn spawn(
        &self,
        _: SpawnRealmChildrenRequest,
        _: Vec<OwnedAttachment>,
        _: &BrokerCallContext,
    ) -> Result<BrokerReply<SpawnRealmChildrenResponse>, BrokerServiceFailure> {
        Err(BrokerServiceFailure::PermissionDenied)
    }

    fn attachment_policy_resolver(&self) -> Option<d2b_session_unix::DescriptorPolicyResolver> {
        Some(guest_material_descriptor_policy_resolver())
    }
}

#[derive(Debug)]
struct ClosedChildFallback;

#[async_trait]
impl BrokerRuntimeDispatch for ClosedChildFallback {
    async fn dispatch(
        &self,
        _: BrokerMethod,
        _: ServiceRequest,
        _: Vec<OwnedAttachment>,
        _: &BrokerCallContext,
    ) -> Result<BrokerReply<ServiceResponse>, BrokerServiceFailure> {
        Err(BrokerServiceFailure::PermissionDenied)
    }
}

struct ChildBundleEntry {
    session_target: GuestMaterialTarget,
    configured_target: GuestMaterialTarget,
    configured_launches: Zeroizing<Vec<u8>>,
    configured_digest: [u8; 32],
}

struct ChildGuestMaterialBundlePort {
    realm_id: String,
    entries: BTreeMap<String, ChildBundleEntry>,
}

impl fmt::Debug for ChildGuestMaterialBundlePort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ChildGuestMaterialBundlePort(REDACTED)")
    }
}

impl GuestMaterialBundlePort for ChildGuestMaterialBundlePort {
    fn resolve(
        &self,
        storage_ref: &str,
        realm_id: &str,
        workload_id: &str,
    ) -> Result<GuestMaterialBundle, GuestMaterialError> {
        if realm_id != self.realm_id {
            return Err(GuestMaterialError::AuthorityMismatch);
        }
        let entry = self
            .entries
            .get(workload_id)
            .ok_or(GuestMaterialError::StorageRefUnknown)?;
        if storage_ref != entry.session_target.storage_ref {
            return Err(GuestMaterialError::StorageRefUnknown);
        }
        let configured_launches = GuestConfiguredLaunchesV1::decode(&entry.configured_launches)
            .and_then(|catalog| catalog.encode())
            .map_err(|_| GuestMaterialError::InventoryInvalid)?;
        if configured_launches.sha256() != entry.configured_digest {
            return Err(GuestMaterialError::DigestMismatch);
        }
        Ok(GuestMaterialBundle {
            session_target: entry.session_target.clone(),
            configured_launch_target: entry.configured_target.clone(),
            configured_launches,
            configured_launch_digest: entry.configured_digest,
        })
    }
}

pub(crate) fn build_guest_material_handler(
    authority: &RealmBrokerChildAuthority,
    mut runtime: RealmBrokerGuestRuntimeBootstrap,
) -> Result<ChildRealmBrokerHandler, RunError> {
    let ledger = Arc::new(
        FileBootstrapReplayLedger::open(PathBuf::from(&runtime.replay_ledger_path), 0, 0)
            .map_err(material_protocol)?,
    );
    let audit = Arc::new(
        FileGuestMaterialAuditSink::open(PathBuf::from(&runtime.audit_log_path), 0, 0)
            .map_err(material_protocol)?,
    );
    let store = Arc::new(FilesystemGuestMaterialStore::realm_child());
    let connector = Arc::new(RealmGuestSessionAuthorityConnector::new(
        authority.realm_id.clone(),
        Arc::clone(&ledger) as Arc<dyn crate::guest_material_authority::BootstrapReplayLedger>,
    ));
    let mut entries = BTreeMap::new();
    for mut workload in runtime.workloads.drain(..) {
        let catalog = GuestConfiguredLaunchesV1::decode(&workload.configured_launches)
            .map_err(|_| protocol("child realm broker configured launches invalid"))?;
        if catalog.realm_id().as_str() != authority.realm_id
            || catalog.workload_id().as_str() != workload.workload_id
        {
            return Err(protocol(
                "child realm broker configured launches identity mismatch",
            ));
        }
        let configured_launches = catalog
            .encode()
            .map_err(|_| protocol("child realm broker configured launches invalid"))?;
        if configured_launches.sha256() != workload.configured_launch_digest {
            return Err(protocol(
                "child realm broker configured launches digest mismatch",
            ));
        }
        let session_target = GuestMaterialTarget {
            storage_ref: workload.session_storage_ref.clone(),
            path: PathBuf::from(&workload.session_path),
            owner_uid: workload.owner_uid,
            owner_gid: workload.owner_gid,
            mode: workload.mode,
        };
        let configured_target = GuestMaterialTarget {
            storage_ref: workload.configured_storage_ref.clone(),
            path: PathBuf::from(&workload.configured_path),
            owner_uid: workload.owner_uid,
            owner_gid: workload.owner_gid,
            mode: workload.mode,
        };
        store
            .recover_enrollment_pair(
                &session_target,
                &configured_target,
                workload.configured_launch_digest,
                ledger.as_ref(),
                audit.as_ref(),
            )
            .map_err(material_protocol)?;
        let operation_id = OperationId::new(workload.bootstrap_operation_id.to_vec())
            .map_err(|_| protocol("child realm broker bootstrap operation invalid"))?;
        let mut psk = std::mem::take(&mut workload.bootstrap_psk);
        let bootstrap_psk = GuestBootstrapPsk::copy_from_and_zeroize(&mut psk)
            .map_err(|_| protocol("child realm broker bootstrap psk invalid"))?;
        connector
            .install(GuestSessionAuthority {
                realm_id: authority.realm_id.clone(),
                workload_id: workload.workload_id.clone(),
                session_generation: authority.session_generation,
                parent_static_public_key: workload.parent_static_public_key,
                channel_binding: workload.channel_binding,
                guest_identity_digest: [0; 32],
                guest_static_public_key: [0; 32],
                bootstrap: Some(GuestBootstrapAuthority {
                    binding: BootstrapPskBinding {
                        operation_id,
                        replay_nonce: workload.replay_nonce,
                        expires_at_unix_ms: workload.expires_at_unix_ms,
                    },
                    issued_at_unix_ms: workload.issued_at_unix_ms,
                    psk: bootstrap_psk,
                }),
            })
            .map_err(material_protocol)?;
        if entries
            .insert(
                workload.workload_id.clone(),
                ChildBundleEntry {
                    session_target,
                    configured_target,
                    configured_launches: Zeroizing::new(configured_launches.as_slice().to_vec()),
                    configured_digest: workload.configured_launch_digest,
                },
            )
            .is_some()
        {
            return Err(protocol("child realm broker duplicate workload"));
        }
    }
    let bundle = Arc::new(ChildGuestMaterialBundlePort {
        realm_id: authority.realm_id.clone(),
        entries,
    });
    let material = Arc::new(GuestSessionMaterialBroker::new(
        Arc::clone(&connector) as Arc<dyn GuestSessionAuthorityPort>,
        bundle as Arc<dyn GuestMaterialBundlePort>,
        store as Arc<dyn GuestMaterialStore>,
        audit as Arc<dyn crate::guest_session_material::GuestMaterialAuditSink>,
        Arc::new(SystemGuestMaterialClock) as Arc<dyn GuestMaterialClock>,
    ));
    let dispatch = Arc::new(
        RealmBoundGuestMaterialDispatch::new(
            authority.realm_id.clone(),
            connector,
            material,
            ClosedChildFallback,
        )
        .map_err(material_protocol)?,
    );
    Ok(ChildRealmBrokerHandler { dispatch })
}

fn material_protocol(_: GuestMaterialError) -> RunError {
    protocol("child realm broker guest material unavailable")
}

fn protocol(message: &'static str) -> RunError {
    RunError::Protocol(message.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::{
        v2_guest_configured_launches::GuestConfiguredLaunchEntryV1,
        v2_identity::{RealmId, WorkloadId},
    };
    use d2b_core::configured_argv::ConfiguredArgv;
    use d2b_realm_core::ProtocolToken;
    use sha2::{Digest as _, Sha256};

    const REALM: &str = "aaaaaaaaaaaaaaaaaaaa";
    const WORKLOAD: &str = "bbbbbbbbbbbbbbbbbbba";
    const STORAGE: &str = "guest-session-bbbbbbbbbbbbbbbbbbba";

    fn encoded_catalog() -> Vec<u8> {
        let entry = GuestConfiguredLaunchEntryV1::new(
            ProtocolToken::parse("editor").unwrap(),
            ConfiguredArgv::new(vec!["editor".to_owned()]).unwrap(),
            false,
        )
        .unwrap();
        GuestConfiguredLaunchesV1::new(
            RealmId::parse(REALM).unwrap(),
            WorkloadId::parse(WORKLOAD).unwrap(),
            Sha256::digest(b"inventory").into(),
            vec![entry],
        )
        .unwrap()
        .encode()
        .unwrap()
        .as_slice()
        .to_vec()
    }

    fn bundle(bytes: Vec<u8>, digest: [u8; 32]) -> ChildGuestMaterialBundlePort {
        ChildGuestMaterialBundlePort {
            realm_id: REALM.to_owned(),
            entries: BTreeMap::from([(
                WORKLOAD.to_owned(),
                ChildBundleEntry {
                    session_target: GuestMaterialTarget {
                        storage_ref: STORAGE.to_owned(),
                        path: PathBuf::from("/run/d2b/session"),
                        owner_uid: 1000,
                        owner_gid: 1000,
                        mode: 0o400,
                    },
                    configured_target: GuestMaterialTarget {
                        storage_ref: "configured-launches-bbbbbbbbbbbbbbbbbbba".to_owned(),
                        path: PathBuf::from("/run/d2b/configured"),
                        owner_uid: 1000,
                        owner_gid: 1000,
                        mode: 0o400,
                    },
                    configured_launches: Zeroizing::new(bytes),
                    configured_digest: digest,
                },
            )]),
        }
    }

    #[test]
    fn child_bundle_resolves_only_exact_authority_and_storage() {
        let encoded = encoded_catalog();
        let port = bundle(encoded.clone(), Sha256::digest(&encoded).into());

        assert!(port.resolve(STORAGE, REALM, WORKLOAD).is_ok());
        assert!(matches!(
            port.resolve(STORAGE, "cccccccccccccccccccc", WORKLOAD),
            Err(GuestMaterialError::AuthorityMismatch)
        ));
        assert!(matches!(
            port.resolve("other-storage", REALM, WORKLOAD),
            Err(GuestMaterialError::StorageRefUnknown)
        ));
        assert!(matches!(
            port.resolve(STORAGE, REALM, "ccccccccccccccccccca"),
            Err(GuestMaterialError::StorageRefUnknown)
        ));
    }

    #[test]
    fn child_bundle_rejects_invalid_inventory_and_digest() {
        assert!(matches!(
            bundle(vec![0xff], [1; 32]).resolve(STORAGE, REALM, WORKLOAD),
            Err(GuestMaterialError::InventoryInvalid)
        ));
        let encoded = encoded_catalog();
        assert!(matches!(
            bundle(encoded, [2; 32]).resolve(STORAGE, REALM, WORKLOAD),
            Err(GuestMaterialError::DigestMismatch)
        ));
    }
}
