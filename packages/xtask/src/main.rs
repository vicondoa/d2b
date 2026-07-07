use std::{
    env, fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use clap_complete::{
    generate,
    shells::{Bash, Fish, Zsh},
};
use clap_mangen::Man;
use d2b_contracts::guest_wire::GuestControlSchema;
use d2b_contracts::{
    WireProtocolSchema,
    cli_output::{
        AuditOutputV2, AuthStatusOutputV2, HostCheckOutputV2, ListOutputV2, OpInspectOutputV1,
        RealmInspectOutputV1, RealmListOutputV1, ShellDetachOutputV1, ShellKillOutputV1,
        ShellListOutputV1, StatusOutputV2, StoreVerifyOutputV2, UsbProbeOutputV1,
        VmAudioSetOutputV1, VmAudioStatusOutputV1, VmDisplayCloseOutputV1, VmDisplayListOutputV1,
        VmExecCreateOutputV1, VmExecKillOutputV1, VmExecListOutputV1, VmExecLogsOutputV1,
        VmExecStatusOutputV1,
    },
};
use d2b_core::{
    allocator_config::AllocatorJson, audio_policy::AudioPolicyState, bundle::Bundle,
    closures::ClosureMetadata, error::Error, host::HostJson, manifest_v04::ManifestV04,
    minijail_profile::MinijailProfile, privileges::PrivilegesJson, processes::ProcessesJson,
    realm_controller_config::RealmControllersJson, storage::StorageJson,
    storage_lifecycle::StorageLifecycleReport, sync::SyncJson,
};
use d2b_realm_core::{
    AccessBindingRef, AdmissionAuditRecord, AuditEnvelope, Capability, CapabilityNegotiation,
    CapabilityPreflightDenialReason, CapabilityPreflightStatus, CapabilitySet, ConstellationError,
    ConstellationFrame, ControllerGenerationId, CorrelationId, DefaultRealmSelectionMetadata,
    DefaultRealmSelectionSource, DescendantRoute, EnrollmentId, EnrollmentRecord, EnrollmentStatus,
    ExecAttachMode, ExecAttachRequest, ExecCancelRequest, ExecLogsRequest, ExecStartRequest,
    ExecutionGeneration, ExecutionId, ExecutionSummary, GatewayId, Handshake, HandshakeAccepted,
    HandshakeRejected, HandshakeRejectedReason, HostLocalPeerCredentialChecker,
    HostLocalPeerCredentialSemantics, HostLocalPeerCredentialSource, HostLocalProxyStatus,
    IdempotencyKey, KeyFingerprint, KeyPin, LegacySurface, MigrationErrorEnvelope,
    MigrationLegacyId, MigrationReasonCode, NodeId, NodeSummary, OperationId, OperationKind,
    OperationRequest, OperationResponse, PrincipalId, ProviderId, ProviderRegistryEntry,
    RealmAccessAliasBinding, RealmAccessAliasSource, RealmAccessBinding,
    RealmAccessCapabilityPreflight, RealmAccessClientBinding, RealmAccessClientBindingKind,
    RealmAccessClientContract, RealmAccessConflictCandidate, RealmAccessResolverDiagnostic,
    RealmAccessResolverError, RealmAccessResolverRequest, RealmAccessResolverResponse,
    RealmAccessTargetInput, RealmControllerPlacement, RealmId, RealmKeyRole, RealmPath,
    RealmTarget, RealmTransportBinding, RealmTreeEdge, RevocationId, RevocationRecord,
    RevocationStatus, RevocationTarget, RouteAdvertisement, RouteId, RouteSignature, ShellAttachId,
    ShellAttachRequest, ShellAttachSummary, ShellCause, ShellDetachRequest, ShellEventBatch,
    ShellEventSummary, ShellGeneration, ShellKillRequest, ShellListRequest, ShellListResponse,
    ShellName, ShellSessionInstanceId, ShellState, ShellSummary, SignatureRef, StreamCursor,
    StreamId, StreamResume, UnixSocketPath, WorkloadId, WorkloadPlacement,
    WorkloadPlacementSummary, WorkloadSelector, WorkloadSummary,
    allocator::{
        AllocatorConflict, AllocatorEventKind, AllocatorEventMetadata, AllocatorLease,
        AllocatorLeaseState, AllocatorReasonCode, GrantedHostResource, HostResourceKind,
        LeaseAllocationRequest, LeaseAllocationResponse, LeaseAllocationResult, LeaseOwner,
        LeaseResourceRequest, ObservedHostResource, ObservedResourceState, PersistedResourceLease,
        ReconciliationDecision, ReconciliationRecord, ReconciliationReport, ResourceAcquisitionKey,
        ResourceAcquisitionOrder, ResourceDelegation, ResourceObservationSource, ResourceShareMode,
    },
    audit::{
        AuditChainCheckFailure, AuditChainCheckResult, AuditChainLink, AuditChainRecord, AuditHash,
        AuditRetentionFloorReason, AuditRetentionFloorStatus, AuditSinkHealth,
        AuditSinkHealthReason, AuditStreamKind,
    },
};
use schemars::schema::RootSchema;

mod inventory;

const SCHEMA_VERSION: &str = "v2";
const DAEMON_API_DOC: &str = "docs/reference/daemon-api.md";

#[allow(dead_code)]
#[derive(schemars::JsonSchema)]
#[serde(untagged)]
enum D2bRealmCoreSchema {
    RealmId(RealmId),
    RealmPath(RealmPath),
    RealmTarget(RealmTarget),
    NodeId(NodeId),
    WorkloadId(WorkloadId),
    ProviderId(ProviderId),
    GatewayId(GatewayId),
    ExecutionId(ExecutionId),
    StreamId(StreamId),
    StreamCursor(StreamCursor),
    PrincipalId(PrincipalId),
    OperationId(OperationId),
    IdempotencyKey(IdempotencyKey),
    RouteId(RouteId),
    CorrelationId(CorrelationId),
    ControllerGenerationId(ControllerGenerationId),
    AllocatorLeaseId(d2b_realm_core::AllocatorLeaseId),
    HostResourceId(d2b_realm_core::HostResourceId),
    EnrollmentId(EnrollmentId),
    RevocationId(RevocationId),
    Capability(Capability),
    CapabilitySet(CapabilitySet),
    CapabilityNegotiation(CapabilityNegotiation),
    RealmControllerPlacement(RealmControllerPlacement),
    UnixSocketPath(UnixSocketPath),
    AccessBindingRef(AccessBindingRef),
    RealmTransportBinding(RealmTransportBinding),
    RealmAccessBinding(RealmAccessBinding),
    RealmAccessTargetInput(RealmAccessTargetInput),
    RealmAccessAliasSource(RealmAccessAliasSource),
    DefaultRealmSelectionSource(DefaultRealmSelectionSource),
    DefaultRealmSelectionMetadata(DefaultRealmSelectionMetadata),
    RealmAccessAliasBinding(RealmAccessAliasBinding),
    RealmAccessClientBindingKind(RealmAccessClientBindingKind),
    RealmAccessClientContract(RealmAccessClientContract),
    HostLocalPeerCredentialSource(HostLocalPeerCredentialSource),
    HostLocalPeerCredentialChecker(HostLocalPeerCredentialChecker),
    HostLocalProxyStatus(HostLocalProxyStatus),
    HostLocalPeerCredentialSemantics(HostLocalPeerCredentialSemantics),
    RealmAccessClientBinding(RealmAccessClientBinding),
    CapabilityPreflightStatus(CapabilityPreflightStatus),
    CapabilityPreflightDenialReason(CapabilityPreflightDenialReason),
    RealmAccessCapabilityPreflight(RealmAccessCapabilityPreflight),
    RealmAccessConflictCandidate(RealmAccessConflictCandidate),
    RealmAccessResolverDiagnostic(RealmAccessResolverDiagnostic),
    RealmAccessResolverError(RealmAccessResolverError),
    RealmAccessResolverRequest(RealmAccessResolverRequest),
    RealmAccessResolverResponse(RealmAccessResolverResponse),
    ProviderRegistryEntry(ProviderRegistryEntry),
    WorkloadPlacement(WorkloadPlacement),
    WorkloadPlacementSummary(WorkloadPlacementSummary),
    KeyFingerprint(KeyFingerprint),
    RealmKeyRole(RealmKeyRole),
    KeyPin(KeyPin),
    EnrollmentStatus(EnrollmentStatus),
    EnrollmentRecord(EnrollmentRecord),
    RevocationTarget(RevocationTarget),
    RevocationStatus(RevocationStatus),
    RevocationRecord(RevocationRecord),
    SignatureRef(SignatureRef),
    RealmTreeEdge(RealmTreeEdge),
    DescendantRoute(DescendantRoute),
    RouteSignature(RouteSignature),
    RouteAdvertisement(RouteAdvertisement),
    LegacySurface(LegacySurface),
    MigrationLegacyId(MigrationLegacyId),
    MigrationReasonCode(MigrationReasonCode),
    MigrationErrorEnvelope(MigrationErrorEnvelope),
    NodeSummary(NodeSummary),
    WorkloadSelector(WorkloadSelector),
    WorkloadSummary(WorkloadSummary),
    ExecutionGeneration(ExecutionGeneration),
    ExecAttachMode(ExecAttachMode),
    ExecStartRequest(ExecStartRequest),
    ExecAttachRequest(ExecAttachRequest),
    ExecLogsRequest(ExecLogsRequest),
    ExecCancelRequest(ExecCancelRequest),
    ExecutionSummary(ExecutionSummary),
    ShellName(ShellName),
    ShellAttachId(ShellAttachId),
    ShellSessionInstanceId(ShellSessionInstanceId),
    ShellGeneration(ShellGeneration),
    ShellState(ShellState),
    ShellCause(ShellCause),
    ShellListRequest(ShellListRequest),
    ShellAttachRequest(ShellAttachRequest),
    ShellDetachRequest(ShellDetachRequest),
    ShellKillRequest(ShellKillRequest),
    ShellSummary(ShellSummary),
    ShellListResponse(ShellListResponse),
    ShellAttachSummary(ShellAttachSummary),
    ShellEventSummary(ShellEventSummary),
    ShellEventBatch(ShellEventBatch),
    Handshake(Handshake),
    HandshakeAccepted(HandshakeAccepted),
    HandshakeRejected(HandshakeRejected),
    HandshakeRejectedReason(HandshakeRejectedReason),
    OperationKind(OperationKind),
    OperationRequest(OperationRequest),
    OperationResponse(OperationResponse),
    StreamResume(StreamResume),
    AdmissionAuditRecord(AdmissionAuditRecord),
    AuditEnvelope(AuditEnvelope),
    AuditHash(AuditHash),
    AuditStreamKind(AuditStreamKind),
    AuditChainLink(AuditChainLink),
    AuditChainRecord(AuditChainRecord),
    AuditChainCheckFailure(AuditChainCheckFailure),
    AuditChainCheckResult(AuditChainCheckResult),
    AuditRetentionFloorReason(AuditRetentionFloorReason),
    AuditRetentionFloorStatus(AuditRetentionFloorStatus),
    AuditSinkHealthReason(AuditSinkHealthReason),
    AuditSinkHealth(AuditSinkHealth),
    HostResourceKind(HostResourceKind),
    LeaseOwner(LeaseOwner),
    ResourceShareMode(ResourceShareMode),
    ResourceAcquisitionOrder(ResourceAcquisitionOrder),
    LeaseResourceRequest(LeaseResourceRequest),
    ResourceAcquisitionKey(ResourceAcquisitionKey),
    ResourceDelegation(ResourceDelegation),
    GrantedHostResource(GrantedHostResource),
    AllocatorLeaseState(AllocatorLeaseState),
    AllocatorLease(AllocatorLease),
    AllocatorReasonCode(AllocatorReasonCode),
    AllocatorConflict(AllocatorConflict),
    LeaseAllocationResponse(LeaseAllocationResponse),
    LeaseAllocationRequest(LeaseAllocationRequest),
    LeaseAllocationResult(LeaseAllocationResult),
    AllocatorEventKind(AllocatorEventKind),
    AllocatorEventMetadata(AllocatorEventMetadata),
    PersistedResourceLease(PersistedResourceLease),
    ResourceObservationSource(ResourceObservationSource),
    ObservedResourceState(ObservedResourceState),
    ObservedHostResource(ObservedHostResource),
    ReconciliationDecision(ReconciliationDecision),
    ReconciliationRecord(ReconciliationRecord),
    ReconciliationReport(ReconciliationReport),
    TypedError(ConstellationError),
    Frame(ConstellationFrame),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ItemKind {
    Struct,
    Enum,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Field {
    name: String,
    ty: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Variant {
    name: String,
    shape: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RustItem {
    name: String,
    kind: ItemKind,
    file_rel: String,
    line: usize,
    fields: Vec<Field>,
    variants: Vec<Variant>,
}

fn main() -> std::process::ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.as_slice() {
        [command] if command == "gen-schemas" => run_task("gen-schemas", gen_schemas),
        [command] if command == "gen-cli-schemas" => run_task("gen-cli-schemas", gen_cli_schemas),
        [command] if command == "gen-error-codes" => run_task("gen-error-codes", gen_error_codes),
        [command] if command == "gen-cli-shell-artifacts" => {
            run_task("gen-cli-shell-artifacts", gen_cli_shell_artifacts)
        }
        [command] if command == "gen-guest-proto" => run_task("gen-guest-proto", gen_guest_proto),
        [command] if command == "gen-guest-ttrpc" => run_task("gen-guest-ttrpc", gen_guest_ttrpc),
        [command] if command == "gen-daemon-api" => {
            run_task("gen-daemon-api", || gen_daemon_api().map(|p| vec![p]))
        }
        [command, version] if command == "release-notes" => run_task("release-notes", move || {
            gen_release_notes(version).map(|p| vec![p])
        }),
        [command] if command == "adr0035-inventory" => run_inventory(None),
        [command, flag, output]
            if command == "adr0035-inventory" && (flag == "--output" || flag == "-o") =>
        {
            run_inventory(Some(PathBuf::from(output.as_str())))
        }
        _ => {
            eprintln!(
                "usage: cargo xtask <gen-schemas|gen-cli-schemas|gen-error-codes|gen-cli-shell-artifacts|gen-guest-proto|gen-guest-ttrpc|gen-daemon-api|release-notes <version>|adr0035-inventory [--output <path>]>"
            );
            std::process::ExitCode::FAILURE
        }
    }
}

fn run_inventory(output_path: Option<PathBuf>) -> std::process::ExitCode {
    match inventory::emit_adr0035_inventory(output_path.as_deref()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("adr0035-inventory failed: {err}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn gen_guest_ttrpc() -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let repo_root = repo_root()?;
    let proto_dir = repo_root.join("packages/d2b-contracts/proto");
    let proto = proto_dir.join("guest_control.proto");
    let out_dir = repo_root.join("packages/d2b-guestd/src/generated");
    fs::create_dir_all(&out_dir)?;

    ttrpc_codegen::Codegen::new()
        .out_dir(&out_dir)
        .input(&proto)
        .include(&proto_dir)
        .customize(ttrpc_codegen::Customize {
            async_server: true,
            ..Default::default()
        })
        .run()?;

    let out_file = out_dir.join("guest_control_ttrpc.rs");
    sanitize_generated_rust(&out_file)?;
    Ok(vec![out_file])
}

fn gen_guest_proto() -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let repo_root = repo_root()?;
    let proto_dir = repo_root.join("packages/d2b-contracts/proto");
    let proto = proto_dir.join("guest_control.proto");
    let out_dir = repo_root.join("packages/d2b-contracts/src/generated");
    fs::create_dir_all(&out_dir)?;
    let out_file = out_dir.join("guest_control.rs");
    let temp_proto_dir = create_exclusive_temp_dir("d2b-guest-proto")?;
    let temp_proto = temp_proto_dir.join("guest_control.proto");
    fs::write(
        &temp_proto,
        message_only_proto(&fs::read_to_string(&proto)?)?,
    )?;

    protobuf_codegen::Codegen::new()
        .pure()
        .include(&temp_proto_dir)
        .input(&temp_proto)
        .out_dir(&out_dir)
        .run()?;

    sanitize_generated_rust(&out_file)?;
    let _ = fs::remove_dir_all(&temp_proto_dir);
    Ok(vec![out_file])
}

fn message_only_proto(proto: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut out = String::new();
    let mut skipping_service = false;
    let mut depth = 0_i32;
    for line in proto.lines() {
        let trimmed = line.trim_start();
        if !skipping_service && trimmed.starts_with("service GuestControl ") {
            skipping_service = true;
        }
        if skipping_service {
            depth += line.matches('{').count() as i32;
            depth -= line.matches('}').count() as i32;
            if depth <= 0 {
                skipping_service = false;
                depth = 0;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if skipping_service || depth != 0 {
        Err("guest_control.proto service block was not closed".into())
    } else {
        Ok(out)
    }
}

fn sanitize_generated_rust(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut generated = fs::read_to_string(path)?;
    generated = generated.replace("#![allow(unsafe_code)]\n", "");
    generated = generated.replace("#![allow(unknown_lints)]\n", "");
    generated = generated.replace("#![allow(clippy::all)]\n", "");
    generated = generated.replace("#![allow(clipto_camel_casepy)]\n", "");
    generated = generated.replace(
        "#![cfg_attr(rustfmt, rustfmt_skip)]\n",
        "#![cfg_attr(rustfmt, rustfmt::skip)]\n",
    );
    generated = generated.replace(
        "// https://github.com/rust-lang/rust-clippy/issues/702\n\n",
        "#![allow(clippy::bool_comparison)]\n#![allow(clippy::derivable_impls)]\n#![allow(clippy::match_like_matches_macro)]\n#![allow(clippy::match_ref_pats)]\n#![allow(clippy::needless_borrow)]\n#![allow(clippy::redundant_static_lifetimes)]\n#![allow(clippy::vec_init_then_push)]\n\n",
    );
    fs::write(path, generated)?;
    Ok(())
}

fn create_exclusive_temp_dir(prefix: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let base = env::temp_dir();
    for attempt in 0..100_u32 {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = base.join(format!("{prefix}-{}-{nonce}-{attempt}", std::process::id()));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(Box::new(error)),
        }
    }
    Err(format!("could not create exclusive temp dir for {prefix}").into())
}

fn run_task<F>(label: &str, task: F) -> std::process::ExitCode
where
    F: FnOnce() -> Result<Vec<PathBuf>, Box<dyn std::error::Error>>,
{
    match task() {
        Ok(files) => {
            println!("{} generated {} file(s)", label, files.len());
            std::process::ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{} failed: {err}", label);
            std::process::ExitCode::FAILURE
        }
    }
}

fn repo_root() -> Result<&'static Path, Box<dyn std::error::Error>> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .ok_or_else(|| "cannot locate repo root".into())
}

fn gen_schemas() -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let repo_root = repo_root()?;
    let out_dir = repo_root
        .join("docs/reference/schemas")
        .join(SCHEMA_VERSION);
    fs::create_dir_all(&out_dir)?;

    let schemas: [(&str, RootSchema); 16] = [
        ("allocator.json", schemars::schema_for!(AllocatorJson)),
        ("bundle.json", schemars::schema_for!(Bundle)),
        (
            "d2b-realm-core.json",
            schemars::schema_for!(D2bRealmCoreSchema),
        ),
        ("host.json", schemars::schema_for!(HostJson)),
        ("processes.json", schemars::schema_for!(ProcessesJson)),
        ("storage.json", schemars::schema_for!(StorageJson)),
        ("sync.json", schemars::schema_for!(SyncJson)),
        (
            "realm-controllers.json",
            schemars::schema_for!(RealmControllersJson),
        ),
        (
            "storage-lifecycle-report.json",
            schemars::schema_for!(StorageLifecycleReport),
        ),
        ("privileges.json", schemars::schema_for!(PrivilegesJson)),
        ("closures.json", schemars::schema_for!(ClosureMetadata)),
        (
            "minijail-profile.json",
            schemars::schema_for!(MinijailProfile),
        ),
        (
            "wire-protocol.json",
            schemars::schema_for!(WireProtocolSchema),
        ),
        (
            "guest-control.json",
            schemars::schema_for!(GuestControlSchema),
        ),
        ("manifest_v04.json", schemars::schema_for!(ManifestV04)),
        ("audio-state.json", schemars::schema_for!(AudioPolicyState)),
    ];

    write_schemas(&out_dir, &schemas)
}

fn gen_cli_schemas() -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let repo_root = repo_root()?;
    let out_dir = repo_root.join("docs/reference/cli-output");
    fs::create_dir_all(&out_dir)?;

    let schemas: [(&str, RootSchema); 22] = [
        ("list.schema.json", schemars::schema_for!(ListOutputV2)),
        ("status.schema.json", schemars::schema_for!(StatusOutputV2)),
        (
            "usb-probe.schema.json",
            schemars::schema_for!(UsbProbeOutputV1),
        ),
        (
            "op-inspect.schema.json",
            schemars::schema_for!(OpInspectOutputV1),
        ),
        (
            "realm-list.schema.json",
            schemars::schema_for!(RealmListOutputV1),
        ),
        (
            "realm-inspect.schema.json",
            schemars::schema_for!(RealmInspectOutputV1),
        ),
        (
            "vm-display-list.schema.json",
            schemars::schema_for!(VmDisplayListOutputV1),
        ),
        (
            "vm-display-close.schema.json",
            schemars::schema_for!(VmDisplayCloseOutputV1),
        ),
        (
            "vm-exec-create.schema.json",
            schemars::schema_for!(VmExecCreateOutputV1),
        ),
        (
            "vm-exec-list.schema.json",
            schemars::schema_for!(VmExecListOutputV1),
        ),
        (
            "vm-exec-status.schema.json",
            schemars::schema_for!(VmExecStatusOutputV1),
        ),
        (
            "vm-exec-logs.schema.json",
            schemars::schema_for!(VmExecLogsOutputV1),
        ),
        (
            "vm-exec-kill.schema.json",
            schemars::schema_for!(VmExecKillOutputV1),
        ),
        ("audit.schema.json", schemars::schema_for!(AuditOutputV2)),
        (
            "shell-list.schema.json",
            schemars::schema_for!(ShellListOutputV1),
        ),
        (
            "shell-detach.schema.json",
            schemars::schema_for!(ShellDetachOutputV1),
        ),
        (
            "shell-kill.schema.json",
            schemars::schema_for!(ShellKillOutputV1),
        ),
        (
            "host-check.schema.json",
            schemars::schema_for!(HostCheckOutputV2),
        ),
        (
            "auth-status.schema.json",
            schemars::schema_for!(AuthStatusOutputV2),
        ),
        (
            "store-verify.schema.json",
            schemars::schema_for!(StoreVerifyOutputV2),
        ),
        (
            "vm-audio-status.schema.json",
            schemars::schema_for!(VmAudioStatusOutputV1),
        ),
        (
            "vm-audio-set.schema.json",
            schemars::schema_for!(VmAudioSetOutputV1),
        ),
    ];

    write_schemas(&out_dir, &schemas)
}

fn gen_error_codes() -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let repo_root = repo_root()?;
    let out_path = repo_root.join("docs/reference/error-codes.md");
    let doc = fs::read_to_string(&out_path)?;
    let rendered = render_error_code_table();
    let updated = replace_generated_block(&doc, "error-table", &rendered)?;
    fs::write(&out_path, updated)?;
    Ok(vec![out_path])
}

fn render_error_code_table() -> String {
    let mut rendered = String::new();
    rendered.push_str(
        "| docs anchor | kind | exit code | owningCommand | message template | remediation |\n",
    );
    rendered.push_str("| --- | --- | --- | --- | --- | --- |\n");
    for record in Error::all_kinds() {
        let anchor_id = &record.docs_anchor[1..];
        rendered.push_str(&format!(
            "| <a id=\"{anchor_id}\"></a>`{}` | `{}` | `{}` | `{}` | {} | {} |\n",
            record.docs_anchor,
            record.kind.discriminant(),
            record.exit_code,
            record.owning_command,
            markdown_cell(record.message_template),
            markdown_cell(record.remediation),
        ));
    }
    rendered
}

fn markdown_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', "<br>")
}

fn gen_cli_shell_artifacts() -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let repo_root = repo_root()?;
    let man_dir = repo_root.join("docs/manpages");
    let comp_dir = repo_root.join("docs/completions");
    fs::create_dir_all(&man_dir)?;
    fs::create_dir_all(&comp_dir)?;

    let mut man_command = d2b::cli_command();
    man_command.build();
    let source = man_command
        .get_version()
        .map(|version| format!("d2b {version}"))
        .unwrap_or_else(|| "d2b".to_owned());
    let man_path = man_dir.join("d2b.1");
    let mut man_buffer = Vec::new();
    Man::new(man_command)
        .title("d2b")
        .section("1")
        .date("1970-01-01")
        .source(source)
        .manual("d2b CLI")
        .render(&mut man_buffer)?;
    fs::write(&man_path, man_buffer)?;
    let host_man_path = write_subcommand_manpage(&man_dir, &["host"], "d2b-host")?;
    let shell_man_path = write_subcommand_manpage(&man_dir, &["shell"], "d2b-shell")?;
    let clipboard_man_path = write_subcommand_manpage(&man_dir, &["clipboard"], "d2b-clipboard")?;
    let clipboard_arm_man_path =
        write_subcommand_manpage(&man_dir, &["clipboard", "arm"], "d2b-clipboard-arm")?;

    let bash_path = comp_dir.join("d2b.bash");
    let mut bash_command = d2b::cli_command();
    let mut bash_buffer = Vec::new();
    generate(Bash, &mut bash_command, "d2b", &mut bash_buffer);
    let bash_buffer = patch_vm_exec_logs_bash_completion(String::from_utf8(bash_buffer)?)?;
    fs::write(&bash_path, bash_buffer)?;

    let zsh_path = comp_dir.join("d2b.zsh");
    let mut zsh_command = d2b::cli_command();
    let mut zsh_buffer = Vec::new();
    generate(Zsh, &mut zsh_command, "d2b", &mut zsh_buffer);
    fs::write(&zsh_path, zsh_buffer)?;

    let fish_path = comp_dir.join("d2b.fish");
    let mut fish_command = d2b::cli_command();
    let mut fish_buffer = Vec::new();
    generate(Fish, &mut fish_command, "d2b", &mut fish_buffer);
    let fish_buffer = patch_vm_exec_logs_fish_completion(String::from_utf8(fish_buffer)?)?;
    fs::write(&fish_path, fish_buffer)?;

    Ok(vec![
        man_path,
        host_man_path,
        shell_man_path,
        clipboard_man_path,
        clipboard_arm_man_path,
        bash_path,
        zsh_path,
        fish_path,
    ])
}

fn write_subcommand_manpage(
    man_dir: &Path,
    path: &[&str],
    title: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let mut command = d2b::cli_command();
    for component in path {
        command = command
            .find_subcommand_mut(component)
            .unwrap_or_else(|| panic!("{component} subcommand exists"))
            .clone();
    }
    command.build();
    let man_path = man_dir.join(format!("{title}.1"));
    let mut man_buffer = Vec::new();
    Man::new(command)
        .title(title)
        .section("1")
        .date("1970-01-01")
        .source("d2b".to_owned())
        .manual("d2b CLI")
        .render(&mut man_buffer)?;
    fs::write(&man_path, man_buffer)?;
    Ok(man_path)
}

fn patch_vm_exec_logs_bash_completion(
    generated: String,
) -> Result<String, Box<dyn std::error::Error>> {
    let generated = replace_once(
        generated,
        r#"            opts="-d -i -t -h --detach --interactive --tty --env --cwd --json --human --help <VM> [MANAGEMENT]... [COMMAND]..."
"#,
        r#"            opts="-d -i -t -h --detach --interactive --tty --env --cwd --json --human --help <VM> [MANAGEMENT]... [COMMAND]..."
            if [[ " ${COMP_WORDS[*]} " == *" logs "* ]] ; then
                opts="${opts} --stdout-offset --stderr-offset --max-len"
            fi
"#,
        "bash vm exec opts",
    )?;
    replace_once(
        generated,
        r#"                --cwd)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
"#,
        r#"                --cwd)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --stdout-offset|--stderr-offset|--max-len)
                    COMPREPLY=()
                    return 0
                    ;;
"#,
        "bash vm exec logs flag values",
    )
}

fn patch_vm_exec_logs_fish_completion(
    generated: String,
) -> Result<String, Box<dyn std::error::Error>> {
    replace_once(
        generated,
        "complete -c d2b -n \"__fish_d2b_using_subcommand vm; and __fish_seen_subcommand_from exec\" -l cwd -d 'Working directory for the guest command' -r\n",
        "complete -c d2b -n \"__fish_d2b_using_subcommand vm; and __fish_seen_subcommand_from exec\" -l cwd -d 'Working directory for the guest command' -r\ncomplete -c d2b -n \"__fish_d2b_using_subcommand vm; and __fish_seen_subcommand_from exec; and __fish_seen_subcommand_from logs\" -l stdout-offset -d 'Resume stdout from this byte offset. The daemon clamps stale offsets' -r\ncomplete -c d2b -n \"__fish_d2b_using_subcommand vm; and __fish_seen_subcommand_from exec; and __fish_seen_subcommand_from logs\" -l stderr-offset -d 'Resume stderr from this byte offset. The daemon clamps stale offsets' -r\ncomplete -c d2b -n \"__fish_d2b_using_subcommand vm; and __fish_seen_subcommand_from exec; and __fish_seen_subcommand_from logs\" -l max-len -d 'Maximum retained bytes to request per stream' -r\n",
        "fish vm exec logs flags",
    )
}

fn replace_once(
    input: String,
    needle: &str,
    replacement: &str,
    label: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    if !input.contains(needle) {
        return Err(format!("could not patch generated completion: missing {label}").into());
    }
    Ok(input.replacen(needle, replacement, 1))
}

fn write_schemas(
    out_dir: &Path,
    schemas: &[(&str, RootSchema)],
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut written = Vec::with_capacity(schemas.len());
    for (file_name, schema) in schemas {
        let mut schema = schema.clone();
        schema.meta_schema = Some("https://json-schema.org/draft/2020-12/schema".to_owned());
        let path = out_dir.join(file_name);
        let mut data = serde_json::to_string_pretty(&schema)?;
        data.push('\n');
        fs::write(&path, data)?;
        written.push(path);
    }
    Ok(written)
}

fn gen_daemon_api() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let repo_root = repo_root()?;
    let doc_path = repo_root.join(DAEMON_API_DOC);
    let mut doc = fs::read_to_string(&doc_path)?;
    let items = parse_ipc_items(repo_root)?;

    doc = replace_generated_block(&doc, "handshake-types", &render_handshake_section(&items))?;
    doc = replace_generated_block(&doc, "request-types", &render_request_section(&items))?;
    doc = replace_generated_block(&doc, "response-types", &render_response_section(&items))?;
    doc = replace_generated_block(&doc, "enum-variants", &render_enum_section(&items))?;
    doc = replace_generated_block(&doc, "error-envelope", &render_error_section(&items))?;

    fs::write(&doc_path, doc)?;
    Ok(doc_path)
}

