//! Contract and schema-drift guards for realm workload DTOs introduced in Wave 15.
//!
//! Coverage:
//!  * `WorkloadIdentity`, `WorkloadTarget` and related types appear in generated schemas.
//!  * Additive-field invariant: `identity` on `RealmControllerLocalWorkload` and
//!    `workloadIdentity` on `ListEntry`/`VmStatus` are NOT in `required[]`.
//!  * Wire-protocol schema separation: workload identity travels in the daemon-wire
//!    schema (`wire-protocol.json`), NOT in CLI output schemas (`list.schema.json`,
//!    `status.schema.json`).
//!  * `desktop-metadata.json` is a bounded, argv-free, non-authoritative
//!    presentation projection keyed by canonical realm/provider ids and workload
//!    targets from the normalized index.
//!  * Source-lint: `WorkloadIdentity` and sibling structs carry `deny_unknown_fields`;
//!    the module-level doc policy comment names both `bundleVersion` and `schemaVersion`
//!    as the required bumps for breaking changes.
//!  * `realm-controllers.json` contains no sensitive credential fields.

use d2b_contract_tests::read_repo_file;
use d2b_contracts::provider_registry_v2::{ProviderBindingV2, ProviderRegistryV2};
use d2b_core::{bundle::Bundle, unsafe_local_workloads::UnsafeLocalWorkloadsJson};
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

/// Desktop identity comes from the normalized index. Presentation details stay
/// nested under metadata/launcher rather than becoming identity aliases.
#[test]
fn normalized_index_owns_canonical_desktop_identity() {
    let index = read_repo_file("nixos-modules/index.nix");
    let realms = read_repo_file("nixos-modules/index-realms.nix");
    let workloads = read_repo_file("nixos-modules/index-workloads.nix");
    let resources = read_repo_file("nixos-modules/index-resources.nix");

    for marker in [
        "realms = realmIndex;",
        "workloads = workloadIndex //",
        "providerBindings =",
        "resourceIndex.providers.bindingsByWorkloadId",
    ] {
        assert!(
            index.contains(marker),
            "normalized index must expose canonical desktop source: {marker}"
        );
    }
    assert!(realms.contains("canonicalTargetSuffix = \"${realmPath}.d2b\";"));
    assert!(
        workloads.contains("canonicalTarget = \"${canonicalName}.${realmRow.realmPath}.d2b\";")
    );
    assert!(workloads.contains("metadata = {"));
    assert!(workloads.contains("launcher = {"));
    assert!(resources.contains("providerId ="));
    assert!(resources.contains("bindingsByWorkloadId"));
    for forbidden in ["iconGroupKey", "legacyVmName", "targetAddress"] {
        assert!(
            !index.contains(forbidden),
            "normalized index must not restore legacy desktop alias {forbidden}"
        );
    }
}

#[test]
fn clipboard_endpoints_follow_canonical_workload_and_provider_bindings() {
    let emitter = read_repo_file("nixos-modules/clipboard.nix");
    for marker in [
        "config.d2b._index.workloads.enabledList",
        "config.d2b._index.providerRegistryV2Mappings.display",
        "workload.providerBindings.runtime",
        "inherit (workload) canonicalTarget realmId workloadId",
        "runtimeProviderId = runtime.providerId;",
        "displayProviderId = display.providerId;",
        "socketComponent = display.endpointIds.proxy;",
    ] {
        assert!(
            emitter.contains(marker),
            "clipboard endpoint policy must consume canonical marker {marker}"
        );
    }
    for forbidden in [
        "config.d2b.vms",
        "normalNixosVms",
        "qemuMediaVms",
        "legacyVmName",
        "providerKind",
    ] {
        assert!(
            !emitter.contains(forbidden),
            "clipboard endpoint policy must not derive identity through {forbidden}"
        );
    }
}

