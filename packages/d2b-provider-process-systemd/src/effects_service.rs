//! The system-systemd provider's declared effects service (U15).
//!
//! The five committed process-systemd operations are served through the
//! family's provider-operation table
//! ([`crate::operations::process_systemd_family_operations`]); this module
//! hosts the family's declared zone-plane service beside them, exactly as
//! the Process and Network families host their effects services. The
//! service carries the five operation surfaces as operation-serving methods
//! (`ServiceMethod::serving`, KD6: an operation resolves to the declaring
//! service exactly when a declared method names it), so a forwarded
//! `StartSystemdUnit` invocation reaches the same crate-owned handler table
//! the daemon's provider-operation tables resolve, over the same kernel
//! seam. One zone-plane method, `inspect-process-systemd`, answers the
//! family's committed operation inventory - hermetic, served from the
//! crate's own handler table, reaching no daemon state.

use std::sync::Arc;

use d2b_contracts_resource::v3::{CanonicalJsonObject, ResourceRef, ZoneId, canonical_json_bytes};
use d2b_provider_toolkit::{
    EffectResponse, EffectService, EffectServiceError, EffectServiceFactory, ServiceInvocation,
};
use d2b_resource_types::{MethodFdContract, OperationCtx, ServiceDecl, ServiceMethod, ValidatedPayload};

use crate::operations::{
    CHECK_SYSTEMD_USER_MANAGER, OBSERVE_SYSTEMD_UNIT, OPEN_SYSTEMD_UNIT_PIDFD,
    START_SYSTEMD_UNIT, STOP_SYSTEMD_UNIT, process_systemd_family_operations,
};

/// The system-systemd provider's declared effects service.
/// The service carries the family's five committed operation surfaces (the
/// operation-serving methods) beside the zone-plane inventory method. A
/// forwarded call naming one of the five committed rows resolves here through
/// the method's `operation` facet (KD6), and the hosted actor dispatches
/// to the crate's committed handler table with the invocation's kernel seam -
/// the same handler table the provider-operation path serves, over the same
/// authority path (U15). The service is hermetic and carries no declared
/// facet; the family's privileged operations reach daemon-structural state
/// through the per-zone kernel seam of the forwarded invocation, never
/// through a daemon handle.
pub const PROCESS_SYSTEMD_EFFECTS_SERVICE: ServiceDecl = ServiceDecl {
    id: "process-systemd.d2bus.org/effects",
    methods:&[
        // StartSystemdUnit and OpenSystemdUnitPidfd return the exact-main
        // pidfd the nested open-pidfd kernel minted, so their response leg
        // declares the one-descriptor carriage (anon-inode kind, the
        // pidfd's fstat shape); the hosting side enforces the declared
        // response leg before replying (U15).
        ServiceMethod::serving_with(
            START_SYSTEMD_UNIT,
            "start-systemd-unit",
            None,
            MethodFdContract::NONE,
            MethodFdContract { max_fds: 1, fd_kind: Some("any") },
            &[],
            &[],
            None,
        ),
        ServiceMethod::serving(CHECK_SYSTEMD_USER_MANAGER, "check-systemd-user-manager"),
        ServiceMethod::serving(OBSERVE_SYSTEMD_UNIT, "observe-systemd-unit"),
        ServiceMethod::serving_with(
            OPEN_SYSTEMD_UNIT_PIDFD,
            "open-systemd-unit-pidfd",
            None,
            MethodFdContract::NONE,
            MethodFdContract { max_fds: 1, fd_kind: Some("any") },
            &[],
            &[],
            None,
        ),
        ServiceMethod::serving(STOP_SYSTEMD_UNIT, "stop-systemd-unit"),
        ServiceMethod::zone_plane("inspect-process-systemd"),
    ],
    attach_kinds:&[],
    streams:&[],
    endpoint_policy:None,
};

/// The family's declared effect services, registered for the daemon's
/// plane:the registration authority resolves this list to serve the
/// service the registration table carries.
pub const PROCESS_SYSTEMD_SERVICES: &[ServiceDecl] = &[PROCESS_SYSTEMD_EFFECTS_SERVICE];

