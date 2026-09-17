//! The Network-local family's declared operations and their handlers (U12).
//!
//! Each handler validates the typed request the envelope forwarded, resolves
//! the trusted inputs it needs from the Zone's bundle through the family
//! seam, and invokes the matching broker-generic kernel as a nested envelope
//! call. These tests drive the real handlers against a fake kernel socket:
//! the server leg answers the exact `EnvelopeInvoke` frame the kernel client
//! sends, so the assertions are grounded in what actually crossed the socket
//!   - the operation name, the zone, the resolved payload, and the evidence
//!     chain (root invocation id plus ordered identities with the handler's
//!     own caller identity appended).

use std::collections::BTreeMap;
use std::io::IoSlice;
use std::os::fd::AsFd;
use std::path::PathBuf;
use parking_lot::Mutex;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use d2b_contracts::types::{BundleOpId, RoleId, ScopeId, VmId};
use d2b_contracts_broker::broker_wire::{
    ApplyNftablesRequest, BrokerCallerRole, BrokerRequestEnvelope, BrokerResponse,
    CreateTapFdRequest, EnvelopeInvokeResponse, SeedDnsmasqLeaseRequest,
};
use d2b_contracts_resource::v3::{
    CanonicalJsonObject, IfName, ResourceBundleGenerationId, ResourceGeneration, ResourceRef,
    ResourceUid, ZoneId, canonical_json_bytes,
};
use d2b_core::bundle::{Bundle, BundleGeneration};
use d2b_core::bundle_resolver::BundleResolver;
use d2b_core::host::HostJson;
use d2b_core::manifest_v04::ManifestV04;
use d2b_core::processes::ProcessesJson;
use d2b_provider_network_local::network_family_operations;
use d2b_resource_types::{
    KernelCaller, OperationCtx, OperationDef, OperationFailure, OperationHandler, OperationResult,
    ValidatedPayload,
};

const HOST_JSON: &str = include_str!("../../../tests/fixtures/deny-unknown/host-valid.json");
const MANIFEST_JSON: &[u8] = include_bytes!("../../../tests/golden/manifest_v04/baseline-vms.json");

fn zone_uid() -> ResourceUid {
    ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").unwrap()
}

fn network_uid() -> ResourceUid {
    ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").unwrap()
}

fn bundle_generation() -> ResourceBundleGenerationId {
    ResourceBundleGenerationId::parse(
        "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    .unwrap()
}

fn resolver() -> BundleResolver {
    let mut host: HostJson = serde_json::from_str(HOST_JSON).expect("host fixture parses");
    host.nftables.ownership_id = "bundle-owner".to_owned();
    let bundle_hash = format!("sha256:{}", "a".repeat(64));
    BundleResolver::from_artifacts_with_zone_resource_bundles(
        Bundle {
            bundle_version: 1,
            schema_version: "v3".to_owned(),
            privileges_path: "privileges.json".to_owned(),
            storage_path: None,
            realm_workloads_launcher_v2_path: None,
            generation: BundleGeneration {
                generator: "test".to_owned(),
                source_revision: None,
                generated_at: None,
            },
            bundle_hash: Some(bundle_hash),
            artifact_hashes: None,
        },
        host,
        ProcessesJson {
            schema_version: "v3".to_owned(),
            vms: Vec::new(),
        },
        ManifestV04::from_slice(MANIFEST_JSON).expect("manifest fixture parses"),
        BTreeMap::new(),
    )
}

/// The handler one family operation reference names.
fn handler_for(operation: &str) -> &'static dyn OperationHandler {
    let defs: &[OperationDef] = network_family_operations();
    defs.iter()
        .find(|def| def.operation_ref.to_canonical_string() == operation)
        .unwrap_or_else(|| panic!("family table must declare {operation}"))
        .handler
}

/// A fake kernel socket: one accepted `EnvelopeInvoke` frame is captured,
/// and the canned reply (plus optional descriptors) travels back over the
/// same connection, exactly as the broker's origination socket would.
struct KernelServer {
    socket_path: PathBuf,
    captured: Arc<Mutex<Option<BrokerRequestEnvelope>>>,
    reply: Arc<Mutex<Option<BrokerResponse>>>,
    reply_fds: Arc<Mutex<Vec<std::os::fd::OwnedFd>>>,
    handle: Option<JoinHandle<()>>,
}

