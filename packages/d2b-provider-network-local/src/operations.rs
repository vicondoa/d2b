//! The family's declared broker operations and their handlers.
//!
//! The Network-local family declares the thirteen network-fds operations
//! (U12): the broker-generic kernels serve each operation's privileged,
//! resource-agnostic core in-broker as a committed row, while the family
//! operation itself stays forwarded to the declaring process. Each handler
//! here validates the typed request the envelope forwarded (the same wire
//! shape the retired typed broker arms consumed), resolves the trusted
//! inputs it needs from the Zone's bundle through the U12 family seam, and
//! invokes the matching kernel as a nested envelope call: the evidence
//! chain the forwarded invocation runs on (root invocation id plus ordered
//! identities) is re-presented with the handler's own caller identity
//! appended, so the graft rule authorizes the kernel call against the
//! chain's initiating principal and the in-broker leg records the
//! correlation leg (KTD6).
//!
//! The handler table is the declaration itself: the descriptor this crate
//! publishes carries [`network_family_operations`], and the daemon's
//! registry serves the handlers from there. There is no second registration
//! step.

use std::os::fd::OwnedFd;
use std::sync::LazyLock;
use std::time::Duration;

use async_trait::async_trait;
use d2b_contracts_broker::broker_wire::{
    ApplyNmUnmanagedRequest, ApplyNftablesProjectionRequest, ApplyNftablesRequest,
    ApplyRouteRequest, ApplySysctlRequest, CreateBridgeRequest, CreatePersistentTapRequest,
    CreateTapFdRequest, DeleteBridgeRequest, DeletePersistentTapRequest, NftablesProjectionAction,
    SeedDnsmasqLeaseRequest, SetBridgePortFlagsRequest, UpdateHostsFileRequest,
};
use d2b_contracts_broker::kernel_client::{
    KernelInvocation, KernelInvokeError, KernelReply, envelope_invoke_kernel,
};
use d2b_contracts_resource::v3::{
    CanonicalJsonObject, NetworkProvenance, ResourceBundleGenerationId, ResourceGeneration,
    ResourceRef, ResourceUid, canonical_json_bytes,
};
use d2b_core::bundle_resolver::BundleResolver;
use d2b_resource_types::{
    OperationCtx, OperationDef, OperationFailure, OperationHandler, OperationResult,
    ValidatedPayload,
};
use d2b_core::kernel_seat::{self, KernelRefusal};

/// The family's `ApplyNftables` operation (U12).
pub const APPLY_NFTABLES: &str = "ApplyNftables";

/// The family's `ApplyNftablesProjection` operation (U12).
pub const APPLY_NFTABLES_PROJECTION: &str = "ApplyNftablesProjection";

/// The family's `ApplyNmUnmanaged` operation (U12).
pub const APPLY_NM_UNMANAGED: &str = "ApplyNmUnmanaged";

/// The family's `ApplyRoute` operation (U12).
pub const APPLY_ROUTE: &str = "ApplyRoute";

/// The family's `ApplySysctl` operation (U12).
pub const APPLY_SYSCTL: &str = "ApplySysctl";

/// The family's `CreateBridge` operation (U12).
pub const CREATE_BRIDGE: &str = "CreateBridge";

/// The family's `DeleteBridge` operation (U12).
pub const DELETE_BRIDGE: &str = "DeleteBridge";

/// The family's `CreatePersistentTap` operation (U12).
pub const CREATE_PERSISTENT_TAP: &str = "CreatePersistentTap";

/// The family's `DeletePersistentTap` operation (U12).
pub const DELETE_PERSISTENT_TAP: &str = "DeletePersistentTap";

/// The family's `CreateTapFd` operation (U12).
pub const CREATE_TAP_FD: &str = "CreateTapFd";

/// The family's `SetBridgePortFlags` operation (U12).
pub const SET_BRIDGE_PORT_FLAGS: &str = "SetBridgePortFlags";

/// The family's `UpdateHostsFile` operation (U12).
pub const UPDATE_HOSTS_FILE: &str = "UpdateHostsFile";

/// The family's `SeedDnsmasqLease` operation (U12).
pub const SEED_DNSMASQ_LEASE: &str = "SeedDnsmasqLease";

/// The broker-generic firewall projection kernel the family's
/// `ApplyNftablesProjection` operation invokes (U12).
pub const KERNEL_APPLY_NFTABLES_PROJECTION: &str = "apply-nftables-projection";

/// The broker-generic DHCP lease kernel the family's `SeedDnsmasqLease`
/// operation invokes (U12).
pub const KERNEL_SEED_DNSMASQ_LEASE: &str = "seed-dnsmasq-lease";

/// The refusal of a family operation whose kernel seam was never wired.
///
/// The composition point wires one [`KernelCaller`] per Zone alongside the
/// provider publication; a Zone whose seam was never wired serves forwarded
/// family operations without a kernel leg and every handler that needs one
/// refuses with this code.
pub const KERNEL_SEAM_UNWIRED: &str = "kernel-seam-unwired";

