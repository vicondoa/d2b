use d2b_contract_tests::read_repo_file;
use serde_json::Value;

fn realm_core_schema() -> Value {
    serde_json::from_str(&read_repo_file(
        "docs/reference/schemas/v2/d2b-realm-core.json",
    ))
    .expect("d2b-realm-core schema parses")
}

fn definition<'a>(schema: &'a Value, name: &str) -> &'a Value {
    schema
        .get("definitions")
        .and_then(Value::as_object)
        .and_then(|definitions| definitions.get(name))
        .unwrap_or_else(|| panic!("schema is missing definition {name}"))
}

#[test]
fn realm_access_dtos_are_in_generated_schema_contract() {
    let schema = realm_core_schema();
    let root_refs = schema
        .get("anyOf")
        .or_else(|| schema.get("oneOf"))
        .map(Value::to_string)
        .unwrap_or_else(|| schema.to_string());

    for dto in [
        "RealmAccessBinding",
        "RealmAccessTargetInput",
        "RealmAccessAliasSource",
        "RealmAccessAliasBinding",
        "RealmAccessClientBindingKind",
        "RealmAccessClientContract",
        "RealmAccessClientBinding",
        "RealmAccessCapabilityPreflight",
        "RealmAccessConflictCandidate",
        "RealmAccessResolverDiagnostic",
        "RealmAccessResolverError",
        "RealmAccessResolverRequest",
        "RealmAccessResolverResponse",
    ] {
        definition(&schema, dto);
        assert!(
            root_refs.contains(&format!("#/definitions/{dto}")),
            "d2b-realm-core root schema must expose {dto}"
        );
    }
}

#[test]
fn direct_host_local_binding_schema_preserves_peercred_no_proxy_contract() {
    let schema = realm_core_schema();
    let client_binding = definition(&schema, "RealmAccessClientBinding").to_string();
    let peercred = definition(&schema, "HostLocalPeerCredentialSemantics").to_string();
    let peercred_source = definition(&schema, "HostLocalPeerCredentialSource").to_string();
    let peercred_checker = definition(&schema, "HostLocalPeerCredentialChecker").to_string();
    let proxy_status = definition(&schema, "HostLocalProxyStatus").to_string();

    assert!(client_binding.contains("direct-host-local-unix"));
    assert!(client_binding.contains("socket_path"));
    assert!(client_binding.contains("peer_credentials"));
    assert!(peercred.contains("HostLocalPeerCredentialSource"));
    assert!(peercred.contains("HostLocalPeerCredentialChecker"));
    assert!(peercred.contains("HostLocalProxyStatus"));
    assert!(peercred_source.contains("connecting-client-process"));
    assert!(peercred_checker.contains("d2bd-public-socket"));
    assert!(proxy_status.contains("no-byte-proxy"));
    assert!(!client_binding.contains("byteProxy"));
    assert!(!client_binding.contains("proxyAllowed"));
}

#[test]
fn resolver_diagnostics_and_docs_cover_access_failure_surface() {
    let schema = realm_core_schema();
    let diagnostics = definition(&schema, "RealmAccessResolverDiagnostic").to_string();
    let docs = read_repo_file("docs/reference/realm-access-resolver.md");
    let schema_docs = read_repo_file("docs/reference/schemas/v2/d2b-realm-core.md");

    for diagnostic in [
        "alias-ambiguous",
        "old-node-qualified-target",
        "missing-realm-binding",
        "unsupported-cross-realm-capability",
        "stale-realm-controller",
        "missing-realm-controller",
    ] {
        assert!(diagnostics.contains(diagnostic));
        assert!(docs.contains(diagnostic));
    }

    for doc_marker in [
        "RealmAccessResolverRequest",
        "RealmAccessResolverResponse",
        "direct-host-local-unix",
        "no-byte-proxy",
    ] {
        assert!(
            schema_docs.contains(doc_marker),
            "schema reference docs must mention {doc_marker}"
        );
    }
}
