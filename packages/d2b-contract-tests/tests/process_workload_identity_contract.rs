//! Contract and schema-drift guards for process DAG and broker SpawnRunner
//! workload identity integration introduced in Wave 16.
//!
//! Coverage:
//!  * `workloadIdentity` appears in `VmProcessDag` schema (`processes.json`)
//!    as an additive, non-required property.
//!  * `workloadIdentity` appears in `SpawnRunnerRequest` schema
//!    (`wire-protocol.json`) as an additive, non-required property.
//!  * `WorkloadIdentity` definition is present in `processes.json`.
//!  * The `VmProcessDag.workloadIdentity` schema points at `WorkloadIdentity`.
//!  * `SpawnRunnerRequest.workloadIdentity` schema points at `WorkloadIdentity`.
//!  * The `processes.json` `VmProcessDag` still has `additionalProperties: false`
//!    (deny_unknown_fields invariant preserved).
//!  * The `wire-protocol.json` `SpawnRunnerRequest` still has
//!    `additionalProperties: false`.
//!  * Source-lint: `VmProcessDag` and `SpawnRunnerRequest` both carry the
//!    `workload_identity` field with additive annotations (default + skip_serializing_if).
//!  * Nix emitter invariants: `processes-json.nix` uses `lib.splitString` for
//!    `realmPath`, uses `lib.optionalAttrs` to gate the identity block, and does
//!    NOT emit `kind =` inside the identity attrset.
//!  * Backward-compat wire test: a `SpawnRunnerRequest` JSON without
//!    `workloadIdentity` deserializes successfully (additive invariant).
//!  * Backward-compat process DAG test: a `VmProcessDag` JSON without
//!    `workloadIdentity` deserializes successfully.
//!  * Runtime propagation: `VmStartRunner` carries `workload_identity` as a
//!    struct field and `spawn_runner` uses `self.workload_identity` (not a
//!    hardcoded `None`) so the DAG identity reaches every `SpawnRunner` request.
//!  * Per-env usbipd invariant: `BrokerPerEnvUsbipdSpawner` uses
//!    `workload_identity: None` because `sys-<env>-usbipd` runners are
//!    framework infrastructure, not realm workloads.

use d2b_contract_tests::read_repo_file;
use d2b_core::processes::{VmProcessDag, VmProcessInvariants};
use serde_json::Value;

// ── schema helpers ─────────────────────────────────────────────────────────

fn processes_schema() -> Value {
    serde_json::from_str(&read_repo_file("docs/reference/schemas/v2/processes.json"))
        .expect("processes.json parses as JSON")
}

fn wire_protocol_schema() -> Value {
    serde_json::from_str(&read_repo_file(
        "docs/reference/schemas/v2/wire-protocol.json",
    ))
    .expect("wire-protocol.json parses as JSON")
}

fn definition<'a>(schema: &'a Value, name: &str) -> &'a Value {
    schema
        .get("definitions")
        .and_then(Value::as_object)
        .and_then(|defs| defs.get(name))
        .unwrap_or_else(|| panic!("schema is missing definition {name}"))
}