/// The refusal of a family operation whose nested kernel leg refused or
/// errored.
///
/// The kernel's own closed code ("errored" for a kernel error,
/// "handler-refused" for a kernel refusal) is preserved so the caller's
/// classification keeps working; the detail names the kernel and its reason.
pub const KERNEL_REFUSED: &str = "handler-refused";

/// The refusal of a typed request that disagrees with the trusted bundle.
///
/// Mirrors the retired broker arms' `BundleIntentMissing` /
/// `NetworkAdmissionMismatch` posture: the family handler resolves the
/// trusted intent from the Zone's bundle and refuses a request that names
/// a different identity by field.
pub const INTENT_MISMATCH: &str = "handler-refused";

/// The broker kernel IO budget one nested invocation may take.
const KERNEL_IO_TIMEOUT: Duration = Duration::from_secs(10);

/// The family's declared operations, assembled once.
///
/// The operation reference is an owned validated value, so the table is
/// built on first use rather than declared `const`; every descriptor the
/// family publishes shares the one table.
pub fn network_family_operations() -> &'static [OperationDef] {
    &NETWORK_FAMILY_OPERATIONS[..]
}

// The committed rows spell the family operations in the catalog's
// PascalCase wire names (`ApplyNftables`, `CreateTapFd`, ...), while a
// `ResourceRef` name is a lowercase label. The envelope matches the
// forwarded wire name case-insensitively (the U12 seam), so the table
// declares each operation under its parseable lowercase reference; the
// public constants above keep the catalog's wire spellings.
static NETWORK_FAMILY_OPERATIONS: LazyLock<[OperationDef; 13]> = LazyLock::new(|| {
    [
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/apply-nftables")
                .expect("the family's operation reference is canonical"),
            handler: &APPLY_NFTABLES_HANDLER,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/apply-nftables-projection")
                .expect("the family's operation reference is canonical"),
            handler: &APPLY_NFTABLES_PROJECTION_HANDLER,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/apply-nm-unmanaged")
                .expect("the family's operation reference is canonical"),
            handler: &APPLY_NM_UNMANAGED_HANDLER,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/apply-route")
                .expect("the family's operation reference is canonical"),
            handler: &APPLY_ROUTE_HANDLER,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/apply-sysctl")
                .expect("the family's operation reference is canonical"),
            handler: &APPLY_SYSCTL_HANDLER,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/create-bridge")
                .expect("the family's operation reference is canonical"),
            handler: &CREATE_BRIDGE_HANDLER,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/delete-bridge")
                .expect("the family's operation reference is canonical"),
            handler: &DELETE_BRIDGE_HANDLER,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/create-persistent-tap")
                .expect("the family's operation reference is canonical"),
            handler: &CREATE_PERSISTENT_TAP_HANDLER,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/delete-persistent-tap")
                .expect("the family's operation reference is canonical"),
            handler: &DELETE_PERSISTENT_TAP_HANDLER,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/create-tap-fd")
                .expect("the family's operation reference is canonical"),
            handler: &CREATE_TAP_FD_HANDLER,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/set-bridge-port-flags")
                .expect("the family's operation reference is canonical"),
            handler: &SET_BRIDGE_PORT_FLAGS_HANDLER,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/update-hosts-file")
                .expect("the family's operation reference is canonical"),
            handler: &UPDATE_HOSTS_FILE_HANDLER,
        },
        OperationDef {
            operation_ref: ResourceRef::parse("Operation/seed-dnsmasq-lease")
                .expect("the family's operation reference is canonical"),
            handler: &SEED_DNSMASQ_LEASE_HANDLER,
        },
    ]
});

static APPLY_NFTABLES_HANDLER: ApplyNftablesHandler = ApplyNftablesHandler;
static APPLY_NFTABLES_PROJECTION_HANDLER: ApplyNftablesProjectionHandler =
    ApplyNftablesProjectionHandler;
static APPLY_NM_UNMANAGED_HANDLER: ApplyNmUnmanagedHandler = ApplyNmUnmanagedHandler;
static APPLY_ROUTE_HANDLER: ApplyRouteHandler = ApplyRouteHandler;
static APPLY_SYSCTL_HANDLER: ApplySysctlHandler = ApplySysctlHandler;
static CREATE_BRIDGE_HANDLER: CreateBridgeHandler = CreateBridgeHandler;
static DELETE_BRIDGE_HANDLER: DeleteBridgeHandler = DeleteBridgeHandler;
static CREATE_PERSISTENT_TAP_HANDLER: CreatePersistentTapHandler = CreatePersistentTapHandler;
static DELETE_PERSISTENT_TAP_HANDLER: DeletePersistentTapHandler = DeletePersistentTapHandler;
static CREATE_TAP_FD_HANDLER: CreateTapFdHandler = CreateTapFdHandler;
static SET_BRIDGE_PORT_FLAGS_HANDLER: SetBridgePortFlagsHandler = SetBridgePortFlagsHandler;
static UPDATE_HOSTS_FILE_HANDLER: UpdateHostsFileHandler = UpdateHostsFileHandler;
static SEED_DNSMASQ_LEASE_HANDLER: SeedDnsmasqLeaseHandler = SeedDnsmasqLeaseHandler;

