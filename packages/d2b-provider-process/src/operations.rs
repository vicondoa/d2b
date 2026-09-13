//! The family's declared broker operations and their handlers.
//!
//! The Process family's first operation is a read-only declaration
//! inspection: for one member type it answers the serving facts the family
//! declares - the verbs, execution domains, reads, and operations its
//! descriptor carries, plus the Zone and invocation identifier the envelope
//! handed the handler. It touches no effect port and changes no host state,
//! so the broker-forwarded path has an operation it can be proven end to end
//! with before any host-touching operation is declared.
//!
//! The handler table is the declaration itself: the descriptor this crate
//! publishes carries [`process_family_operations`], and the daemon's registry
//! serves the handler from there. There is no second registration step.

use std::sync::LazyLock;

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    CanonicalJsonObject, CanonicalJsonValue, ResourceRef, canonical_json_bytes,
};
use d2b_resource_types::{
    OperationCtx, OperationDef, OperationFailure, OperationHandler, OperationResult,
    ValidatedPayload, WellKnownType,
};

use crate::driver::{PROCESS_FAMILY_EXECUTION_DOMAINS, PROCESS_FAMILY_READS, PROCESS_FAMILY_VERBS};

/// The operation the family declares first.
///
/// The name is the committed row's name and the operation reference's own
/// name: a `ResourceRef` name is a lowercase label, and the broker names a
/// committed operation by exactly the string its declaring crate declares.
/// There is no second spelling to translate between.
pub const INSPECT_PROCESS_FAMILY: &str = "inspect-process-family";

/// The refusal of a payload that names no member type of this family.
pub const INVALID_PROCESS_TYPE: &str = "invalid-process-type";

/// The resource types the family's descriptors cover.
const MEMBER_TYPES: [WellKnownType; 2] = [WellKnownType::PROCESS, WellKnownType::EPHEMERAL_PROCESS];

/// The family's declared operations, assembled once.
///
/// The operation reference is an owned validated value, so the table is built
/// on first use rather than declared `const`; every descriptor the family
/// publishes shares the one table.
pub fn process_family_operations() -> &'static [OperationDef] {
    &PROCESS_FAMILY_OPERATIONS[..]
}

static PROCESS_FAMILY_OPERATIONS: LazyLock<[OperationDef; 1]> = LazyLock::new(|| {
    [OperationDef {
        operation_ref: ResourceRef::parse(&format!("Operation/{INSPECT_PROCESS_FAMILY}"))
            .expect("the family's operation reference is canonical"),
        handler: &INSPECT_PROCESS_FAMILY_HANDLER,
    }]
});

static INSPECT_PROCESS_FAMILY_HANDLER: InspectProcessFamilyHandler = InspectProcessFamilyHandler;

/// The handler of [`INSPECT_PROCESS_FAMILY`].
struct InspectProcessFamilyHandler;

#[async_trait]
impl OperationHandler for InspectProcessFamilyHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let requested = match payload.object().get("resourceType") {
            Some(CanonicalJsonValue::String(name)) => name.as_str(),
            _ => return Err(OperationFailure::new(INVALID_PROCESS_TYPE)),
        };
        let resource_type = MEMBER_TYPES
            .iter()
            .find(|member| member.to_resource_type_name().as_str() == requested)
            .ok_or_else(|| OperationFailure::new(INVALID_PROCESS_TYPE))?;
        let member_types: Vec<String> = MEMBER_TYPES
            .iter()
            .map(|member| member.to_resource_type_name().as_str().to_owned())
            .collect();
        let reads: Vec<String> = PROCESS_FAMILY_READS
            .iter()
            .map(|read| read.to_resource_type_name().as_str().to_owned())
            .collect();
        let operations = [INSPECT_PROCESS_FAMILY];
        let bytes = canonical_json_bytes(&serde_json::json!({
            "family": "process",
            "resourceType": resource_type.to_resource_type_name().as_str(),
            "memberTypes": member_types,
            "verbs": PROCESS_FAMILY_VERBS,
            "execution": PROCESS_FAMILY_EXECUTION_DOMAINS,
            "reads": reads,
            "operations": operations,
            "zone": ctx.zone.as_str(),
            "invocation": ctx.invocation_id,
        }))
        .expect("the inspection result is canonical JSON");
        let result =
            CanonicalJsonObject::parse(&bytes).expect("the inspection result is a JSON object");
        Ok(OperationResult::new(result))
    }
}