/// The provider-owned system-systemd effects service (U15).
///
/// One value per zone serves the declared methods; the factory constructs it
/// from crate-owned constants alone, so a respawn rebuilds the same surface
/// from its durable row (KTD5).
#[derive(Default)]
pub struct SystemdEffectsService;

impl SystemdEffectsService {
    /// Build the zone's effects service value.
    pub fn new() -> Self {
        Self
    }
}

/// The one `inspect-process-systemd` response payload:the family's
/// committed operation inventory, built through the canonical JSON object
/// path.
fn inspect_response() -> Result<EffectResponse, EffectServiceError> {
    let operations = [
        START_SYSTEMD_UNIT,
        CHECK_SYSTEMD_USER_MANAGER,
        OBSERVE_SYSTEMD_UNIT,
        OPEN_SYSTEMD_UNIT_PIDFD,
        STOP_SYSTEMD_UNIT,
    ];
    let bytes = canonical_json_bytes(&serde_json::json!({
        "family":"process-systemd",
        "declaringProvider":"d2b-provider-process-systemd",
        "operations":operations,
        "declaredOperations":process_systemd_family_operations().len(),
        "service":PROCESS_SYSTEMD_EFFECTS_SERVICE.id,
    }))
    .map_err(|error| EffectServiceError::Declined {
        service:PROCESS_SYSTEMD_EFFECTS_SERVICE.id.to_owned(),
        reason:format!("inspect-process-systemd: response is not canonical JSON: {error}"),
    })?;
    let payload = CanonicalJsonObject::parse(&bytes).map_err(|error| {
        EffectServiceError::Declined {
            service:PROCESS_SYSTEMD_EFFECTS_SERVICE.id.to_owned(),
            reason:format!("inspect-process-systemd: response is not a canonical object: {error}"),
        }
    })?;
    Ok(EffectResponse::new(payload))
}

/// Run one committed family operation under the forwarded invocation's
/// capability object.
///
/// The operation resolves to this service through its declared method's
/// `operation` facet (KD6); the declaration's method name is the
/// operation's kebab `Operation/<method>` reference label, so the handler
/// table the crate publishes is looked up by that label against the table's
/// canonical references. The handler runs under an [`OperationCtx`]
/// assembled from the invocation:the zone, the caller `Provider/process-
/// systemd` the family's registered identity, the invocation id, the
/// request-leg descriptors, the invocation's kernel seam, and an empty
/// evidence chain (a root forwarded call carries none; the chain the broker
/// minted for the forwarded root stays broker-side, exactly as a
/// provider-operation-table dispatch's root call does.
async fn run_operation(
    invocation: &ServiceInvocation<'_>,
) -> Result<EffectResponse, EffectServiceError> {
    let Some(def) = process_systemd_family_operations()
        .iter()
        .find(|def| def.operation_ref.name().as_str() == invocation.method)
    else {
        return Err(EffectServiceError::Declined {
            service:PROCESS_SYSTEMD_EFFECTS_SERVICE.id.to_owned(),
            reason:format!("unserved-method:{}", invocation.method),
        });
    };
let zone = ZoneId::parse(invocation.zone).map_err(|_| {
        EffectServiceError::Declined {
            service:PROCESS_SYSTEMD_EFFECTS_SERVICE.id.to_owned(),
            reason:"invalid-zone".to_owned(),
        }
    })?;
    let caller = ResourceRef::parse("Provider/process-systemd").map_err(|_| {
        EffectServiceError::Declined {
            service:PROCESS_SYSTEMD_EFFECTS_SERVICE.id.to_owned(),
            reason:"provider-ref-invalid".to_owned(),
        }
    })?;
    let ctx = OperationCtx {
        zone:&zone,
        caller:&caller,
        operation:&def.operation_ref,
        invocation_id:invocation.invocation_id,
        fds:invocation.request_fds,
        chain_identities:invocation.chain_identities,
        kernel:invocation.kernel,
    };
    let result = def
        .handler
        .execute(ctx, ValidatedPayload::new(invocation.payload.clone()))
        .await
        .map_err(|failure| EffectServiceError::Declined {
            service:PROCESS_SYSTEMD_EFFECTS_SERVICE.id.to_owned(),
            reason:failure.code().to_owned(),
        })?;
    let (payload, fds) = result.into_parts();
    Ok(EffectResponse { payload, fds })
}