// ---------------------------------------------------------------------------
// The nested kernel leg
// ---------------------------------------------------------------------------

/// Invoke one broker-generic kernel as the nested core of a family operation.
///
/// The kernel call re-presents the evidence chain the forwarded invocation
/// runs on - the root invocation id plus the ordered identities - with the
/// handler's own caller identity appended, so the graft rule authorizes the
/// kernel call against the chain's initiating principal and the in-broker
/// leg records the correlation leg keyed by the same root invocation id
/// (KTD6). The kernel's reply carries the envelope response plus the
/// descriptors the kernel minted, in frame order.
async fn invoke_kernel_nested(
    ctx: &OperationCtx<'_>,
    kernel_operation: &'static str,
    payload: serde_json::Value,
    fds: Vec<OwnedFd>,
) -> Result<KernelReply, OperationFailure> {
    let kernel = ctx
        .kernel
        .ok_or_else(|| OperationFailure::new(KERNEL_SEAM_UNWIRED))?;
    let socket_path = kernel.socket_path.clone();
    let caller_role = kernel.caller_role.clone();
    let zone = ctx.zone.as_str().to_owned();
    let invocation_id = ctx.invocation_id.to_owned();
    let mut chain_identities = ctx.chain_identities.to_vec();
    chain_identities.push(ctx.caller.to_canonical_string());
    let reply = match kernel_seat::run(move || {
        envelope_invoke_kernel(
            &socket_path,
            KERNEL_IO_TIMEOUT,
            caller_role,
            KernelInvocation {
                operation: kernel_operation,
                zone: &zone,
                payload,
                fds: &fds,
                chain_root_invocation_id: Some(&invocation_id),
                chain_identities: Some(&chain_identities),
            },
        )
    })
    .await
    {
        Ok(Ok(reply)) => reply,
        Ok(Err(error)) => {
        // The kernel's own closed code is preserved: "errored" for a kernel
        // error, "handler-refused" for a kernel refusal, so the caller's
        // classification of the leg keeps working exactly as it did for the
        // retired typed arms.
        let (code, detail) = match error {
            KernelInvokeError::Refused { code, detail } => (code, detail),
            other => ("errored".to_owned(), Some(other.to_string())),
        };
        return Err(OperationFailure::with_detail(
            if code == "errored" {
                "errored"
            } else {
                KERNEL_REFUSED
            },
            format!(
                "{kernel_operation}: {code}{}",
                detail
                    .map(|detail| format!(" ({detail})"))
                    .unwrap_or_default()
            ),
        ));
        }
        Err(KernelRefusal::Busy) => {
            return Err(OperationFailure::with_detail(
                KERNEL_REFUSED,
                format!("{kernel_operation}: kernel worker busy"),
            ));
        }
        Err(KernelRefusal::Unavailable) => {
            return Err(OperationFailure::with_detail(
                KERNEL_REFUSED,
                format!("{kernel_operation}: kernel worker unavailable"),
            ));
        }
    };
    Ok(reply)
}

/// The canonical result object of one kernel reply.
///
/// A kernel reply that carries no result object is a protocol violation on
/// the broker side; the handler refuses rather than inventing a result.
fn kernel_result(reply: &KernelReply) -> Result<&serde_json::Value, OperationFailure> {
    reply.response.result.as_ref().ok_or_else(|| {
        OperationFailure::with_detail(
            KERNEL_REFUSED,
            format!("{}: kernel reply carries no result", reply.response.operation),
        )
    })
}

/// The kernel reply's result object as a canonical object.
fn kernel_result_object(reply: &KernelReply) -> Result<CanonicalJsonObject, OperationFailure> {
    let value = kernel_result(reply)?;
    let bytes = canonical_json_bytes(value).map_err(|error| {
        OperationFailure::with_detail(
            KERNEL_REFUSED,
            format!("{}: kernel result is not canonical JSON: {error}", reply.response.operation),
        )
    })?;
    CanonicalJsonObject::parse(&bytes).map_err(|error| {
        OperationFailure::with_detail(
            KERNEL_REFUSED,
            format!("{}: kernel result is not a canonical object: {error}", reply.response.operation),
        )
    })
}