impl KernelServer {
    fn spawn(root: &tempfile::TempDir) -> Self {
        let socket_path = root.path().join("kernel.sock");
        let listener = socket2::Socket::new(
            socket2::Domain::UNIX,
            socket2::Type::SEQPACKET,
            None,
        )
        .expect("create kernel socket");
        listener
            .bind(&socket2::SockAddr::unix(&socket_path).expect("kernel socket address"))
            .expect("bind kernel socket");
        listener.listen(1).expect("listen kernel socket");
        let captured = Arc::new(Mutex::new(None));
        let reply = Arc::new(Mutex::new(None));
        let reply_fds = Arc::new(Mutex::new(Vec::new()));
        let captured_leg = Arc::clone(&captured);
        let reply_leg = Arc::clone(&reply);
        let reply_fds_leg = Arc::clone(&reply_fds);
let handle = std::thread::spawn(move || {
            serve_kernel_call(listener, captured_leg, reply_leg, reply_fds_leg);
        });
        Self {
            socket_path,
            captured,
            reply,
            reply_fds,
            handle: Some(handle),
        }
    }

    fn kernel(&self) -> KernelCaller {
        KernelCaller {
            socket_path: self.socket_path.clone(),
            caller_role: BrokerCallerRole::AdminUid { uid: 1000 },
            bundle: Arc::new(resolver()),
            runner_lookup: None,
        }
    }

    fn answer(&self, response: BrokerResponse) {
        *self.reply.lock() = Some(response);
    }

    fn answer_with_fds(&self, response: BrokerResponse, fds: Vec<std::os::fd::OwnedFd>) {
        *self.reply.lock() = Some(response);
        *self.reply_fds.lock() = fds;
    }

    // Joining the fake kernel server's thread is the sync test harness's own
    // blocking wait;the server leg itself runs on a plain test thread.

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn captured(&mut self) -> BrokerRequestEnvelope {
        self.handle
            .take()
            .expect("server joined once")
            .join()
            .expect("kernel server completes");
        self.captured
            .lock()
            .clone()
            .expect("the kernel server captured one frame")
    }
}

