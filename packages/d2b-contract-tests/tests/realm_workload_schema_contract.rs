//! Contract and schema-drift guards for realm workload DTOs introduced in Wave 15.
//!
//! Coverage:
//!  * `WorkloadIdentity`, `WorkloadTarget` and related types appear in generated schemas.
//!  * Additive-field invariant: `identity` on `RealmControllerLocalWorkload` and
//!    `workloadIdentity` on `ListEntry`/`VmStatus` are NOT in `required[]`.
//!  * Wire-protocol schema separation: workload identity travels in the daemon-wire
//!    schema (`wire-protocol.json`), NOT in CLI output schemas (`list.schema.json`,
//!    `status.schema.json`).
//!  * `realm-workloads-launcher.json` emitter carries the `noSensitiveCommandPayloads`
//!    invariant, `canonicalTarget`, `appCommand`, and `actions` fields, and is
//!    registered as `contractPrivateNonSecret` / `nonSecret`.
//!  * Source-lint: `WorkloadIdentity` and sibling structs carry `deny_unknown_fields`;
//!    the module-level doc policy comment names both `bundleVersion` and `schemaVersion`
//!    as the required bumps for breaking changes.
//!  * `realm-controllers.json` contains no sensitive credential fields.

use d2b_contract_tests::read_repo_file;
use d2b_contracts::provider_registry_v2::{ProviderBindingV2, ProviderRegistryV2};
use d2b_core::{
    bundle::Bundle, realm_workloads_launcher::RealmWorkloadsLauncherV2Json,
    unsafe_local_workloads::UnsafeLocalWorkloadsJson,
};
use serde_json::Value;
use std::{env, fs, path::Path};