/// Take one descriptor out of a kernel reply's fd vector by index.
///
/// The reply's descriptors are owned by the reply; taking one by index
/// leaves the rest to drop with the reply.
fn take_reply_fd(reply: KernelReply, index: usize) -> Result<OwnedFd, OperationFailure> {
    reply.fds.into_iter().nth(index).ok_or_else(|| {
        OperationFailure::with_detail(
            KERNEL_REFUSED,
            format!("{}: fd index {index} missing", reply.response.operation),
        )
    })
}

/// Deserialize one typed request from the forwarded payload.
fn typed_request<T: serde::de::DeserializeOwned>(
    operation: &str,
    payload: &ValidatedPayload,
) -> Result<T, OperationFailure> {
    let value = serde_json::to_value(payload.object()).map_err(|error| {
        OperationFailure::with_detail(
            KERNEL_REFUSED,
            format!("{operation}: payload conversion failed: {error}"),
        )
    })?;
    serde_json::from_value(value).map_err(|error| {
        OperationFailure::with_detail(
            KERNEL_REFUSED,
            format!("{operation}: invalid typed request: {error}"),
        )
    })
}

/// The Zone's trusted bundle one family handler resolves intents from.
fn kernel_bundle<'a>(ctx: &'a OperationCtx<'a>) -> Result<&'a BundleResolver, OperationFailure> {
    ctx.kernel
        .map(|kernel| kernel.bundle.as_ref())
        .ok_or_else(|| OperationFailure::new(KERNEL_SEAM_UNWIRED))
}

/// Map a trusted-bundle Network spec parse failure onto the family refusal
/// surface, keeping the manifest-parse-error reason in the detail.
fn intent_parse_failure(operation: &str, error: d2b_contracts::error::Error) -> OperationFailure {
    OperationFailure::with_detail(
        INTENT_MISMATCH,
        format!("{operation}: trusted bundle Network spec parse failed: {error}"),
    )
}

/// The exact Network effect provenance one request's identity tuple names.
///
/// Mirrors the retired arms' provenance derivation: the complete admitted
/// Network identity is the tuple every owned host effect is bound to.
fn network_provenance(
    zone_uid: &ResourceUid,
    network_uid: &ResourceUid,
    network_generation: &ResourceGeneration,
    attachment_generation: &ResourceGeneration,
    bundle_generation: &ResourceBundleGenerationId,
) -> NetworkProvenance {
    NetworkProvenance::new(
        zone_uid.clone(),
        network_uid.clone(),
        *network_generation,
        *attachment_generation,
        bundle_generation.clone(),
    )
}

/// The resolved bridge intent payload one bridge kernel invocation carries.
fn resolved_bridge_payload(
    intent: &d2b_core::bundle_resolver::ResolvedBridgeIntent,
) -> Result<serde_json::Value, OperationFailure> {
    Ok(serde_json::json!({
        "intentId": intent.intent_id,
        "scopeLabel": intent.scope_label,
        "bridgeIfname": intent.bridge_ifname.as_str(),
        "mtu": intent.mtu,
        "stpDisabled": intent.stp_disabled,
        "multicastSnoopingDisabled": intent.multicast_snooping_disabled,
        "ipv6Suppressed": intent.ipv6_suppressed,
        "ipv4Address": intent.ipv4_address.as_ref().map(|cidr| cidr.as_str()),
        "provenance": intent.provenance.as_ref().map(serde_json::to_value).transpose().map_err(|error| {
            OperationFailure::with_detail(
                KERNEL_REFUSED,
                format!("provenance serialization failed: {error}"),
            )
        })?,
        "ownershipMarker": intent.ownership_marker,
    }))
}

/// The resolved route intent payload one apply-route kernel invocation
/// carries.
fn resolved_route_payload(
    intent: &d2b_core::bundle_resolver::ResolvedRouteIntent,
    provenance: &NetworkProvenance,
    destroy: bool,
) -> Result<serde_json::Value, OperationFailure> {
    Ok(serde_json::json!({
        "intentId": intent.intent_id,
        "routeSpec": intent.route_spec,
        "destination": intent.destination,
        "via": intent.via,
        "device": intent.device,
        "table": intent.table,
        "owned": intent.owned,
        "routeName": intent.route_name,
        "provenance": serde_json::to_value(provenance).map_err(|error| {
            OperationFailure::with_detail(
                KERNEL_REFUSED,
                format!("provenance serialization failed: {error}"),
            )
        })?,
        "ownershipMarker": intent.ownership_marker,
        "destroy": destroy,
    }))
}

// ---------------------------------------------------------------------------
// The ApplyNftables handler
// ---------------------------------------------------------------------------

/// The handler of [`APPLY_NFTABLES`].
struct ApplyNftablesHandler;