#[test]
fn desktop_metadata_artifact_is_public_non_secret() {
    let emitter = read_repo_file("nixos-modules/desktop-metadata-json.nix");
    for marker in [
        "desktopMetadataJson",
        "installFileName = \"desktop-metadata.json\";",
        "classification = \"contractPublic\";",
        "sensitivity = \"nonSecret\";",
    ] {
        assert!(
            emitter.contains(marker),
            "desktop metadata emitter must contain {marker}"
        );
    }
}

#[test]
fn desktop_metadata_consumes_normalized_rows_without_rederiving_ids() {
    let emitter = read_repo_file("nixos-modules/desktop-metadata-json.nix");
    for marker in [
        "cfg._index.realms.enabledList",
        "cfg._index.workloads.enabledList",
        "cfg._index.providers.enabledList",
        "inherit (realm) realmId",
        "inherit (workload) canonicalTarget realmId workloadId",
        "inherit (provider) providerId realmId",
    ] {
        assert!(
            emitter.contains(marker),
            "desktop metadata must consume normalized marker {marker}"
        );
    }
    for forbidden in [
        "deriveRealmId",
        "deriveWorkloadId",
        "deriveProviderId",
        "v2-identity.nix",
    ] {
        assert!(
            !emitter.contains(forbidden),
            "desktop metadata must not rederive normalized identity via {forbidden}"
        );
    }
}

#[test]
fn desktop_metadata_keeps_configured_argv_private() {
    let emitter = read_repo_file("nixos-modules/desktop-metadata-json.nix");
    assert!(emitter.contains("argvPrivate = true;"));
    assert!(emitter.contains("items = lib.mapAttrsToList publicItem workload.launcher.items;"));
    for forbidden in [
        "item.argv",
        "argv = item",
        "inherit (item) argv",
        "appCommand",
    ] {
        assert!(
            !emitter.contains(forbidden),
            "public desktop metadata must not project configured argv through {forbidden}"
        );
    }
}

#[test]
fn private_launcher_intents_use_normalized_workload_provider_identity() {
    let emitter = read_repo_file("nixos-modules/unsafe-local-workloads-json.nix");
    for marker in [
        "cfg._index.workloads.enabledList",
        "workload.providerBindings.runtime",
        "inherit (workload) workloadId realmId canonicalTarget",
        "runtimeKind = runtime.implementationId;",
        "providerId = runtime.providerId;",
        "items = privateItems workload.launcher.items;",
    ] {
        assert!(
            emitter.contains(marker),
            "private launcher intent must consume canonical marker {marker}"
        );
    }
    for forbidden in [
        "cfg.vms",
        "legacyVmName",
        "runtimeProviderId",
        "providerId = \"unsafe-local\"",
    ] {
        assert!(
            !emitter.contains(forbidden),
            "private launcher intent must not restore legacy identity through {forbidden}"
        );
    }
}

#[test]
fn desktop_metadata_preserves_presentation_without_legacy_group_aliases() {
    let emitter = read_repo_file("nixos-modules/desktop-metadata-json.nix");
    for marker in [
        "icon = publicIcon workload.metadata.icon;",
        "label = workload.metadata.label;",
        "realmAccentColor",
        "accentColor",
    ] {
        assert!(
            emitter.contains(marker),
            "desktop metadata must preserve presentation marker {marker}"
        );
    }
    for forbidden in ["iconGroupKey", "iconId =", "iconName ="] {
        assert!(
            !emitter.contains(forbidden),
            "desktop metadata must not restore presentation alias {forbidden}"
        );
    }
}

#[test]
fn desktop_metadata_is_bounded_and_non_authoritative() {
    let emitter = read_repo_file("nixos-modules/desktop-metadata-json.nix");
    for marker in [
        "maxRealms = 64;",
        "maxWorkloads = 256;",
        "maxProviders = 256;",
        "maxItemsPerWorkload = 64;",
        "maxCapabilitiesPerEntry = 64;",
        "colorsArePresentationOnly = true;",
        "metadataIsNotAuthorization = true;",
        "nonAuthoritativeProjection = true;",
    ] {
        assert!(
            emitter.contains(marker),
            "desktop metadata contract must contain {marker}"
        );
    }
}