fn parse_ipc_items(repo_root: &Path) -> Result<Vec<RustItem>, Box<dyn std::error::Error>> {
    let ipc_dir = repo_root.join("packages/d2b-contracts/src");
    let mut files = fs::read_dir(&ipc_dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("rs"))
        .filter(|path| path.file_name().and_then(|name| name.to_str()) != Some("cli_output.rs"))
        .collect::<Vec<_>>();
    files.sort();

    let mut items = Vec::new();
    for path in files {
        items.extend(parse_rust_items(repo_root, &path)?);
    }
    items.sort_by(|left, right| {
        left.file_rel
            .cmp(&right.file_rel)
            .then(left.line.cmp(&right.line))
            .then(left.name.cmp(&right.name))
    });
    Ok(items)
}

fn parse_rust_items(
    repo_root: &Path,
    path: &Path,
) -> Result<Vec<RustItem>, Box<dyn std::error::Error>> {
    let text = fs::read_to_string(path)?;
    let lines = text.lines().collect::<Vec<_>>();
    let file_rel = path
        .strip_prefix(repo_root)?
        .to_string_lossy()
        .replace('\\', "/");

    let mut items = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let line = lines[index].trim_start();

        // Skip macro_rules! definitions: their bodies contain
        // `pub struct $name(...)` templates whose `$name`
        // placeholder is not a valid Rust identifier and would
        // make extract_name fail. We track brace depth from the
        // `macro_rules! foo {` opening line until the matched
        // closing brace and skip everything in between.
        if line.starts_with("macro_rules!") {
            let mut depth = brace_delta(lines[index]);
            index += 1;
            // If the `{` is on a later line, advance until we see it.
            while depth == 0 && index < lines.len() {
                depth += brace_delta(lines[index]);
                index += 1;
                if depth > 0 {
                    break;
                }
            }
            while depth > 0 && index < lines.len() {
                depth += brace_delta(lines[index]);
                index += 1;
            }
            continue;
        }

        let kind = if line.starts_with("pub struct ") {
            Some(ItemKind::Struct)
        } else if line.starts_with("pub enum ") {
            Some(ItemKind::Enum)
        } else {
            None
        };

        let Some(kind) = kind else {
            index += 1;
            continue;
        };

        let start = index;
        let mut item_lines = vec![lines[index].to_string()];
        let mut depth = brace_delta(lines[index]);
        index += 1;
        while depth > 0 && index < lines.len() {
            item_lines.push(lines[index].to_string());
            depth += brace_delta(lines[index]);
            index += 1;
        }

        let item_text = item_lines.join("\n");
        let body = extract_body(&item_text);
        let name = extract_name(
            item_lines.first().map(String::as_str).unwrap_or_default(),
            &kind,
        )?;
        let fields = if kind == ItemKind::Struct {
            parse_fields(&body)
        } else {
            Vec::new()
        };
        let variants = if kind == ItemKind::Enum {
            parse_variants(&body)
        } else {
            Vec::new()
        };
        items.push(RustItem {
            name,
            kind,
            file_rel: file_rel.clone(),
            line: start + 1,
            fields,
            variants,
        });
    }

    Ok(items)
}