#[async_trait]
impl OperationHandler for ApplyNftablesHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: ApplyNftablesRequest = typed_request(APPLY_NFTABLES, &payload)?;
        let bundle = kernel_bundle(&ctx)?;
        let intent = bundle
            .find_nft_intent(request.bundle_nft_intent_ref.as_str())
            .ok_or_else(|| {
                OperationFailure::with_detail(
                    INTENT_MISMATCH,
                    format!(
                        "ApplyNftables: unknown nft intent {}",
                        request.bundle_nft_intent_ref.as_str()
                    ),
                )
            })?;
        let reply = invoke_kernel_nested(
            &ctx,
            "apply-nftables",
            serde_json::json!({
                "family": bundle.host().nftables.family,
                "table": bundle.host().nftables.table,
                "scriptBody": intent.script_body,
                "ownershipId": intent.ownership_id,
                "destroy": request.destroy,
                "desiredHash": request.desired_hash,
                "tableHashAfterApply": bundle.host().nftables.table_hash_after_apply,
                "coexistencePolicy": bundle.host().firewall_coexistence_policy,
            }),
            Vec::new(),
        )
        .await?;
        Ok(OperationResult::new(kernel_result_object(&reply)?))
    }
}

// ---------------------------------------------------------------------------
// The ApplyNftablesProjection handler
// ---------------------------------------------------------------------------

/// The handler of [`APPLY_NFTABLES_PROJECTION`].
struct ApplyNftablesProjectionHandler;

#[async_trait]
impl OperationHandler for ApplyNftablesProjectionHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: ApplyNftablesProjectionRequest =
            typed_request(APPLY_NFTABLES_PROJECTION, &payload)?;
        let bundle = kernel_bundle(&ctx)?;
        let provenance = network_provenance(
            &request.zone_uid,
            &request.network_uid,
            &request.network_generation,
            &request.attachment_generation,
            &request.expected_generation_id,
        );
        let intent = bundle
            .resolve_network_projection_intent(
                request.bundle_nft_projection_intent_ref.as_str(),
                &provenance,
            )
            .map_err(|error| intent_parse_failure("ApplyNftablesProjection", error))?
            .ok_or_else(|| {
                OperationFailure::with_detail(
                    INTENT_MISMATCH,
                    format!(
                        "ApplyNftablesProjection: unknown projection intent {}",
                        request.bundle_nft_projection_intent_ref.as_str()
                    ),
                )
            })?;
        let marker = bundle
            .resolve_network_marker_intent(&intent.ownership_marker_intent_ref, &provenance)
            .map_err(|error| intent_parse_failure("ApplyNftablesProjection", error))?
            .ok_or_else(|| {
                OperationFailure::with_detail(
                    INTENT_MISMATCH,
                    format!(
                        "ApplyNftablesProjection: unknown ownership marker {}",
                        intent.ownership_marker_intent_ref
                    ),
                )
            })?;
        let installed = bundle.installed_generation_identity().ok_or_else(|| {
            OperationFailure::with_detail(
                KERNEL_REFUSED,
                "ApplyNftablesProjection: installed-generation-unavailable".to_owned(),
            )
        })?;
        if installed.as_str() != request.expected_generation_id.as_str() {
            return Err(OperationFailure::with_detail(
                INTENT_MISMATCH,
                "ApplyNftablesProjection: stale-projection-generation".to_owned(),
            ));
        }
        let action = match request.action {
            NftablesProjectionAction::Apply => "apply",
            NftablesProjectionAction::Remove => "remove",
        };
        let reply = invoke_kernel_nested(
            &ctx,
            "apply-nftables-projection",
            serde_json::json!({
                "scriptBody": intent.script_body,
                "marker": marker.marker,
                "trustedHash": intent.desired_hash,
                "callerHash": request.desired_hash,
                "expectedGenerationId": request.expected_generation_id.as_str(),
                "installedGenerationId": installed.as_str(),
                "action": action,
            }),
            Vec::new(),
        )
        .await?;
        Ok(OperationResult::new(kernel_result_object(&reply)?))
    }
}

// ---------------------------------------------------------------------------
// The ApplyNmUnmanaged handler
// ---------------------------------------------------------------------------

/// The handler of [`APPLY_NM_UNMANAGED`].
struct ApplyNmUnmanagedHandler;

#[async_trait]
impl OperationHandler for ApplyNmUnmanagedHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: ApplyNmUnmanagedRequest = typed_request(APPLY_NM_UNMANAGED, &payload)?;
        let bundle = kernel_bundle(&ctx)?;
        let intent = bundle
            .find_nm_unmanaged_intent(request.bundle_nm_intent_ref.as_str())
            .ok_or_else(|| {
                OperationFailure::with_detail(
                    INTENT_MISMATCH,
                    format!(
                        "ApplyNmUnmanaged: unknown nm-unmanaged intent {}",
                        request.bundle_nm_intent_ref.as_str()
                    ),
                )
            })?
            .clone();
        let reply = invoke_kernel_nested(
            &ctx,
            "apply-nm-unmanaged",
            serde_json::json!({
                "intentId": intent.intent_id,
                "filePath": intent.file_path.display().to_string(),
                "contents": intent.contents,
                "mode": intent.mode,
                "owner": intent.owner,
                "group": intent.group,
                "reloadBehavior": intent.reload_behavior,
                "destroy": request.destroy,
            }),
            Vec::new(),
        )
        .await?;
        Ok(OperationResult::new(kernel_result_object(&reply)?))
    }
}