#[test]
fn desktop_metadata_maps_systemd_user_to_unsafe_local_posture() {
    let emitter = read_repo_file("nixos-modules/desktop-metadata-json.nix");
    for marker in [
        "implementationId == \"systemd-user\"",
        "isolation = \"unsafe-local\";",
        "environment = \"systemd-user-manager-ambient\";",
        "displayEnvironment = \"wayland-proxy-only\";",
        "executionIdentity = \"authenticated-requester-uid\";",
        "sessionPersistence = \"user-manager-lifetime\";",
    ] {
        assert!(
            emitter.contains(marker),
            "systemd-user desktop posture must contain {marker}"
        );
    }
}

#[test]
fn legacy_launcher_emitters_do_not_emit_compatibility_artifacts() {
    assert!(
        !Path::new(&env!("CARGO_MANIFEST_DIR"))
            .join("../../nixos-modules/realm-workloads-launcher-json.nix")
            .exists(),
        "legacy realm-workloads-launcher emitter must remain deleted"
    );
    assert_eq!(
        read_repo_file("nixos-modules/realm-workloads-launcher-v2-json.nix").trim(),
        "{ }",
        "legacy v2 launcher emitter must not alias or emit desktop metadata"
    );
}

#[test]
fn private_launcher_schema_remains_closed_and_argv_bearing() {
    let private_schema = read_repo_file("docs/reference/schemas/v2/unsafe-local-workloads.json");
    let helper_schema = read_repo_file("docs/reference/schemas/v2/unsafe-local-helper-wire.json");

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
    assert!(schema_text.contains("\"local-observability\""));
    assert!(schema_text.contains("\"vmStartIntentId\""));
    assert!(schema_text.contains("\"runnerIntentId\""));
    let variants = schema["definitions"]["ProviderBindingV2"]["oneOf"]
        .as_array()
        .expect("provider binding variants");
    let find_variant = |axis: &str| {
        variants
            .iter()
            .find(|variant| {
                variant["properties"]["axis"]["enum"]
                    .as_array()
                    .is_some_and(|values| values.iter().any(|value| value == axis))
            })
            .unwrap_or_else(|| panic!("provider binding variant {axis}"))
    };
    let local_runtime = find_variant("local-runtime");
    assert_eq!(
        local_runtime["additionalProperties"],
        serde_json::json!(false)
    );
    assert!(
        local_runtime["properties"].get("realmId").is_none(),
        "local runtime binding realm must come exclusively from descriptor placement"
    );
    assert!(local_runtime["properties"].get("workloadId").is_some());
    let local_observability = find_variant("local-observability");
    assert_eq!(
        local_observability["additionalProperties"],
        serde_json::json!(false)
    );
    assert_eq!(
        local_observability["properties"]["maxRecords"]["maximum"],
        serde_json::json!(256.0)
    );
    assert_eq!(
        local_observability["properties"]["maxBytes"]["maximum"],
        serde_json::json!(1_048_576.0)
    );
    assert_eq!(
        local_observability["properties"]["maxTimeWindowMs"]["maximum"],
        serde_json::json!(2_678_400_000_f64)
    );
    for forbidden in ["realmId", "workloadId", "providerId"] {
        assert!(
            local_observability["properties"].get(forbidden).is_none(),
            "local observability binding must not carry {forbidden}"
        );
    }
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
fn rendered_private_launcher_intent_resolves_argv_without_debug_leakage() {
    let Some(dir) = env::var_os("D2B_FIXTURES").map(std::path::PathBuf::from) else {
        eprintln!("  (skipping rendered unsafe-local contracts; D2B_FIXTURES unset)");
        return;
    };

    let private: UnsafeLocalWorkloadsJson = read_fixture_json(&dir, "unsafe-local-workloads.json");
    let provider_registry: ProviderRegistryV2 =
        read_fixture_json(&dir, "provider-registry-v2.json");
    let bundle: Bundle = read_fixture_json(&dir, "bundle.json");

    private.validate().expect("private artifact validates");
    provider_registry
        .validate()
        .expect("provider registry artifact validates");
    assert_eq!(private.workloads.len(), 1);
    assert_eq!(
        provider_registry
            .providers
            .iter()
            .filter(|entry| matches!(&entry.binding, ProviderBindingV2::LocalRuntime(_)))
            .count(),
        1
    );
    let private_debug = format!("{private:?}");
    assert!(!private_debug.contains("rendered-private-argv-canary"));
    let configured_workload = &private.workloads[0];
    assert_eq!(
        configured_workload.identity.canonical_target.to_canonical(),
        "tools.host.local-root.d2b"
    );
    assert_ne!(
        configured_workload
            .identity
            .provider_id
            .as_ref()
            .expect("private intent carries canonical runtime provider id")
            .as_str(),
        "unsafe-local"
    );
    assert_eq!(
        configured_workload
            .identity
            .runtime_kind
            .as_ref()
            .expect("private intent carries runtime implementation")
            .as_str(),
        "systemd-user"
    );
    let exec = configured_workload
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
        bundle.unsafe_local_workloads_path.as_deref(),
        Some("/etc/d2b/unsafe-local-workloads.json")
    );
    assert_eq!(
        bundle.provider_registry_v2_path.as_deref(),
        Some("/etc/d2b/provider-registry-v2.json")
    );
    let provider_entry = provider_registry
        .providers
        .iter()
        .find(|entry| matches!(&entry.binding, ProviderBindingV2::LocalRuntime(_)))
        .expect("rendered local runtime provider");
    assert_eq!(
        provider_entry.descriptor.implementation_id.as_str(),
        "cloud-hypervisor"
    );
    assert!(matches!(
        &provider_entry.binding,
        ProviderBindingV2::LocalRuntime(_)
    ));
    let binding_json = serde_json::to_value(&provider_entry.binding).unwrap();
    assert!(binding_json.get("realmId").is_none());
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
        "/etc/d2b/unsafe-local-workloads.json",
        "/etc/d2b/provider-registry-v2.json",
    ] {
        assert!(
            artifact_hashes.contains_key(path),
            "rendered bundle must hash {path}"
        );
    }

    let controllers: Value = read_fixture_json(&dir, "realm-controllers.json");
    let controllers = controllers["controllers"]
        .as_array()
        .expect("rendered controller list");
    assert!(!controllers.is_empty());
    for controller in controllers {
        let providers = controller["providers"]
            .as_array()
            .expect("controller canonical provider list");
        for provider in providers {
            assert!(provider["providerId"].as_str().is_some());
            assert!(provider["providerName"].as_str().is_some());
            for forbidden in ["legacyVmName", "providerKind", "runtimeProviderId"] {
                assert!(
                    provider.get(forbidden).is_none(),
                    "controller provider metadata must not carry {forbidden}"
                );
            }
        }
    }
}

// ── realm-controller-config emitter: canonical normalized metadata ────────────

#[test]
fn realm_controller_config_emitter_wires_workload_identity_fields() {
    let emitter = read_repo_file("nixos-modules/realm-controller-config-json.nix");
    for marker in [
        "cfg._index.realms.byId.${row.realmId}",
        "cfg._index.providers.enabledList",
        "inherit (provider)",
        "providerName",
        "providerId",
        "kind = provider.providerType;",
        "providers = providersFor row.realmId;",
    ] {
        assert!(
            emitter.contains(marker),
            "realm controller artifact must consume canonical normalized marker {marker}"
        );
    }
}

#[test]
fn realm_controller_config_emitter_uses_legacy_vm_name_not_vm_ref() {
    let emitter = read_repo_file("nixos-modules/realm-controller-config-json.nix");
    for forbidden in [
        "cfg.vms",
        "vmRef",
        "legacyVmName",
        "runtimeProviderId",
        "providerKind",
    ] {
        assert!(
            !emitter.contains(forbidden),
            "realm controller artifact must not restore legacy identity through {forbidden}"
        );
    }
    assert!(
        emitter.contains("localRuntime = null;"),
        "realm controller metadata must not synthesize legacy local runtime authority"
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