fn required_fields(def: &Value) -> Vec<String> {
    def.get("required")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn properties_keys(def: &Value) -> Vec<String> {
    def.get("properties")
        .and_then(Value::as_object)
        .map(|props| props.keys().cloned().collect())
        .unwrap_or_default()
}

// ── processes.json: VmProcessDag.workloadIdentity ─────────────────────────

/// `WorkloadIdentity` must be present in `processes.json` definitions so
/// `VmProcessDag.workloadIdentity` can reference it.
#[test]
fn workload_identity_definition_in_processes_schema() {
    let schema = processes_schema();
    definition(&schema, "WorkloadIdentity");
}

/// `VmProcessDag` must have `workloadIdentity` as a property.
#[test]
fn vm_process_dag_has_workload_identity_property() {
    let schema = processes_schema();
    let def = definition(&schema, "VmProcessDag");
    let props = properties_keys(def);
    assert!(
        props.contains(&"workloadIdentity".to_owned()),
        "VmProcessDag.properties must contain 'workloadIdentity'"
    );
}

/// `workloadIdentity` must NOT appear in `VmProcessDag.required[]` — it is an
/// additive field and old bundles omit it.
#[test]
fn vm_process_dag_workload_identity_is_not_required() {
    let schema = processes_schema();
    let def = definition(&schema, "VmProcessDag");
    let required = required_fields(def);
    assert!(
        !required.contains(&"workloadIdentity".to_owned()),
        "workloadIdentity is additive; it MUST NOT appear in VmProcessDag required[]"
    );
}

/// `VmProcessDag` must have `additionalProperties: false` — the
/// `deny_unknown_fields` invariant must be preserved after adding the new field.
#[test]
fn vm_process_dag_preserves_deny_unknown_fields() {
    let schema = processes_schema();
    let def = definition(&schema, "VmProcessDag");
    assert_eq!(
        def.get("additionalProperties"),
        Some(&Value::Bool(false)),
        "VmProcessDag must set additionalProperties: false after adding workloadIdentity"
    );
}

/// The `workloadIdentity` property on `VmProcessDag` must reference
/// `WorkloadIdentity` via `$ref`.
#[test]
fn vm_process_dag_workload_identity_refs_workload_identity() {
    let schema = processes_schema();
    let def = definition(&schema, "VmProcessDag");
    let prop = def
        .get("properties")
        .and_then(Value::as_object)
        .and_then(|props| props.get("workloadIdentity"))
        .expect("VmProcessDag.workloadIdentity property must exist");

    // The field is Option<WorkloadIdentity>: schema uses anyOf with $ref + null.
    let any_of = prop
        .get("anyOf")
        .and_then(Value::as_array)
        .expect("VmProcessDag.workloadIdentity must use anyOf (Option<T>)");

    let has_ref = any_of.iter().any(|entry| {
        entry
            .get("$ref")
            .and_then(Value::as_str)
            .is_some_and(|r| r.ends_with("WorkloadIdentity"))
    });
    assert!(
        has_ref,
        "VmProcessDag.workloadIdentity anyOf must include a $ref to WorkloadIdentity"
    );

    let has_null = any_of.iter().any(|entry| {
        entry
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|t| t == "null")
    });
    assert!(
        has_null,
        "VmProcessDag.workloadIdentity anyOf must include a null variant (Option<T>)"
    );
}

// ── wire-protocol.json: SpawnRunnerRequest.workloadIdentity ──────────────

/// `SpawnRunnerRequest` must have `workloadIdentity` as a property.
#[test]
fn spawn_runner_request_has_workload_identity_property() {
    let schema = wire_protocol_schema();
    let def = definition(&schema, "SpawnRunnerRequest");
    let props = properties_keys(def);
    assert!(
        props.contains(&"workloadIdentity".to_owned()),
        "SpawnRunnerRequest.properties must contain 'workloadIdentity'"
    );
}

/// `workloadIdentity` must NOT appear in `SpawnRunnerRequest.required[]` — it
/// is additive so old daemons/brokers do not reject new wire messages.
#[test]
fn spawn_runner_request_workload_identity_is_not_required() {
    let schema = wire_protocol_schema();
    let def = definition(&schema, "SpawnRunnerRequest");
    let required = required_fields(def);
    assert!(
        !required.contains(&"workloadIdentity".to_owned()),
        "workloadIdentity is additive; it MUST NOT appear in SpawnRunnerRequest required[]"
    );
}

/// `SpawnRunnerRequest` must preserve `additionalProperties: false` after the
/// new field is added.
#[test]
fn spawn_runner_request_preserves_deny_unknown_fields() {
    let schema = wire_protocol_schema();
    let def = definition(&schema, "SpawnRunnerRequest");
    assert_eq!(
        def.get("additionalProperties"),
        Some(&Value::Bool(false)),
        "SpawnRunnerRequest must set additionalProperties: false after adding workloadIdentity"
    );
}

/// The `workloadIdentity` property on `SpawnRunnerRequest` must reference
/// `WorkloadIdentity` via `$ref`.
#[test]
fn spawn_runner_request_workload_identity_refs_workload_identity() {
    let schema = wire_protocol_schema();
    let def = definition(&schema, "SpawnRunnerRequest");
    let prop = def
        .get("properties")
        .and_then(Value::as_object)
        .and_then(|props| props.get("workloadIdentity"))
        .expect("SpawnRunnerRequest.workloadIdentity property must exist");

    let any_of = prop
        .get("anyOf")
        .and_then(Value::as_array)
        .expect("SpawnRunnerRequest.workloadIdentity must use anyOf (Option<T>)");

    let has_ref = any_of.iter().any(|entry| {
        entry
            .get("$ref")
            .and_then(Value::as_str)
            .is_some_and(|r| r.ends_with("WorkloadIdentity"))
    });
    assert!(
        has_ref,
        "SpawnRunnerRequest.workloadIdentity anyOf must include a $ref to WorkloadIdentity"
    );
}