// ---------------------------------------------------------------------------
// The ApplyRoute handler
// ---------------------------------------------------------------------------

/// The handler of [`APPLY_ROUTE`].
struct ApplyRouteHandler;

#[async_trait]
impl OperationHandler for ApplyRouteHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: ApplyRouteRequest = typed_request(APPLY_ROUTE, &payload)?;
        let bundle = kernel_bundle(&ctx)?;
        let provenance = network_provenance(
            &request.zone_uid,
            &request.network_uid,
            &request.network_generation,
            &request.attachment_generation,
            &request.bundle_generation,
        );
        let intent = bundle
            .resolve_network_route_intent(request.bundle_route_intent_ref.as_str(), &provenance)
            .map_err(|error| intent_parse_failure("ApplyRoute", error))?
            .ok_or_else(|| {
                OperationFailure::with_detail(
                    INTENT_MISMATCH,
                    format!(
                        "ApplyRoute: unknown route intent {}",
                        request.bundle_route_intent_ref.as_str()
                    ),
                )
            })?;
        let reply = invoke_kernel_nested(
            &ctx,
            "apply-route",
            resolved_route_payload(&intent, &provenance, request.destroy)?,
            Vec::new(),
        )
        .await?;
        Ok(OperationResult::new(kernel_result_object(&reply)?))
    }
}

// ---------------------------------------------------------------------------
// The ApplySysctl handler
// ---------------------------------------------------------------------------

/// The handler of [`APPLY_SYSCTL`].
struct ApplySysctlHandler;

#[async_trait]
impl OperationHandler for ApplySysctlHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: ApplySysctlRequest = typed_request(APPLY_SYSCTL, &payload)?;
        let bundle = kernel_bundle(&ctx)?;
        let provenance = network_provenance(
            &request.zone_uid,
            &request.network_uid,
            &request.network_generation,
            &request.attachment_generation,
            &request.bundle_generation,
        );
        let intent = bundle
            .resolve_network_sysctl_intent(request.bundle_sysctl_intent_ref.as_str(), &provenance)
            .map_err(|error| intent_parse_failure("ApplySysctl", error))?
            .ok_or_else(|| {
                OperationFailure::with_detail(
                    INTENT_MISMATCH,
                    format!(
                        "ApplySysctl: unknown sysctl intent {}",
                        request.bundle_sysctl_intent_ref.as_str()
                    ),
                )
            })?;
        let reply = invoke_kernel_nested(
            &ctx,
            "apply-sysctl",
            serde_json::json!({
                "key": intent.key,
                "value": intent.value,
                "destroy": request.destroy,
            }),
            Vec::new(),
        )
        .await?;
        Ok(OperationResult::new(kernel_result_object(&reply)?))
    }
}

// ---------------------------------------------------------------------------
// The CreateBridge / DeleteBridge handlers
// ---------------------------------------------------------------------------

/// The handler of [`CREATE_BRIDGE`].
struct CreateBridgeHandler;

#[async_trait]
impl OperationHandler for CreateBridgeHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: CreateBridgeRequest = typed_request(CREATE_BRIDGE, &payload)?;
        let bundle = kernel_bundle(&ctx)?;
        let provenance = network_provenance(
            &request.zone_uid,
            &request.network_uid,
            &request.network_generation,
            &request.attachment_generation,
            &request.bundle_generation,
        );
        let intent = bundle
            .resolve_network_bridge_intent(request.bundle_bridge_intent_ref.as_str(), &provenance)
            .map_err(|error| intent_parse_failure("CreateBridge", error))?
            .ok_or_else(|| {
                OperationFailure::with_detail(
                    INTENT_MISMATCH,
                    format!(
                        "CreateBridge: unknown bridge intent {}",
                        request.bundle_bridge_intent_ref.as_str()
                    ),
                )
            })?;
        let reply = invoke_kernel_nested(
            &ctx,
            "create-bridge",
            resolved_bridge_payload(&intent)?,
            Vec::new(),
        )
        .await?;
        Ok(OperationResult::new(kernel_result_object(&reply)?))
    }
}

/// The handler of [`DELETE_BRIDGE`].
struct DeleteBridgeHandler;