fn brace_delta(line: &str) -> i32 {
    let opens = line.chars().filter(|&ch| ch == '{').count() as i32;
    let closes = line.chars().filter(|&ch| ch == '}').count() as i32;
    opens - closes
}

fn extract_name(header: &str, kind: &ItemKind) -> Result<String, Box<dyn std::error::Error>> {
    let needle = match kind {
        ItemKind::Struct => "pub struct ",
        ItemKind::Enum => "pub enum ",
    };
    let after = header
        .split_once(needle)
        .map(|(_, tail)| tail)
        .ok_or("missing type header")?
        .trim_start();
    let name = after
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
        .collect::<String>();
    if name.is_empty() {
        return Err("could not parse type name".into());
    }
    Ok(name)
}

fn extract_body(item_text: &str) -> String {
    let Some(open) = item_text.find('{') else {
        return String::new();
    };
    let Some(close) = item_text.rfind('}') else {
        return String::new();
    };
    item_text[open + 1..close].to_string()
}

fn parse_fields(body: &str) -> Vec<Field> {
    split_top_level_entries(&strip_non_code_lines(body))
        .into_iter()
        .filter_map(|entry| {
            let trimmed = entry.trim();
            let (name, ty) = trimmed.split_once(':')?;
            Some(Field {
                name: name.trim().trim_start_matches("pub ").trim().to_string(),
                ty: normalize_ws(ty),
            })
        })
        .collect()
}