// ── Backward-compat: old JSON without workloadIdentity still parses ────────

/// A minimal `VmProcessDag` JSON that omits `workloadIdentity` must
/// deserialize successfully. This validates the `#[serde(default)]` annotation.
#[test]
fn vm_process_dag_without_workload_identity_deserializes() {
    let json = serde_json::json!({
        "vm": "corp-vm",
        "nodes": [],
        "edges": [],
        "invariants": {
            "swtpmPreStartFlush": true,
            "perVmAuditPipeline": true,
            "usbipGating": true,
            "tpmOwnershipMigrationWithoutRunningVmMutation": true
        }
    });
    let dag: VmProcessDag =
        serde_json::from_value(json).expect("VmProcessDag without workloadIdentity must parse");
    assert_eq!(dag.vm, "corp-vm");
    assert!(
        dag.workload_identity.is_none(),
        "workload_identity must be None when absent from JSON"
    );
}

/// A `VmProcessDag` JSON with `workloadIdentity: null` must also deserialize
/// cleanly (explicit null is equivalent to absent).
#[test]
fn vm_process_dag_with_null_workload_identity_deserializes() {
    let json = serde_json::json!({
        "vm": "corp-vm",
        "workloadIdentity": null,
        "nodes": [],
        "edges": [],
        "invariants": {
            "swtpmPreStartFlush": true,
            "perVmAuditPipeline": true,
            "usbipGating": true,
            "tpmOwnershipMigrationWithoutRunningVmMutation": true
        }
    });
    let dag: VmProcessDag =
        serde_json::from_value(json).expect("VmProcessDag with workloadIdentity: null must parse");
    assert!(
        dag.workload_identity.is_none(),
        "workload_identity must be None for explicit null"
    );
}

/// `VmProcessDag.workloadIdentity = None` must serialize with the field absent
/// (not `null`) — the `skip_serializing_if = "Option::is_none"` invariant.
#[test]
fn vm_process_dag_none_workload_identity_omitted_from_json() {
    let dag = VmProcessDag {
        vm: "corp-vm".to_owned(),
        workload_identity: None,
        nodes: vec![],
        edges: vec![],
        invariants: VmProcessInvariants {
            swtpm_pre_start_flush: true,
            per_vm_audit_pipeline: true,
            usbip_gating: true,
            tpm_ownership_migration_without_running_vm_mutation: true,
        },
    };
    let json_str = serde_json::to_string(&dag).expect("serialize");
    let json: Value = serde_json::from_str(&json_str).unwrap();
    assert!(
        json.get("workloadIdentity").is_none(),
        "workloadIdentity must be absent from JSON when None (skip_serializing_if invariant)"
    );
}

// ── Source-lint: additive annotations on Rust DTOs ───────────────────────────

/// `VmProcessDag.workload_identity` must carry both `#[serde(default)]` and
/// `skip_serializing_if = "Option::is_none"` to satisfy the additive-field
/// contract documented in `workload_identity.rs`.
#[test]
fn vm_process_dag_workload_identity_has_additive_serde_annotations() {
    let source = read_repo_file("packages/d2b-core/src/processes.rs");

    // Find the workload_identity field declaration in VmProcessDag:
    let lines: Vec<&str> = source.lines().collect();
    let field_idx = lines
        .iter()
        .position(|line| {
            line.contains("pub workload_identity") && line.contains("WorkloadIdentity")
        })
        .expect("processes.rs must declare pub workload_identity: Option<WorkloadIdentity>");

    // The 5 lines before the field must include the serde annotations:
    let start = field_idx.saturating_sub(5);
    let window = lines[start..field_idx].join("\n");
    assert!(
        window.contains("serde(default"),
        "workload_identity in VmProcessDag must carry #[serde(default)] for backward-compat \
         deserialization"
    );
    assert!(
        window.contains("skip_serializing_if"),
        "workload_identity in VmProcessDag must carry skip_serializing_if = \"Option::is_none\" \
         so None is absent from JSON"
    );
}