#[async_trait::async_trait]
impl EffectService for SystemdEffectsService {
    async fn handle(
        &self,
        invocation:ServiceInvocation<'_>,
    ) -> Result<EffectResponse, EffectServiceError> {
        if invocation.method == "inspect-process-systemd" {
            return inspect_response();
        }
        run_operation(&invocation).await
    }
}

/// The composition-root factory that hosts the system-systemd effects
/// service in one zone (R5): no facet set, so one value serves every zone.
#[derive(Default)]
pub struct SystemdEffectsServiceFactory;

impl SystemdEffectsServiceFactory {
    /// Build the zone-agnostic factory.
    pub fn new() -> Self {
        Self
    }
}

impl EffectServiceFactory for SystemdEffectsServiceFactory {
    fn build(&self) -> Arc<dyn EffectService> {
        Arc::new(SystemdEffectsService::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::CanonicalJsonValue;
    use d2b_resource_runtime::context::ServiceResourceContext;
    use d2b_resource_types::MethodFdContract;

    #[tokio::test]
    async fn the_declared_service_answers_its_operation_inventory() {
        let service = SystemdEffectsService::new();
        let payload = CanonicalJsonObject::parse(
            &canonical_json_bytes(&serde_json::json!({})).expect("payload bytes"),
        )
        .expect("payload object");
        let mut resources = ServiceResourceContext::fail_closed();
        let response = service
            .handle(ServiceInvocation {
                zone:"zone-a",
                method:"inspect-process-systemd",
                invocation_id:"invocation-test",
                payload:&payload,
                resources:&mut resources,
                state_cells:&[],
                kernel:None,
                request_fds:&[],
                response_fds:MethodFdContract::NONE,
                payload_schema:None,
                chain_identities:&[],
            })
            .await
            .expect("the inspection answers");
        let operations = response
            .payload
            .get("operations")
            .and_then(|value| match value {
                CanonicalJsonValue::Array(items) => Some(items),
                _ => None,
            })
            .expect("operations array");
        assert_eq!(operations.len(),5);
        assert_eq!(
            response
                .payload
                .get("family")
                .and_then(|value| match value {
                    CanonicalJsonValue::String(text2) => Some(text2.as_str()),
                    _ => None,
                }),
            Some("process-systemd")
        );
    }

    /// The service dispatches every declared operation surface to the crate's
    /// committed handler table:an operation method an unknown or absent
    /// table entry refuses by method name rather than half-serving. The
    /// table lookup is the dispatch decision, so the five committed rows
    /// each resolve to a handler entry here (KD6/U15).
    #[tokio::test]
    async fn the_operation_methods_resolve_to_the_committed_handler_table() {
        let service = SystemdEffectsService::new();
        for method in PROCESS_SYSTEMD_EFFECTS_SERVICE.methods {
            if method.name == "inspect-process-systemd" {
                continue;
            }
            let payload = CanonicalJsonObject::parse(
                &canonical_json_bytes(&serde_json::json!({})).expect("payload bytes"),
            )
            .expect("payload object");
            let mut resources = ServiceResourceContext::fail_closed();
            // No kernel seam is wired in this hermetic test; the handler que
            // validates the typed request before its kernel read, so an
            // empty payload refuses with the invalid-request code rather
            // than reaching the kernel-seam check. Either code is the
            // committed handler's own closed refusal, never a synthesized
            // half-serving result.


            let error = service
                .handle(ServiceInvocation {
                    zone:"zone-a",
                    method:method.name,
                    invocation_id:"invocation-test",
                    payload:&payload,
                    resources:&mut resources,
                    state_cells:&[],
                    kernel:None,
                    request_fds:&[],
                    response_fds:method.response_fds,
                    payload_schema:method.payload_schema,
                    chain_identities:&[],
                })
                .await
                .expect_err("the operation method refuses without a kernel seam");
            assert!(
            matches!(
                error,
                EffectServiceError::Declined { ref reason, .. }
                    if reason == "kernel-seam-unwired" || reason == "unit-invalid-request"
            ),
            "the method must reach the committed handler's closed refusal, never a synthesized result: {error:?}"
        );
        }
    }

    }