fn parse_variants(body: &str) -> Vec<Variant> {
    split_top_level_entries(&strip_non_code_lines(body))
        .into_iter()
        .filter_map(|entry| {
            let trimmed = entry.trim();
            if trimmed.is_empty() {
                return None;
            }
            let name = trimmed
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                .collect::<String>();
            if name.is_empty() {
                return None;
            }
            let rest = trimmed[name.len()..].trim();
            let shape = if rest.is_empty() {
                "unit".to_string()
            } else if rest.starts_with('{') {
                let fields = parse_fields(&extract_body(rest));
                if fields.is_empty() {
                    "struct {}".to_string()
                } else {
                    format!("struct {{ {} }}", render_fields(&fields))
                }
            } else {
                normalize_ws(rest)
            };
            Some(Variant { name, shape })
        })
        .collect()
}

fn strip_non_code_lines(body: &str) -> String {
    body.lines()
        .filter(|line| {
            let trimmed = line.trim_start();
            !trimmed.is_empty()
                && !trimmed.starts_with("///")
                && !trimmed.starts_with("//!")
                && !trimmed.starts_with("//")
                && !trimmed.starts_with("#")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn split_top_level_entries(input: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut current = String::new();
    let mut paren = 0i32;
    let mut brace = 0i32;
    let mut bracket = 0i32;
    let mut angle = 0i32;

    for ch in input.chars() {
        match ch {
            '(' => paren += 1,
            ')' => paren -= 1,
            '{' => brace += 1,
            '}' => brace -= 1,
            '[' => bracket += 1,
            ']' => bracket -= 1,
            '<' => angle += 1,
            '>' if angle > 0 => {
                angle -= 1;
            }
            ',' if paren == 0 && brace == 0 && bracket == 0 && angle == 0 => {
                let trimmed = current.trim();
                if !trimmed.is_empty() {
                    entries.push(trimmed.to_string());
                }
                current.clear();
                continue;
            }
            _ => {}
        }
        current.push(ch);
    }

    let trimmed = current.trim();
    if !trimmed.is_empty() {
        entries.push(trimmed.to_string());
    }
    entries
}

fn normalize_ws(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn render_fields(fields: &[Field]) -> String {
    fields
        .iter()
        .map(|field| format!("`{}`: `{}`", field.name, field.ty))
        .collect::<Vec<_>>()
        .join("; ")
}

fn render_shape(item: &RustItem) -> String {
    match item.kind {
        ItemKind::Struct => {
            if item.fields.is_empty() {
                "empty struct".to_string()
            } else {
                format!("struct {{ {} }}", render_fields(&item.fields))
            }
        }
        ItemKind::Enum => {
            if item.variants.is_empty() {
                "empty enum".to_string()
            } else {
                item.variants
                    .iter()
                    .map(|variant| {
                        if variant.shape == "unit" {
                            format!("`{}`", variant.name)
                        } else {
                            format!("`{}` — {}", variant.name, variant.shape)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            }
        }
    }
}

fn rust_link(item: &RustItem) -> String {
    format!("[`{}`](../../{}#L{})", item.name, item.file_rel, item.line)
}

fn replace_generated_block(
    doc: &str,
    marker: &str,
    content: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let begin = format!("<!-- BEGIN AUTO-GENERATED: {marker} -->");
    let end = format!("<!-- END AUTO-GENERATED: {marker} -->");
    let start = doc
        .find(&begin)
        .ok_or_else(|| format!("missing begin marker for {marker}"))?;
    let after_begin = start + begin.len();
    let end_index = doc[after_begin..]
        .find(&end)
        .map(|index| after_begin + index)
        .ok_or_else(|| format!("missing end marker for {marker}"))?;

    let mut rebuilt = String::new();
    rebuilt.push_str(&doc[..after_begin]);
    rebuilt.push('\n');
    rebuilt.push_str(content.trim_end());
    rebuilt.push('\n');
    rebuilt.push_str(&doc[end_index..]);
    Ok(rebuilt)
}

fn render_handshake_section(items: &[RustItem]) -> String {
    let mut selected = items
        .iter()
        .filter(|item| {
            item.name.starts_with("Hello")
                || item.name == "SemverRange"
                || item.name.contains("FeatureFlag")
                || item.name.contains("Capability")
        })
        .collect::<Vec<_>>();
    selected.sort_by(|left, right| left.name.cmp(&right.name));
    render_item_table("Handshake and negotiation types", &selected)
}

fn render_request_section(items: &[RustItem]) -> String {
    let public = items
        .iter()
        .filter(|item| {
            item.file_rel.ends_with("public_wire.rs")
                && (item.name.ends_with("Request")
                    || item.name.ends_with("Command")
                    || item.name == "Hello")
        })
        .collect::<Vec<_>>();
    let broker = items
        .iter()
        .filter(|item| {
            item.file_rel.ends_with("broker_wire.rs")
                && (item.name.ends_with("Request") || item.name.ends_with("Command"))
        })
        .collect::<Vec<_>>();
    render_grouped_tables(
        &[
            ("Public socket request types", public),
            ("Broker socket request types", broker),
        ],
        "No request types were found under `packages/d2b-contracts/src/` yet.",
    )
}

fn render_response_section(items: &[RustItem]) -> String {
    let public = items
        .iter()
        .filter(|item| {
            item.file_rel.ends_with("public_wire.rs")
                && (item.name.ends_with("Response")
                    || item.name.ends_with("Ok")
                    || item.name.ends_with("Rejected"))
        })
        .collect::<Vec<_>>();
    let broker = items
        .iter()
        .filter(|item| item.file_rel.ends_with("broker_wire.rs") && item.name.ends_with("Response"))
        .collect::<Vec<_>>();
    render_grouped_tables(
        &[
            ("Public socket response types", public),
            ("Broker socket response types", broker),
        ],
        "No response types were found under `packages/d2b-contracts/src/` yet.",
    )
}

fn render_enum_section(items: &[RustItem]) -> String {
    let lifecycle = items
        .iter()
        .find(|item| is_lifecycle_enum(item))
        .map(|item| vec![item])
        .unwrap_or_default();
    let other = items
        .iter()
        .filter(|item| {
            item.kind == ItemKind::Enum
                && !is_lifecycle_enum(item)
                && !item.name.starts_with("Hello")
                && !item.name.ends_with("Request")
                && !item.name.ends_with("Response")
                && !item.name.ends_with("Ok")
                && !item.name.ends_with("Rejected")
                && !is_error_item(item)
        })
        .collect::<Vec<_>>();
    render_grouped_tables(
        &[
            ("Lifecycle enum", lifecycle),
            ("Other documented enums", other),
        ],
        "No documented enums were found under `packages/d2b-contracts/src/` yet.",
    )
}

fn render_error_section(items: &[RustItem]) -> String {
    let selected = items
        .iter()
        .filter(|item| is_error_item(item))
        .collect::<Vec<_>>();
    render_item_table("Typed error envelope types", &selected)
}

fn is_lifecycle_enum(item: &RustItem) -> bool {
    item.kind == ItemKind::Enum
        && [
            "Stopped",
            "Starting",
            "Booted",
            "Running",
            "Stopping",
            "Restarting",
            "Failed",
            "Unknown",
        ]
        .iter()
        .all(|name| item.variants.iter().any(|variant| variant.name == *name))
}

fn is_error_item(item: &RustItem) -> bool {
    let lower = item.name.to_ascii_lowercase();
    if lower.contains("error") {
        return true;
    }
    item.fields.iter().any(|field| field.name == "kind")
        && item.fields.iter().any(|field| field.name == "code")
        && item.fields.iter().any(|field| field.name == "message")
}

fn render_grouped_tables(groups: &[(&str, Vec<&RustItem>)], empty_message: &str) -> String {
    let mut rendered = String::new();
    let mut any = false;
    for (title, items) in groups {
        if items.is_empty() {
            continue;
        }
        any = true;
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        rendered.push_str(&render_item_table(title, items));
    }
    if any {
        rendered
    } else {
        format!("> {empty_message}")
    }
}

fn render_item_table(title: &str, items: &[&RustItem]) -> String {
    if items.is_empty() {
        return "> No matching IPC types were found yet.".to_string();
    }

    let mut rendered = String::new();
    rendered.push_str(&format!("### {title}\n\n"));
    rendered.push_str("| Type | Kind | Rust definition | Shape |\n");
    rendered.push_str("| --- | --- | --- | --- |\n");
    for item in items {
        let kind = match item.kind {
            ItemKind::Struct => "struct",
            ItemKind::Enum => "enum",
        };
        rendered.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            item.name,
            kind,
            rust_link(item),
            render_shape(item)
        ));
    }
    rendered
}

/// Aggregate the `Unreleased` section of CHANGELOG.md
/// into a versioned section. Re-runs are idempotent — if a section
/// for `version` already exists, the function exits with a clear
/// error rather than duplicating it.
///
/// Output: writes CHANGELOG.md in place; returns the path so the
/// caller can announce the artifact.
fn gen_release_notes(version: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    use std::io::Write;

    let repo_root = repo_root()?;
    let changelog_path = repo_root.join("CHANGELOG.md");
    let original =
        fs::read_to_string(&changelog_path).map_err(|e| format!("read CHANGELOG.md: {e}"))?;

    let versioned_header = format!("## [{version}]");
    if original.contains(&versioned_header) {
        return Err(format!(
            "CHANGELOG.md already has a section for {version}: refusing to duplicate"
        )
        .into());
    }

    let unreleased_header = "## Unreleased";
    let unreleased_idx = original
        .find(unreleased_header)
        .ok_or_else(|| "CHANGELOG.md is missing the '## Unreleased' section".to_string())?;
    let after_unreleased_header = unreleased_idx + unreleased_header.len();

    let body_search_start = after_unreleased_header;
    let next_section_offset = original[body_search_start..]
        .find("\n## ")
        .map(|i| body_search_start + i + 1)
        .unwrap_or(original.len());

    let unreleased_body = original[after_unreleased_header..next_section_offset].trim();
    if unreleased_body.is_empty() {
        return Err("CHANGELOG.md '## Unreleased' section is empty; nothing to release".into());
    }

    let date = today_utc_iso8601();
    let mut rendered = String::new();
    rendered.push_str(&original[..unreleased_idx]);
    rendered.push_str(unreleased_header);
    rendered.push_str("\n\n");
    rendered.push_str(&format!("## [{version}] - {date}\n\n"));
    rendered.push_str(unreleased_body);
    rendered.push_str("\n\n");
    if next_section_offset < original.len() {
        rendered.push_str(&original[next_section_offset..]);
    }

    let mut out =
        fs::File::create(&changelog_path).map_err(|e| format!("write CHANGELOG.md: {e}"))?;
    out.write_all(rendered.as_bytes())
        .map_err(|e| format!("write CHANGELOG.md body: {e}"))?;

    Ok(changelog_path)
}

fn today_utc_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let (y, m, d) = civil_from_days(days);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i32 + era as i32 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