/// `SpawnRunnerRequest.workload_identity` must carry both `#[serde(default)]`
/// and `skip_serializing_if = "Option::is_none"`.
#[test]
fn spawn_runner_request_workload_identity_has_additive_serde_annotations() {
    let source = read_repo_file("packages/d2b-contracts/src/broker_wire.rs");

    let lines: Vec<&str> = source.lines().collect();
    let field_idx = lines
        .iter()
        .position(|line| {
            line.contains("pub workload_identity") && line.contains("WorkloadIdentity")
        })
        .expect(
            "broker_wire.rs must declare pub workload_identity: Option<WorkloadIdentity> in \
             SpawnRunnerRequest",
        );

    let start = field_idx.saturating_sub(5);
    let window = lines[start..field_idx].join("\n");
    assert!(
        window.contains("serde(default"),
        "workload_identity in SpawnRunnerRequest must carry #[serde(default)]"
    );
    assert!(
        window.contains("skip_serializing_if"),
        "workload_identity in SpawnRunnerRequest must carry skip_serializing_if"
    );
}

// ── Nix emitter source-lints ─────────────────────────────────────────────────

/// `processes-json.nix` must use `lib.splitString "." ...realmPath` to convert
/// the dot-separated realm path string into the array that `RealmPath` expects.
#[test]
fn processes_json_nix_uses_split_string_for_realm_path() {
    let emitter = read_repo_file("nixos-modules/processes-json.nix");
    assert!(
        emitter.contains(r#"lib.splitString "." "#) && emitter.contains("realmPath"),
        "processes-json.nix must use `lib.splitString \".\"` to convert realmPath string \
         to the array form required by WorkloadIdentity"
    );
}

/// `processes-json.nix` must gate the `workloadIdentity` block with
/// `lib.optionalAttrs` so VMs without a realm workload declaration do not emit
/// the field at all (satisfying the skip_serializing_if invariant in the
/// Nix→JSON path).
#[test]
fn processes_json_nix_uses_optional_attrs_for_workload_identity() {
    let emitter = read_repo_file("nixos-modules/processes-json.nix");
    assert!(
        emitter.contains("lib.optionalAttrs") && emitter.contains("workloadIdentity"),
        "processes-json.nix must use lib.optionalAttrs to conditionally include \
         workloadIdentity in VmProcessDag"
    );
}

/// `processes-json.nix` must NOT assign `kind = workloadRow.kind` inside any
/// workload identity attrset. `WorkloadIdentity` has no `kind` field; the
/// `kind` attribute belongs to the index row only.
#[test]
fn processes_json_nix_no_kind_in_workload_identity() {
    let emitter = read_repo_file("nixos-modules/processes-json.nix");
    // We check that the identity attrset block does not contain `kind = vmWorkloadRow`
    // which would be a wrong field reference.
    assert!(
        !emitter.contains("kind = vmWorkloadRow"),
        "processes-json.nix must not assign 'kind = vmWorkloadRow' inside the workload \
         identity block; WorkloadIdentity has no 'kind' field"
    );
}

/// `processes-json.nix` must use `runtimeProviderId` as the *Nix value source*
/// but emit it as `providerId` (the DTO field name). Check that the JSON key
/// name `runtimeProviderId =` is absent from the emitter.
#[test]
fn processes_json_nix_emits_provider_id_not_runtime_provider_id() {
    let emitter = read_repo_file("nixos-modules/processes-json.nix");
    assert!(
        !emitter.contains("runtimeProviderId ="),
        "processes-json.nix must not use 'runtimeProviderId =' as a JSON key; \
         the WorkloadIdentity DTO field is 'providerId'"
    );
}

/// `processes-json.nix` must use `vmWorkloadRow.runtimeProviderId` as the value
/// source (accessed from the index row) for the `providerId` JSON key.
#[test]
fn processes_json_nix_reads_runtime_provider_id_from_row() {
    let emitter = read_repo_file("nixos-modules/processes-json.nix");
    assert!(
        emitter.contains("runtimeProviderId"),
        "processes-json.nix must read runtimeProviderId from the workload index row \
         to populate the 'providerId' identity field"
    );
}

/// The separation invariant: `processes-json.nix` must emit `workloadIdentity`
/// blocks for BOTH `vmDag` (Cloud Hypervisor VMs) and `qemuMediaDag` (QEMU
/// media VMs) — the two primary local VM runtime kinds. Both functions appear
/// in the file and both must contain the realm workload identity lookup.
#[test]
fn processes_json_nix_emits_workload_identity_in_both_dag_functions() {
    let emitter = read_repo_file("nixos-modules/processes-json.nix");

    // Count occurrences of the workload identity lookup pattern.
    // Each vmDag/qemuMediaDag defines `vmWorkloadRow` and `vmWorkloadIdentity`.
    let row_count = emitter.matches("vmWorkloadRow").count();
    assert!(
        row_count >= 4,
        "processes-json.nix must declare vmWorkloadRow in both vmDag and qemuMediaDag \
         (expected ≥4 occurrences, found {row_count})"
    );
    let identity_count = emitter.matches("vmWorkloadIdentity").count();
    assert!(
        identity_count >= 4,
        "processes-json.nix must declare vmWorkloadIdentity in both vmDag and qemuMediaDag \
         (expected ≥4 occurrences, found {identity_count})"
    );
}

// ── VmStartRunner runtime propagation source-lints ───────────────────────────

/// `VmStartRunner` must declare a `workload_identity: Option<WorkloadIdentity>`
/// field so the daemon can thread the DAG identity into every `SpawnRunner`
/// broker request.
#[test]
fn vm_start_runner_struct_has_workload_identity_field() {
    let lib = read_repo_file("packages/d2bd/src/lib.rs");

    // Find the struct VmStartRunner block and verify the field is there.
    let struct_start = lib
        .find("struct VmStartRunner")
        .expect("VmStartRunner struct must be present in d2bd/src/lib.rs");
    // The struct body ends at the first `}` after the struct declaration.
    let struct_body_end = lib[struct_start..]
        .find('}')
        .map(|rel| struct_start + rel)
        .expect("VmStartRunner struct must have a closing brace");
    let struct_body = &lib[struct_start..=struct_body_end];

    assert!(
        struct_body.contains("workload_identity"),
        "VmStartRunner struct must have a `workload_identity` field; \
         found body: {struct_body}"
    );
}

/// `VmStartRunner::spawn_runner` must use `self.workload_identity` when
/// building the `SpawnRunnerRequest`, not a hardcoded `None`.  This lint
/// verifies the propagation path is wired.
#[test]
fn vm_start_runner_spawn_runner_uses_self_workload_identity() {
    let lib = read_repo_file("packages/d2bd/src/lib.rs");

    // Find the spawn_runner function body inside VmStartRunner's impl block.
    let fn_start = lib
        .find("fn spawn_runner(")
        .expect("spawn_runner fn must be present in d2bd/src/lib.rs");

    // Look for `SpawnRunner(BrokerSpawnRunnerRequest` within 5000 chars of
    // spawn_runner (the function is ~150 lines; 5000 chars is a safe window).
    let window_end = (fn_start + 5000).min(lib.len());
    let window = &lib[fn_start..window_end];

    // The SpawnRunner request must reference `self.workload_identity`.
    assert!(
        window.contains("self.workload_identity"),
        "VmStartRunner::spawn_runner must use `self.workload_identity` in the \
         SpawnRunnerRequest, not a hardcoded `None`"
    );

    // Belt-and-suspenders: confirm there is no raw `workload_identity: None`
    // in the same function window (hardcoded None would be a regression).
    assert!(
        !window.contains("workload_identity: None"),
        "VmStartRunner::spawn_runner must not hardcode `workload_identity: None`; \
         it must propagate `self.workload_identity`"
    );
}

/// `BrokerPerEnvUsbipdSpawner::spawn` must explicitly use `workload_identity: None`
/// for per-env usbipd runners.  The comment adjacent to that field documents why:
/// these are framework infrastructure services, not realm workloads.
#[test]
fn broker_perenv_usbipd_spawner_workload_identity_is_none_and_documented() {
    let lib = read_repo_file("packages/d2bd/src/lib.rs");

    // Locate BrokerPerEnvUsbipdSpawner struct definition.
    let struct_offset = lib
        .find("struct BrokerPerEnvUsbipdSpawner")
        .expect("BrokerPerEnvUsbipdSpawner must be present in d2bd/src/lib.rs");

    // The spawn fn is after the struct; search from there.
    let spawn_offset = lib[struct_offset..]
        .find("fn spawn(")
        .map(|rel| struct_offset + rel)
        .expect("BrokerPerEnvUsbipdSpawner::spawn fn must be present");

    // 2000 chars covers the full spawn implementation.
    let window_end = (spawn_offset + 2000).min(lib.len());
    let window = &lib[spawn_offset..window_end];

    assert!(
        window.contains("workload_identity: None"),
        "BrokerPerEnvUsbipdSpawner::spawn must use `workload_identity: None` \
         for per-env usbipd runners (they are infrastructure, not realm workloads)"
    );
    // The comment explaining why None is used must be present near the field.
    assert!(
        window.contains("infrastructure") || window.contains("not realm workload"),
        "BrokerPerEnvUsbipdSpawner::spawn must include a comment explaining why \
         workload_identity is None (these are infrastructure services, not realm workloads)"
    );
}