#[async_trait]
impl OperationHandler for DeleteBridgeHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: DeleteBridgeRequest = typed_request(DELETE_BRIDGE, &payload)?;
        let bundle = kernel_bundle(&ctx)?;
        let provenance = network_provenance(
            &request.zone_uid,
            &request.network_uid,
            &request.network_generation,
            &request.attachment_generation,
            &request.bundle_generation,
        );
        let intent = bundle
            .resolve_network_bridge_intent(request.bundle_bridge_intent_ref.as_str(), &provenance)
            .map_err(|error| intent_parse_failure("DeleteBridge", error))?
            .ok_or_else(|| {
                OperationFailure::with_detail(
                    INTENT_MISMATCH,
                    format!(
                        "DeleteBridge: unknown bridge intent {}",
                        request.bundle_bridge_intent_ref.as_str()
                    ),
                )
            })?;
        let reply = invoke_kernel_nested(
            &ctx,
            "delete-bridge",
            resolved_bridge_payload(&intent)?,
            Vec::new(),
        )
        .await?;
        Ok(OperationResult::new(kernel_result_object(&reply)?))
    }
}

// ---------------------------------------------------------------------------
// The CreatePersistentTap handler
// ---------------------------------------------------------------------------

/// The handler of [`CREATE_PERSISTENT_TAP`].
///
/// The kernel re-derives the trusted tap intent from the broker's own
/// bundle copy, so the handler forwards the typed request intact after
/// parsing it; the parse is the payload validation.
struct CreatePersistentTapHandler;

#[async_trait]
impl OperationHandler for CreatePersistentTapHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: CreatePersistentTapRequest = typed_request(CREATE_PERSISTENT_TAP, &payload)?;
        let reply = invoke_kernel_nested(
            &ctx,
            "create-persistent-tap",
            serde_json::to_value(&request).map_err(|error| {
                OperationFailure::with_detail(
                    KERNEL_REFUSED,
                    format!("CreatePersistentTap: payload serialization failed: {error}"),
                )
            })?,
            Vec::new(),
        )
        .await?;
        Ok(OperationResult::new(kernel_result_object(&reply)?))
    }
}

// ---------------------------------------------------------------------------
// The DeletePersistentTap handler
// ---------------------------------------------------------------------------

/// The handler of [`DELETE_PERSISTENT_TAP`].
struct DeletePersistentTapHandler;

#[async_trait]
impl OperationHandler for DeletePersistentTapHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: DeletePersistentTapRequest = typed_request(DELETE_PERSISTENT_TAP, &payload)?;
        let reply = invoke_kernel_nested(
            &ctx,
            "delete-persistent-tap",
            serde_json::to_value(&request).map_err(|error| {
                OperationFailure::with_detail(
                    KERNEL_REFUSED,
                    format!("DeletePersistentTap: payload serialization failed: {error}"),
                )
            })?,
            Vec::new(),
        )
        .await?;
        Ok(OperationResult::new(kernel_result_object(&reply)?))
    }
}

// ---------------------------------------------------------------------------
// The CreateTapFd handler
// ---------------------------------------------------------------------------

/// The handler of [`CREATE_TAP_FD`].
///
/// This is the ONLY fd-bearing network family operation: the kernel mints
/// the VMM TAP descriptor and returns it over the fd leg, and the handler
/// carries it back in the operation result.
struct CreateTapFdHandler;

#[async_trait]
impl OperationHandler for CreateTapFdHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: CreateTapFdRequest = typed_request(CREATE_TAP_FD, &payload)?;
        let reply = invoke_kernel_nested(
            &ctx,
            "create-tap-fd",
            serde_json::to_value(&request).map_err(|error| {
                OperationFailure::with_detail(
                    KERNEL_REFUSED,
                    format!("CreateTapFd: payload serialization failed: {error}"),
                )
            })?,
            Vec::new(),
        )
        .await?;
        let result = kernel_result_object(&reply)?;
        let fd = take_reply_fd(reply, 0)?;
        Ok(OperationResult::with_fds(result, vec![fd]))
    }
}

// ---------------------------------------------------------------------------
// The SetBridgePortFlags handler
// ---------------------------------------------------------------------------

/// The handler of [`SET_BRIDGE_PORT_FLAGS`].
struct SetBridgePortFlagsHandler;

#[async_trait]
impl OperationHandler for SetBridgePortFlagsHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: SetBridgePortFlagsRequest = typed_request(SET_BRIDGE_PORT_FLAGS, &payload)?;
        let reply = invoke_kernel_nested(
            &ctx,
            "set-bridge-port-flags",
            serde_json::to_value(&request).map_err(|error| {
                OperationFailure::with_detail(
                    KERNEL_REFUSED,
                    format!("SetBridgePortFlags: payload serialization failed: {error}"),
                )
            })?,
            Vec::new(),
        )
        .await?;
        Ok(OperationResult::new(kernel_result_object(&reply)?))
    }
}

// ---------------------------------------------------------------------------
// The UpdateHostsFile handler
// ---------------------------------------------------------------------------

/// The handler of [`UPDATE_HOSTS_FILE`].
struct UpdateHostsFileHandler;