fn read_fixture_json<T: serde::de::DeserializeOwned>(dir: &Path, name: &str) -> T {
    let path = dir.join(name);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

// ── schema loaders ────────────────────────────────────────────────────────────

fn realm_controllers_schema() -> Value {
    serde_json::from_str(&read_repo_file(
        "docs/reference/schemas/v2/realm-controllers.json",
    ))
    .expect("realm-controllers schema parses as JSON")
}

fn wire_protocol_schema() -> Value {
    serde_json::from_str(&read_repo_file(
        "docs/reference/schemas/v2/wire-protocol.json",
    ))
    .expect("wire-protocol schema parses as JSON")
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

// ── realm-controllers.json: WorkloadIdentity definition ──────────────────────

/// `WorkloadIdentity` must appear in the generated `realm-controllers.json`
/// schema because `RealmControllerLocalWorkload.identity` references it.
#[test]
fn workload_identity_definition_is_in_realm_controllers_schema() {
    let schema = realm_controllers_schema();
    definition(&schema, "WorkloadIdentity");
}

/// `RealmTarget` (aliased as `WorkloadTarget`) must appear in
/// `realm-controllers.json` because `WorkloadIdentity.canonical_target` uses it.
#[test]
fn realm_target_definition_is_in_realm_controllers_schema() {
    let schema = realm_controllers_schema();
    definition(&schema, "RealmTarget");
}

/// `WorkloadIdentity` required fields must be exactly the non-optional core
/// identity fields. Optional fields (`workloadName`, `legacyVmName`,
/// `runtimeKind`, `providerId`) MUST NOT appear in `required[]`.
#[test]
fn workload_identity_required_fields_match_non_optional_core() {
    let schema = realm_controllers_schema();
    let def = definition(&schema, "WorkloadIdentity");
    let required = required_fields(def);

    // Exactly these four fields are non-optional:
    for field in ["canonicalTarget", "realmId", "realmPath", "workloadId"] {
        assert!(
            required.contains(&field.to_owned()),
            "WorkloadIdentity required[] must include {field}"
        );
    }
    // Optional fields must NOT appear in required[]:
    for optional in ["workloadName", "legacyVmName", "runtimeKind", "providerId"] {
        assert!(
            !required.contains(&optional.to_owned()),
            "WorkloadIdentity required[] must NOT include optional field {optional}"
        );
    }
}

// ── realm-controllers.json: identity is additive on LocalWorkload ─────────────

/// `identity` must be present in `RealmControllerLocalWorkload.properties`
/// (the field is wired) but must NOT appear in `required[]` (it is additive;
/// old Nix emitters omit it and old code must still parse without it).
#[test]
fn workload_identity_is_additive_field_not_required_in_local_workload() {
    let schema = realm_controllers_schema();
    let def = definition(&schema, "RealmControllerLocalWorkload");

    // Must appear as a property:
    let props = def
        .get("properties")
        .and_then(Value::as_object)
        .expect("RealmControllerLocalWorkload must have properties");
    assert!(
        props.contains_key("identity"),
        "RealmControllerLocalWorkload.properties must contain 'identity'"
    );

    // Must NOT be required:
    let required = required_fields(def);
    assert!(
        !required.contains(&"identity".to_owned()),
        "identity is an additive field; it MUST NOT appear in RealmControllerLocalWorkload required[]"
    );

    // The schema closes unknown fields (additionalProperties: false):
    assert_eq!(
        def.get("additionalProperties"),
        Some(&Value::Bool(false)),
        "RealmControllerLocalWorkload must set additionalProperties: false"
    );
}

// ── wire-protocol.json: workloadIdentity on ListEntry and VmStatus ────────────

/// `WorkloadIdentity` must appear in the generated `wire-protocol.json` schema.
#[test]
fn workload_identity_definition_is_in_wire_protocol_schema() {
    let schema = wire_protocol_schema();
    definition(&schema, "WorkloadIdentity");
}

/// `workloadIdentity` must be a property of `ListEntry` in the wire-protocol
/// schema, but MUST NOT appear in `required[]` (additive field — old daemons
/// omit it; new CLI consumers must tolerate its absence).
#[test]
fn workload_identity_is_additive_not_required_in_list_entry() {
    let schema = wire_protocol_schema();
    let def = definition(&schema, "ListEntry");

    let props = def
        .get("properties")
        .and_then(Value::as_object)
        .expect("ListEntry must have properties");
    assert!(
        props.contains_key("workloadIdentity"),
        "ListEntry.properties must contain 'workloadIdentity'"
    );

    let required = required_fields(def);
    assert!(
        !required.contains(&"workloadIdentity".to_owned()),
        "workloadIdentity is additive; it MUST NOT appear in ListEntry required[]"
    );
}

/// `workloadIdentity` must be a property of `VmStatus` in the wire-protocol
/// schema, but MUST NOT appear in `required[]`.
#[test]
fn workload_identity_is_additive_not_required_in_vm_status() {
    let schema = wire_protocol_schema();
    let def = definition(&schema, "VmStatus");

    let props = def
        .get("properties")
        .and_then(Value::as_object)
        .expect("VmStatus must have properties");
    assert!(
        props.contains_key("workloadIdentity"),
        "VmStatus.properties must contain 'workloadIdentity'"
    );

    let required = required_fields(def);
    assert!(
        !required.contains(&"workloadIdentity".to_owned()),
        "workloadIdentity is additive; it MUST NOT appear in VmStatus required[]"
    );
}

// ── Wire / CLI schema separation ──────────────────────────────────────────────

/// `WorkloadIdentity` must NOT appear in the CLI-facing `list.schema.json`.
/// That schema is generated from `ListOutputV2` / `ListItemOutputV2` which
/// intentionally does not carry the daemon-wire identity fields, keeping the
/// public CLI output schema stable regardless of daemon upgrades.
#[test]
fn workload_identity_absent_from_cli_list_output_schema() {
    let list_schema = read_repo_file("docs/reference/cli-output/list.schema.json");
    assert!(
        !list_schema.contains("WorkloadIdentity"),
        "WorkloadIdentity must not appear in list.schema.json; \
         the CLI output schema (ListOutputV2) is intentionally separate from the daemon-wire schema"
    );
}

/// `WorkloadIdentity` must NOT appear in the CLI-facing `status.schema.json`.
#[test]
fn workload_identity_absent_from_cli_status_output_schema() {
    let status_schema = read_repo_file("docs/reference/cli-output/status.schema.json");
    assert!(
        !status_schema.contains("WorkloadIdentity"),
        "WorkloadIdentity must not appear in status.schema.json; \
         the CLI output schema (StatusOutputV2) is intentionally separate from the daemon-wire schema"
    );
}

// ── realm-controllers.json: no sensitive credential fields ───────────────────

/// `realm-controllers.json` must not contain sensitive credential field names.
/// Controller config is a `nonSecret` artifact; any realm credential material
/// must live inside the gateway guest, not in the host-resident bundle.
#[test]
fn realm_controllers_schema_contains_no_sensitive_credential_fields() {
    let schema_text = read_repo_file("docs/reference/schemas/v2/realm-controllers.json");
    for forbidden in [
        "\"privateKey\"",
        "\"credentialMaterial\"",
        "\"providerToken\"",
        "\"relayCredential\"",
        "\"signatureBytes\"",
        "\"sessionToken\"",
    ] {
        assert!(
            !schema_text.contains(forbidden),
            "realm-controllers.json must not expose sensitive field {forbidden}"
        );
    }
}

// ── realm-workloads-launcher.json emitter contract ───────────────────────────

/// The launcher JSON emitter must be imported from `default.nix` and registered
/// as a `contractPrivateNonSecret` / `nonSecret` artifact in
/// `bundle-artifacts.nix`.
#[test]
fn realm_workloads_launcher_artifact_wired_as_private_non_secret() {
    let default_nix = read_repo_file("nixos-modules/default.nix");
    assert!(
        default_nix.contains("./realm-workloads-launcher-json.nix"),
        "default.nix must import realm-workloads-launcher-json.nix"
    );

    let bundle_artifacts = read_repo_file("nixos-modules/bundle-artifacts.nix");
    assert!(
        bundle_artifacts.contains("realmWorkloadsLauncherJson"),
        "bundle-artifacts.nix must declare realmWorkloadsLauncherJson metadata"
    );

    let emitter = read_repo_file("nixos-modules/realm-workloads-launcher-json.nix");
    for marker in [
        "installFileName = \"realm-workloads-launcher.json\";",
        "classification = \"contractPrivateNonSecret\";",
        "sensitivity = \"nonSecret\";",
    ] {
        assert!(
            emitter.contains(marker),
            "realm-workloads-launcher emitter missing contract marker: {marker}"
        );
    }
}

/// The launcher JSON emitter must assert the `noSensitiveCommandPayloads`
/// invariant. Static operator-declared launch commands (`appCommand`,
/// `actions[].command`) are not sensitive payloads; this invariant name
/// encodes that design decision and must not be silently renamed back to the
/// original `noCommandPayloads`.
#[test]
fn realm_workloads_launcher_invariant_is_no_sensitive_command_payloads() {
    let emitter = read_repo_file("nixos-modules/realm-workloads-launcher-json.nix");
    assert!(
        emitter.contains("noSensitiveCommandPayloads = true;"),
        "realm-workloads-launcher emitter must assert noSensitiveCommandPayloads = true"
    );
    assert!(
        !emitter.contains("noCommandPayloads"),
        "noCommandPayloads is the old invariant name; it must be replaced by noSensitiveCommandPayloads"
    );
}

/// The launcher JSON emitter must expose `canonicalTarget`, `appCommand`, and
/// `actions` per workload row. These fields ground the desktop launcher
/// integration contract.
#[test]
fn realm_workloads_launcher_exposes_canonical_target_and_actions() {
    let emitter = read_repo_file("nixos-modules/realm-workloads-launcher-json.nix");
    for field in ["canonicalTarget", "appCommand", "actions"] {
        assert!(
            emitter.contains(field),
            "realm-workloads-launcher emitter must wire {field} per workload row"
        );
    }
}

/// The launcher JSON emitter must expose `workloadId` as an explicit DTO-named
/// field alongside `workloadName`.  This matches the `WorkloadIdentity.workloadId`
/// contract and ensures downstream consumers (Waybar, wlcontrol, clip-picker) can
/// use a stable daemon-wire-compatible identifier without relying on `workloadName`.
#[test]
fn realm_workloads_launcher_exposes_workload_id_field() {
    let emitter = read_repo_file("nixos-modules/realm-workloads-launcher-json.nix");
    assert!(
        emitter.contains("workloadId"),
        "realm-workloads-launcher emitter must expose workloadId per workload row"
    );
    // Both workloadName (backward compat) and workloadId (explicit DTO alias) must be present.
    assert!(
        emitter.contains("workloadName"),
        "realm-workloads-launcher emitter must also retain workloadName for backward compat"
    );
}

/// The launcher JSON emitter must expose `iconId`, `iconName`, and `iconGroupKey`
/// in addition to the existing resolved `icon` field.
///
/// - `iconId`       — raw launcher.icon.id value (null when not set)
/// - `iconName`     — raw launcher.icon.name fallback value (null when not set)
/// - `iconGroupKey` — stable clustering key for duplicate-icon / app-chooser
///   semantics; equals iconId when set, else iconName, else null
///
/// These fields let desktop consumers (Waybar, wlcontrol, clip-picker) cluster
/// workloads that represent the same application type across realms without
/// having to re-derive the group key from display strings.
#[test]
fn realm_workloads_launcher_exposes_icon_grouping_fields() {
    let emitter = read_repo_file("nixos-modules/realm-workloads-launcher-json.nix");
    for field in ["iconId", "iconName", "iconGroupKey"] {
        assert!(
            emitter.contains(field),
            "realm-workloads-launcher emitter must wire {field} per workload row"
        );
    }
    // The backward-compatible resolved `icon` field must still be present.
    assert!(
        emitter.contains("icon = workload.icon"),
        "realm-workloads-launcher emitter must retain backward-compat 'icon' resolved field"
    );
}

/// The launcher JSON emitter must document the `iconGroupKey` field with an
/// explanation of its duplicate-icon / app-chooser purpose so that the
/// design decision is discoverable in the source.
#[test]
fn realm_workloads_launcher_icon_group_key_has_semantic_comment() {
    let emitter = read_repo_file("nixos-modules/realm-workloads-launcher-json.nix");
    // The comment must mention the grouping / app-chooser semantic:
    assert!(
        emitter.contains("iconGroupKey"),
        "realm-workloads-launcher emitter must contain iconGroupKey"
    );
    assert!(
        emitter.contains("app-chooser") || emitter.contains("grouping"),
        "realm-workloads-launcher emitter must document the duplicate-icon / app-chooser semantic \
         for iconGroupKey"
    );
}

/// The index emitter (`index.nix`) must also derive `workloadId`, `iconId`,
/// `iconName`, and `iconGroupKey` on the workload row so that any consumer of
/// the index (not just the launcher emitter) has access to these fields.
#[test]
fn realm_workloads_index_derives_grouping_fields() {
    let index = read_repo_file("nixos-modules/index.nix");
    for field in ["workloadId", "iconId", "iconName", "iconGroupKey"] {
        assert!(
            index.contains(field),
            "nixos-modules/index.nix must derive {field} on the workload row"
        );
    }
}

#[test]
fn provider_neutral_launcher_and_unsafe_local_artifacts_are_wired() {
    let default_nix = read_repo_file("nixos-modules/default.nix");
    for module in [
        "./realm-workloads-launcher-v2-json.nix",
        "./unsafe-local-workloads-json.nix",
        "./provider-registry-v2-json.nix",
    ] {
        assert!(
            default_nix.contains(module),
            "default.nix must import {module}"
        );
    }

    let bundle_artifacts = read_repo_file("nixos-modules/bundle-artifacts.nix");
    for artifact in [
        "realmWorkloadsLauncherV2Json",
        "unsafeLocalWorkloadsJson",
        "providerRegistryV2Json",
    ] {
        assert!(
            bundle_artifacts.contains(artifact),
            "bundle-artifacts.nix must declare {artifact}"
        );
    }
}

#[test]
fn generated_unsafe_local_schemas_are_closed_and_argv_is_private() {
    let public_schema =
        read_repo_file("docs/reference/schemas/v2/realm-workloads-launcher-v2.json");
    let private_schema = read_repo_file("docs/reference/schemas/v2/unsafe-local-workloads.json");
    let helper_schema = read_repo_file("docs/reference/schemas/v2/unsafe-local-helper-wire.json");

    assert!(!public_schema.contains("\"argv\""));
    assert!(public_schema.contains("\"providerKind\""));
    assert!(public_schema.contains("\"executionPosture\""));
    assert!(private_schema.contains("\"argv\""));
    assert!(private_schema.contains("\"additionalProperties\": false"));
    let private_schema: serde_json::Value = serde_json::from_str(&private_schema).unwrap();
    assert_eq!(
        private_schema["properties"]["workloads"]["maxItems"],
        serde_json::json!(256)
    );
    assert_eq!(
        private_schema["properties"]["localVmWorkloads"]["maxItems"],
        serde_json::json!(256)
    );
    assert_eq!(
        private_schema["definitions"]["LocalVmConfiguredWorkload"]["properties"]["items"]["minItems"],
        serde_json::json!(1)
    );
    assert!(helper_schema.contains("\"protocolVersion\""));
    assert!(helper_schema.contains("\"terminalProtocolVersion\""));
}

#[test]
fn generated_provider_registry_schema_is_closed_and_authority_free() {
    let schema_text = read_repo_file("docs/reference/schemas/v2/provider-registry-v2.json");
    let schema: Value = serde_json::from_str(&schema_text).expect("provider registry schema");
    assert_eq!(schema["additionalProperties"], serde_json::json!(false));
    for forbidden in ["\"argv\"", "\"secret\""] {
        assert!(
            !schema_text.contains(forbidden),
            "provider registry schema must not contain {forbidden}"
        );
    }
    assert!(schema_text.contains("\"local-runtime\""));
    assert!(schema_text.contains("\"vmStartIntentId\""));
    assert!(schema_text.contains("\"runnerIntentId\""));
}

#[test]
fn helper_shell_schema_is_correlated_bounded_and_authority_free() {
    let helper_schema = read_repo_file("docs/reference/schemas/v2/unsafe-local-helper-wire.json");
    let helper_schema: serde_json::Value = serde_json::from_str(&helper_schema).unwrap();
    for root_field in [
        "daemonToHelper",
        "helperToDaemon",
        "protocolVersion",
        "terminalProtocolVersion",
        "terminalRequest",
        "terminalResponse",
    ] {
        assert!(
            helper_schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|field| field == root_field),
            "helper schema must require {root_field}"
        );
    }

    let definitions = helper_schema["definitions"].as_object().unwrap();
    let shell_request = serde_json::to_string(&definitions["HelperShellRequest"]).unwrap();
    for field in ["requestId", "operationId", "initialTerminalSize", "policy"] {
        assert!(
            shell_request.contains(field),
            "helper shell request schema must use wire field {field}"
        );
    }
    let shell_policy = &definitions["HelperShellPolicy"];
    assert_eq!(
        shell_policy["properties"]["maxSessions"]["minimum"].as_f64(),
        Some(1.0)
    );
    assert_eq!(
        shell_policy["properties"]["maxSessions"]["maximum"].as_f64(),
        Some(64.0)
    );
    for required in ["defaultName", "maxSessions"] {
        assert!(
            shell_policy["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|field| field == required),
            "trusted helper shell policy must require {required}"
        );
    }
    for forbidden in ["request_id", "operation_id", "initial_terminal_size"] {
        assert!(
            !shell_request.contains(forbidden),
            "helper shell request schema must not expose Rust field {forbidden}"
        );
    }

    let shell_and_terminal = definitions
        .iter()
        .filter(|(name, _)| {
            name.starts_with("HelperShell")
                || name.starts_with("HelperTerminal")
                || *name == "HelperPersistentShellSnapshot"
        })
        .map(|(_, definition)| definition)
        .collect::<Vec<_>>();
    let shell_and_terminal = serde_json::to_string(&shell_and_terminal).unwrap();
    for forbidden in [
        "\"uid\"",
        "\"argv\"",
        "\"environment\"",
        "\"cwd\"",
        "\"path\"",
        "\"transcript\"",
        "\"pid\"",
        "\"unitName\"",
        "\"compositor\"",
        "\"session\"",
    ] {
        assert!(
            !shell_and_terminal.contains(forbidden),
            "helper shell/terminal schema must not contain {forbidden}"
        );
    }
}

#[test]
fn rendered_launcher_metadata_hides_argv_and_private_bundle_resolves_it() {
    let Some(dir) = env::var_os("D2B_FIXTURES").map(std::path::PathBuf::from) else {
        eprintln!("  (skipping rendered unsafe-local contracts; D2B_FIXTURES unset)");
        return;
    };

    let public: RealmWorkloadsLauncherV2Json =
        read_fixture_json(&dir, "realm-workloads-launcher-v2.json");
    let private: UnsafeLocalWorkloadsJson = read_fixture_json(&dir, "unsafe-local-workloads.json");
    let provider_registry: ProviderRegistryV2 =
        read_fixture_json(&dir, "provider-registry-v2.json");
    let bundle: Bundle = read_fixture_json(&dir, "bundle.json");

    private.validate().expect("private artifact validates");
    provider_registry
        .validate()
        .expect("provider registry artifact validates");
    assert_eq!(public.schema_version, "v2");
    assert_eq!(public.workloads.len(), 2);
    assert_eq!(private.workloads.len(), 1);
    assert_eq!(provider_registry.providers.len(), 1);
    assert!(
        public
            .workloads
            .iter()
            .any(|workload| workload.realm_accent_color == "#cc3344")
    );

    let public_json = serde_json::to_string(&public).unwrap();
    assert!(!public_json.contains("rendered-private-argv-canary"));
    assert!(!public_json.contains("\"argv\""));

    let private_debug = format!("{private:?}");
    assert!(!private_debug.contains("rendered-private-argv-canary"));
    let exec = private.workloads[0]
        .items
        .iter()
        .find_map(|item| match item {
            d2b_core::unsafe_local_workloads::UnsafeLocalLauncherItem::Exec(item) => Some(item),
            d2b_core::unsafe_local_workloads::UnsafeLocalLauncherItem::Shell(_) => None,
        })
        .expect("configured exec item exists");
    assert!(
        exec.argv
            .as_slice()
            .iter()
            .any(|arg| arg == "rendered-private-argv-canary")
    );
    assert_eq!(
        bundle.realm_workloads_launcher_v2_path.as_deref(),
        Some("/etc/d2b/realm-workloads-launcher-v2.json")
    );
    assert_eq!(
        bundle.unsafe_local_workloads_path.as_deref(),
        Some("/etc/d2b/unsafe-local-workloads.json")
    );
    assert_eq!(
        bundle.provider_registry_v2_path.as_deref(),
        Some("/etc/d2b/provider-registry-v2.json")
    );
    let provider_entry = &provider_registry.providers[0];
    assert_eq!(
        provider_entry.descriptor.implementation_id.as_str(),
        "cloud-hypervisor"
    );
    assert!(matches!(
        &provider_entry.binding,
        ProviderBindingV2::LocalRuntime(_)
    ));
    let provider_json = serde_json::to_string(&provider_registry).unwrap();
    for forbidden in ["\"argv\"", "\"secret\"", "\"azure-vm\"", "runtime.execute"] {
        assert!(
            !provider_json.contains(forbidden),
            "rendered provider registry must not contain {forbidden}"
        );
    }
    let artifact_hashes = bundle
        .artifact_hashes
        .as_ref()
        .expect("rendered bundle carries artifact hashes");
    for path in [
        "/etc/d2b/realm-workloads-launcher-v2.json",
        "/etc/d2b/unsafe-local-workloads.json",
        "/etc/d2b/provider-registry-v2.json",
    ] {
        assert!(
            artifact_hashes.contains_key(path),
            "rendered bundle must hash {path}"
        );
    }
}

// ── realm-controller-config emitter: identity fields ─────────────────────────

/// The controller config emitter must wire workload identity as a **nested**
/// `identity = { ... }` object (matching the `RealmControllerLocalWorkload.identity:
/// Option<WorkloadIdentity>` field) with the correct field names.
///
/// Required WorkloadIdentity fields in the emitter:
///   - `workloadId`   (maps from workload name)
///   - `realmId`      (from `workloadRow.realmId`)
///   - `realmPath`    (as a Nix list, via `lib.splitString "." workloadRow.realmPath`)
///   - `canonicalTarget`
///
/// Optional WorkloadIdentity fields in the emitter:
///   - `legacyVmName`, `runtimeKind`, `providerId` (renamed from `runtimeProviderId`)
///
/// Fields that must NOT appear as identity keys:
///   - `kind`           (not in WorkloadIdentity; was a W15 pre-review error)
///   - `runtimeProviderId` as a JSON key (renamed to `providerId`)
///
/// The identity object must be nested (not flat-merged with `//` into the
/// workload root), because `RealmControllerLocalWorkload` has `deny_unknown_fields`.
#[test]
fn realm_controller_config_emitter_wires_workload_identity_fields() {
    let emitter = read_repo_file("nixos-modules/realm-controller-config-json.nix");

    // Required fields must appear as Nix keys inside the identity block:
    for field in [
        "workloadId",
        "realmId",
        "realmPath",
        "canonicalTarget",
        "legacyVmName",
        "runtimeKind",
    ] {
        assert!(
            emitter.contains(field),
            "realm-controller-config emitter must wire WorkloadIdentity field {field}"
        );
    }

    // The renamed field: the Nix key must be `providerId` (the DTO name),
    // not `runtimeProviderId`. The value source `workloadRow.runtimeProviderId`
    // may still appear, but `providerId` must be present as a key.
    assert!(
        emitter.contains("providerId"),
        "realm-controller-config emitter must use 'providerId' as the WorkloadIdentity key \
         (not runtimeProviderId)"
    );

    // Identity must be nested, not flat-merged: the `identity =` assignment
    // must appear so that the identity fields travel in a sub-object.
    assert!(
        emitter.contains("identity ="),
        "realm-controller-config emitter must nest workload identity under 'identity = {{ ... }}' \
         (RealmControllerLocalWorkload.identity: Option<WorkloadIdentity> — deny_unknown_fields \
         rejects flat identity keys at the workload root)"
    );

    // `kind` must NOT be used as a key inside the identity block.
    // It was a pre-review error: WorkloadIdentity has no `kind` field.
    // The emitter may still reference `workload.kind` elsewhere (index row
    // access), but there must not be a `kind = workloadRow.kind` assignment
    // inside the identity attrset.
    assert!(
        !emitter.contains("kind = workloadRow.kind"),
        "realm-controller-config emitter must not assign 'kind = workloadRow.kind' \
         inside the identity block; WorkloadIdentity has no 'kind' field"
    );

    // Bug-fix from W14: vmRef was renamed to legacyVmName; the emitter must
    // not reference the old name.
    assert!(
        !emitter.contains("row.vmRef"),
        "realm-controller-config emitter must not reference removed field 'row.vmRef'; use 'row.legacyVmName'"
    );
}

/// The controller config emitter must use `row.legacyVmName` (not the
/// previously broken `row.vmRef`) in all three call sites of
/// `localRuntimeWorkloadsFor`.  Also guards that the old flat field name
/// `runtimeProviderId` is not used as a JSON key in the identity object
/// (it was renamed to `providerId` to match WorkloadIdentity).
#[test]
fn realm_controller_config_emitter_uses_legacy_vm_name_not_vm_ref() {
    let emitter = read_repo_file("nixos-modules/realm-controller-config-json.nix");
    assert!(
        !emitter.contains("vmRef"),
        "realm-controller-config emitter must not contain any reference to 'vmRef' \
         (renamed to legacyVmName in W14)"
    );
    // `runtimeProviderId` may appear as the Nix value source
    // (workloadRow.runtimeProviderId) but must NOT appear as the
    // JSON key name; the DTO field is `providerId`.
    assert!(
        !emitter.contains("runtimeProviderId ="),
        "realm-controller-config emitter must not use 'runtimeProviderId =' as a key; \
         the WorkloadIdentity DTO field is 'providerId'"
    );
}

// ── Source-lint: deny_unknown_fields on workload identity structs ─────────────

/// `WorkloadIdentity`, `LocalVmBackendConfig`, `LocalQemuMediaBackendConfig`,
/// and `WorkloadRuntimeIntent` must carry `#[serde(deny_unknown_fields)]` (or a
/// combined rename_all + deny_unknown_fields attribute) so that unknown JSON
/// keys are rejected rather than silently ignored. This is a security-sensitive
/// gate: host-resident bundle artifacts parse these types.
#[test]
fn workload_identity_structs_carry_deny_unknown_fields() {
    let source = read_repo_file("packages/d2b-core/src/workload_identity.rs");

    for dto in [
        "WorkloadIdentity",
        "LocalVmBackendConfig",
        "LocalQemuMediaBackendConfig",
        "WorkloadRuntimeIntent",
    ] {
        // Find the struct declaration line:
        let decl_idx = source
            .lines()
            .position(|line| line.starts_with("pub struct ") && line.contains(dto))
            .unwrap_or_else(|| panic!("workload_identity.rs does not declare struct {dto}"));

        // The 10 lines before the declaration must include deny_unknown_fields:
        let lines: Vec<&str> = source.lines().collect();
        let start = decl_idx.saturating_sub(10);
        let window = &lines[start..decl_idx];
        assert!(
            window.iter().any(|l| l.contains("deny_unknown_fields")),
            "struct {dto} in workload_identity.rs must carry #[serde(deny_unknown_fields)] \
             within the 10 lines preceding the declaration"
        );
    }
}

// ── Source-lint: additive-vs-breaking version policy documented ──────────────

/// The `workload_identity` module doc comment must explain the additive-vs-breaking
/// version policy and name both `bundleVersion` and `schemaVersion` as the required
/// bumps for breaking changes. This guards against the policy comment being silently
/// removed or made incomplete.
#[test]
fn workload_identity_module_doc_names_version_bump_requirements() {
    let source = read_repo_file("packages/d2b-core/src/workload_identity.rs");
    for marker in [
        "bundleVersion",
        "schemaVersion",
        "Additive changes",
        "Breaking changes",
    ] {
        assert!(
            source.contains(marker),
            "workload_identity.rs module doc must mention {marker} as part of the DTO version policy"
        );
    }
}