/// Serve one accepted kernel call:capture the EnvelopeInvoke frame and
/// sendthe canned reply over the same SEQPACKET connection, exactly as
/// the broker's origination socket would. Runs on the test's own blocking
/// thread;the poll-and-sleep wait for the caller's canned answer is part
/// of this sync test harness, so the leg carries the test-helper sanction.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn serve_kernel_call(
    listener: socket2::Socket,
    captured: Arc<Mutex<Option<BrokerRequestEnvelope>>>,
    reply: Arc<Mutex<Option<BrokerResponse>>>,
    reply_fds: Arc<Mutex<Vec<std::os::fd::OwnedFd>>>,
) {
    use std::io::Read;
    use std::io::Write;

    let (mut connection, _) = listener.accept().expect("accept kernel call");
    let mut buf = vec![0_u8; d2b_contracts::MAX_FRAME_SIZE + 4];
    let read = connection.read(&mut buf).expect("read kernel frame");
    let envelope: BrokerRequestEnvelope =
        d2b_contracts::decode_frame("BrokerRequestEnvelope", &buf[..read])
            .expect("decode kernel frame");
    *captured.lock() = Some(envelope);
    let response = loop {
        if let Some(response) = reply.lock().take() {
            break response;
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let frame = d2b_contracts::encode_frame(&response).expect("encode kernel reply");
    let fds = std::mem::take(&mut *reply_fds.lock());
    if fds.is_empty() {
        connection.write_all(&frame).expect("write kernel reply");
    } else {
        let descriptors = fds.iter().map(AsFd::as_fd).collect::<Vec<_>>();
        let mut control_bytes = vec![0_u8; rustix::cmsg_space!(ScmRights(256))];
        let mut control = rustix::net::SendAncillaryBuffer::new(&mut control_bytes);
        assert!(
            control.push(rustix::net::SendAncillaryMessage::ScmRights(&descriptors)),
            "fd control data accepted"
        );
        let iov = [IoSlice::new(&frame)];
        rustix::net::sendmsg(&connection, &iov, &mut control, rustix::net::SendFlags::empty())
            .expect("write kernel reply with fds");
    }
}

/// Invoke one family handler with a canonical payload under a fixed
/// evidence chain.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
fn invoke(
    handler: &'static dyn OperationHandler,
    kernel: Option<KernelCaller>,
    payload: serde_json::Value,
) -> Result<OperationResult, OperationFailure> {
    let zone = ZoneId::parse("zone-test").expect("valid zone");
    let caller = ResourceRef::parse("Provider/network-local").expect("valid caller ref");
    let operation = ResourceRef::parse("Operation/apply-nftables").expect("valid operation ref");
    let bytes = canonical_json_bytes(&payload).expect("canonical payload");
    let object = CanonicalJsonObject::parse(&bytes).expect("payload object");
    let ctx = OperationCtx {
        zone: &zone,
        caller: &caller,
        operation: &operation,
        invocation_id: "invocation-test",
        fds: &[],
        chain_identities: &["identity-a".to_owned(), "identity-b".to_owned()],
        kernel: kernel.as_ref(),
    };
    tokio::runtime::Runtime::new()
        .expect("runtime")
        .block_on(handler.execute(ctx, ValidatedPayload::new(object)))
}

fn envelope_response(operation: &str, result: serde_json::Value) -> BrokerResponse {
    BrokerResponse::EnvelopeInvoke(EnvelopeInvokeResponse {
        operation: operation.to_owned(),
        invocation_id: "invocation-kernel".to_owned(),
        result: Some(result),
        refusal: None,
        detail: None,
        fd_indexes: Vec::new(),
        fd_kinds: Vec::new(),
    })
}

fn seed_dnsmasq_request() -> SeedDnsmasqLeaseRequest {
    SeedDnsmasqLeaseRequest {
        vm_id: VmId::new("work-vm"),
        scope_id: ScopeId::new("network:zone:test"),
        zone_uid: zone_uid(),
        network_uid: network_uid(),
        network_generation: ResourceGeneration::new(7).unwrap(),
        attachment_generation: ResourceGeneration::new(11).unwrap(),
        bundle_generation: bundle_generation(),
        tracing_span_id: None,
    }
}

#[test]
fn the_family_declares_the_thirteen_network_operations() {
    let defs: &[OperationDef] = network_family_operations();
    let refs: Vec<String> = defs
        .iter()
        .map(|def| def.operation_ref.to_canonical_string())
        .collect();
    assert_eq!(
        refs,
        [
            "Operation/apply-nftables".to_owned(),
            "Operation/apply-nftables-projection".to_owned(),
            "Operation/apply-nm-unmanaged".to_owned(),
            "Operation/apply-route".to_owned(),
            "Operation/apply-sysctl".to_owned(),
            "Operation/create-bridge".to_owned(),
            "Operation/delete-bridge".to_owned(),
            "Operation/create-persistent-tap".to_owned(),
            "Operation/delete-persistent-tap".to_owned(),
            "Operation/create-tap-fd".to_owned(),
            "Operation/set-bridge-port-flags".to_owned(),
            "Operation/update-hosts-file".to_owned(),
            "Operation/seed-dnsmasq-lease".to_owned(),
        ]
    );
    let handlers: Vec<*const dyn OperationHandler> = defs
        .iter()
        .map(|def| def.handler as *const dyn OperationHandler)
        .collect();
    for (index, handler) in handlers.iter().enumerate() {
        for other in &handlers[index + 1..] {
            assert!(
                !std::ptr::eq(*handler, *other),
                "every declared operation has its own handler"
            );
        }
    }
}

#[test]
fn apply_nftables_resolves_the_trusted_intent_and_invokes_the_kernel_nested() {
    let root = tempfile::tempdir().expect("temp dir");
    let mut server = KernelServer::spawn(&root);
    server.answer(envelope_response("apply-nftables", serde_json::json!({})));

    let result = invoke(
        handler_for("Operation/apply-nftables"),
        Some(server.kernel()),
        serde_json::to_value(ApplyNftablesRequest {
            bundle_nft_intent_ref: BundleOpId::new("nft:host"),
            scope_id: ScopeId::new("host"),
            desired_hash: None,
            destroy: false,
            tracing_span_id: None,
        })
        .expect("request serializes"),
    )
    .expect("the handler serves the resolved intent");

    let captured = server.captured();
    let BrokerRequestEnvelope { request, caller_role, .. } = captured;
    let d2b_contracts_broker::broker_wire::BrokerRequest::EnvelopeInvoke(invoke) = request else {
        panic!("the kernel leg must carry an EnvelopeInvoke frame");
    };
    assert_eq!(invoke.operation, "apply-nftables");
    assert_eq!(invoke.zone, "zone-test");
    assert_eq!(caller_role, BrokerCallerRole::AdminUid { uid: 1000 });
    // The evidence chain is re-presented with the handler's own caller
    // identity appended (KTD6).
    assert_eq!(
        invoke.chain_root_invocation_id.as_deref(),
        Some("invocation-test")
    );
    assert_eq!(
        invoke.chain_identities.as_deref(),
        Some(&["identity-a".to_owned(), "identity-b".to_owned(), "Provider/network-local".to_owned()][..])
    );
    // The kernel payload is the resolved trusted intent, never caller
    // rule text: the fixture host's nft script body, ownership id, and
    // table identity.
    assert_eq!(invoke.payload.get("family").and_then(serde_json::Value::as_str), Some("inet"));
    assert_eq!(invoke.payload.get("table").and_then(serde_json::Value::as_str), Some("d2b"));
    assert_eq!(
        invoke.payload.get("destroy").and_then(serde_json::Value::as_bool),
        Some(false)
    );
    let script_body = invoke
        .payload
        .get("scriptBody")
        .and_then(serde_json::Value::as_str)
        .expect("resolved script body");
    assert!(!script_body.is_empty(), "the resolved nft script is carried");
    assert!(
        invoke
            .payload
            .get("ownershipId")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|id| !id.is_empty()),
        "the ownership id is carried: {:?}",
        invoke.payload
    );
    assert!(result.object().is_empty(), "the kernel result is the ack object");
}

#[test]
fn seed_dnsmasq_lease_invokes_the_kernel_nested_with_the_typed_request() {
    let root = tempfile::tempdir().expect("temp dir");
    let mut server = KernelServer::spawn(&root);
    server.answer(envelope_response(
        "seed-dnsmasq-lease",
        serde_json::json!({ "seeded": true }),
    ));

    let result = invoke(
        handler_for("Operation/seed-dnsmasq-lease"),
        Some(server.kernel()),
        serde_json::to_value(seed_dnsmasq_request()).expect("request serializes"),
    )
    .expect("the handler serves the typed request");

    let captured = server.captured();
    let d2b_contracts_broker::broker_wire::BrokerRequest::EnvelopeInvoke(invoke) =
        captured.request
    else {
        panic!("the kernel leg must carry an EnvelopeInvoke frame");
    };
    assert_eq!(invoke.operation, "seed-dnsmasq-lease");
    assert_eq!(
        invoke.payload.get("vmId").and_then(serde_json::Value::as_str),
        Some("work-vm")
    );
    assert_eq!(
        invoke.payload.get("zoneUid").and_then(serde_json::Value::as_str),
        Some(zone_uid().as_str())
    );
    assert_eq!(
        invoke.payload.get("networkUid").and_then(serde_json::Value::as_str),
        Some(network_uid().as_str())
    );
    assert_eq!(
        invoke.payload.get("networkGeneration").and_then(serde_json::Value::as_u64),
        Some(7)
    );
    assert_eq!(
        invoke.payload.get("attachmentGeneration").and_then(serde_json::Value::as_u64),
        Some(11)
    );
    assert_eq!(
        invoke.payload.get("bundleGeneration").and_then(serde_json::Value::as_str),
        Some(bundle_generation().as_str())
    );
    assert_eq!(
        result
            .object()
            .get("seeded")
            .map(serde_json::to_value)
            .transpose()
            .ok()
            .flatten()
            .and_then(|value| value.as_bool()),
        Some(true)
    );
}

#[test]
fn create_tap_fd_returns_the_kernel_descriptor_over_the_fd_leg() {

    let root = tempfile::tempdir().expect("temp dir");
    let mut server = KernelServer::spawn(&root);
    let (read_end, write_end) = rustix::pipe::pipe().expect("pipe for the fd leg");
    server.answer_with_fds(
        envelope_response(
            "create-tap-fd",
            serde_json::json!({ "bridge": null, "tap": "tap-work", "fdIndex": 0 }),
        ),
        vec![read_end],
    );

    let result = invoke(
        handler_for("Operation/create-tap-fd"),
        Some(server.kernel()),
        serde_json::to_value(CreateTapFdRequest {
            role_id: RoleId::new("network-attachment"),
            vm_id: VmId::new("work-vm"),
            bundle_tap_intent_ref: BundleOpId::new("tap:test"),
            attachment_id: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            network_generation: ResourceGeneration::new(7).unwrap(),
            attachment_generation: ResourceGeneration::new(11).unwrap(),
            zone_uid: zone_uid(),
            network_uid: network_uid(),
            bundle_generation: bundle_generation(),
            admitted_interface_names: vec![IfName::new("enp0s1").unwrap()],
            tracing_span_id: None,
        })
        .expect("request serializes"),
    )
    .expect("the handler serves the typed request");

    let captured = server.captured();
    let d2b_contracts_broker::broker_wire::BrokerRequest::EnvelopeInvoke(invoke) =
        captured.request
    else {
        panic!("the kernel leg must carry an EnvelopeInvoke frame");
    };
    assert_eq!(invoke.operation, "create-tap-fd");
    assert_eq!(
        invoke.payload.get("vmId").and_then(serde_json::Value::as_str),
        Some("work-vm")
    );
    assert_eq!(result.fds().len(), 1, "the tap descriptor returns over the fd leg");
    let value = |key: &str| {
        result
            .object()
            .get(key)
            .map(serde_json::to_value)
            .transpose()
            .ok()
            .flatten()
    };
    assert_eq!(
        value("fdIndex").and_then(|value| value.as_u64()),
        Some(0)
    );
    assert_eq!(
        value("tap")
            .and_then(|value| value.as_str().map(str::to_owned)),
        Some("tap-work".to_owned())
    );
    drop(write_end);
}

#[test]
fn a_kernel_refusal_is_mapped_with_its_code_preserved() {
    let root = tempfile::tempdir().expect("temp dir");
    let mut server = KernelServer::spawn(&root);
    server.answer(BrokerResponse::EnvelopeInvoke(EnvelopeInvokeResponse {
        operation: "seed-dnsmasq-lease".to_owned(),
        invocation_id: "invocation-kernel".to_owned(),
        result: None,
        refusal: Some("handler-refused".to_owned()),
        detail: Some("network-admission-mismatch".to_owned()),
        fd_indexes: Vec::new(),
        fd_kinds: Vec::new(),
    }));

    let error = invoke(
        handler_for("Operation/seed-dnsmasq-lease"),
        Some(server.kernel()),
        serde_json::to_value(seed_dnsmasq_request()).expect("request serializes"),
    )
    .expect_err("the kernel refusal travels back");

    assert_eq!(error.code(), "handler-refused");
    let detail = error.detail().expect("refusal detail");
    assert!(
        detail.contains("network-admission-mismatch"),
        "the kernel's own detail is preserved: {detail}"
    );
    server.captured();
}

#[test]
fn a_malformed_typed_payload_is_refused() {
    let error = invoke(
        handler_for("Operation/apply-nftables"),
        None,
        serde_json::json!({ "bogus": true }),
    )
    .expect_err("a malformed typed request is refused");
    assert_eq!(error.code(), "handler-refused");
    assert!(
        error
            .detail()
            .expect("detail")
            .contains("invalid typed request"),
        "the parse failure names the typed contract: {}",
        error.detail().expect("detail")
    );
}

#[test]
fn an_unwired_kernel_seam_is_refused() {
    let error = invoke(
        handler_for("Operation/seed-dnsmasq-lease"),
        None,
        serde_json::to_value(seed_dnsmasq_request()).expect("request serializes"),
    )
    .expect_err("a Zone without a kernel seam refuses");
    assert_eq!(error.code(), "kernel-seam-unwired");
}

#[test]
fn an_unknown_trusted_intent_is_refused_before_any_kernel_call() {
    let root = tempfile::tempdir().expect("temp dir");
    let server = KernelServer::spawn(&root);
    let error = invoke(
        handler_for("Operation/apply-nftables"),
        Some(server.kernel()),
        serde_json::to_value(ApplyNftablesRequest {
            bundle_nft_intent_ref: BundleOpId::new("nft:missing"),
            scope_id: ScopeId::new("host"),
            desired_hash: None,
            destroy: false,
            tracing_span_id: None,
        })
        .expect("request serializes"),
    )
    .expect_err("an intent the bundle does not carry is refused");
    assert_eq!(error.code(), "handler-refused");
    assert!(
        error.detail().expect("detail").contains("unknown nft intent"),
        "the refusal names the missing intent: {}",
        error.detail().expect("detail")
    );
    // The refusal happens before the kernel leg: the server never sees a
    // frame, so joining it would hang; drop it instead.
    drop(server);
}