#[async_trait]
impl OperationHandler for UpdateHostsFileHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: UpdateHostsFileRequest = typed_request(UPDATE_HOSTS_FILE, &payload)?;
        let bundle = kernel_bundle(&ctx)?;
        let (intent, provenance) =
            if request.bundle_hosts_intent_ref.as_str().starts_with("network-hosts:") {
                let Some(zone_uid) = request.zone_uid.as_ref() else {
                    return Err(OperationFailure::with_detail(
                        INTENT_MISMATCH,
                        "UpdateHostsFile: network-hosts intent without a zone identity"
                            .to_owned(),
                    ));
                };
                let Some(network_uid) = request.network_uid.as_ref() else {
                    return Err(OperationFailure::with_detail(
                        INTENT_MISMATCH,
                        "UpdateHostsFile: network-hosts intent without a network identity"
                            .to_owned(),
                    ));
                };
                let Some(network_generation) = request.network_generation.as_ref() else {
                    return Err(OperationFailure::with_detail(
                        INTENT_MISMATCH,
                        "UpdateHostsFile: network-hosts intent without a network generation"
                            .to_owned(),
                    ));
                };
                let Some(attachment_generation) = request.attachment_generation.as_ref() else {
                    return Err(OperationFailure::with_detail(
                        INTENT_MISMATCH,
                        "UpdateHostsFile: network-hosts intent without an attachment generation"
                            .to_owned(),
                    ));
                };
                let Some(bundle_generation) = request.bundle_generation.as_ref() else {
                    return Err(OperationFailure::with_detail(
                        INTENT_MISMATCH,
                        "UpdateHostsFile: network-hosts intent without a bundle generation"
                            .to_owned(),
                    ));
                };
                let provenance = network_provenance(
                    zone_uid,
                    network_uid,
                    network_generation,
                    attachment_generation,
                    bundle_generation,
                );
                let intent = bundle
                    .resolve_network_hosts_intent(
                        request.bundle_hosts_intent_ref.as_str(),
                        &provenance,
                    )
                    .map_err(|error| intent_parse_failure("UpdateHostsFile", error))?
                    .ok_or_else(|| {
                        OperationFailure::with_detail(
                            INTENT_MISMATCH,
                            format!(
                                "UpdateHostsFile: unknown network-hosts intent {}",
                                request.bundle_hosts_intent_ref.as_str()
                            ),
                        )
                    })?;
                (intent, Some(provenance))
            } else {
                let intent = bundle
                    .find_hosts_intent(request.bundle_hosts_intent_ref.as_str())
                    .ok_or_else(|| {
                        OperationFailure::with_detail(
                            INTENT_MISMATCH,
                            format!(
                                "UpdateHostsFile: unknown hosts intent {}",
                                request.bundle_hosts_intent_ref.as_str()
                            ),
                        )
                    })?
                    .clone();
                (intent, None)
            };
        let reply = invoke_kernel_nested(
            &ctx,
            "update-hosts-file",
            serde_json::json!({
                "intentId": intent.intent_id,
                "path": intent.path.display().to_string(),
                "managedBlock": intent.managed_block,
                "startMarker": intent.start_marker,
                "endMarker": intent.end_marker,
                "mode": intent.mode,
                "provenance": provenance.as_ref().map(serde_json::to_value).transpose().map_err(|error| {
                    OperationFailure::with_detail(
                        KERNEL_REFUSED,
                        format!("UpdateHostsFile: provenance serialization failed: {error}"),
                    )
                })?,
                "ownershipMarker": intent.ownership_marker,
                "destroy": request.destroy,
            }),
            Vec::new(),
        )
        .await?;
        Ok(OperationResult::new(kernel_result_object(&reply)?))
    }
}

// ---------------------------------------------------------------------------
// The SeedDnsmasqLease handler
// ---------------------------------------------------------------------------

/// The handler of [`SEED_DNSMASQ_LEASE`].
struct SeedDnsmasqLeaseHandler;

#[async_trait]
impl OperationHandler for SeedDnsmasqLeaseHandler {
    async fn execute(
        &self,
        ctx: OperationCtx<'_>,
        payload: ValidatedPayload,
    ) -> Result<OperationResult, OperationFailure> {
        let request: SeedDnsmasqLeaseRequest = typed_request(SEED_DNSMASQ_LEASE, &payload)?;
        let reply = invoke_kernel_nested(
            &ctx,
            "seed-dnsmasq-lease",
            serde_json::json!({
                "vmId": request.vm_id.as_str(),
                "scopeId": request.scope_id.as_str(),
                "zoneUid": request.zone_uid.as_str(),
                "networkUid": request.network_uid.as_str(),
                "networkGeneration": request.network_generation.get(),
                "attachmentGeneration": request.attachment_generation.get(),
                "bundleGeneration": request.bundle_generation.as_str(),
            }),
            Vec::new(),
        )
        .await?;
        Ok(OperationResult::new(kernel_result_object(&reply)?))
    }
}

