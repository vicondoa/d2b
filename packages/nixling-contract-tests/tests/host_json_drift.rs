//! host.json per-field schema gold-file drift gate, migrated from
//! `tests/host-json-drift-gate.sh`.
//!
//! The shell gate was a raw JSON/source-doc policy lint: the committed
//! `tests/golden/host-json/*.json` fixtures predate today's
//! `nixling_core::host::HostJson` DTO shape in places, so the golden checks
//! intentionally assert over `serde_json::Value` rather than deserializing the
//! historical fixtures as `HostJson`.

use std::{env, fs};

use nixling_contract_tests::{read_repo_file, repo_path_exists, repo_root};
use regex::Regex;
use serde_json::{Map, Value};

const HOST_JSON_FIXTURES_DIR: &str = "tests/golden/host-json";
const LIVE_SHAPE_FIXTURE: &str = "tests/fixtures/deny-unknown/host-valid.json";
const HOST_SCHEMA: &str = "docs/reference/schemas/v2/host.json";
const HOST_SCHEMA_MD: &str = "docs/reference/schemas/v2/host.md";

const REQUIRED_BASELINE_FIELDS: &[&str] = &[
    "schemaVersion",
    "bundleVersion",
    "site",
    "environments",
    "nftables",
    "networkManager",
    "hostsFile",
    "kernelModules",
    "bridgePortFlags",
    "firewallCoexistence",
    "ifnameMapping",
    "ch",
];

const EXPECTED_REJECTIONS: &[(&str, &str)] = &[
    ("ifname-collision.json", "ifname-collision"),
    ("ifname-too-long.json", "ifname-too-long"),
    ("unknown-field-kernelmodules.json", "wire-unknown-field"),
    ("unknown-field-bridgeportflags.json", "wire-unknown-field"),
    (
        "unknown-field-firewallcoexistence.json",
        "wire-unknown-field",
    ),
    ("unknown-field-ifnamemapping.json", "wire-unknown-field"),
];

const CANONICAL_LIVE_OWNERSHIP: &[(&str, &[&str], &str)] = &[
    (
        "networkManager.filePath",
        &["networkManager", "filePath"],
        "/etc/NetworkManager/conf.d/00-nixling-unmanaged.conf",
    ),
    (
        "hostsFile.startMarker",
        &["hostsFile", "startMarker"],
        "# nixling-managed begin",
    ),
    (
        "hostsFile.endMarker",
        &["hostsFile", "endMarker"],
        "# nixling-managed end",
    ),
];

fn fixture_rel(filename: &str) -> String {
    format!("{HOST_JSON_FIXTURES_DIR}/{filename}")
}

fn parse_repo_json(rel: &str) -> Value {
    let raw = read_repo_file(rel);
    serde_json::from_str(&raw).unwrap_or_else(|err| panic!("{rel}: invalid JSON ({err})"))
}

fn as_object<'a>(value: &'a Value, context: &str) -> &'a Map<String, Value> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("{context}: expected JSON object"))
}

fn get_path_str<'a>(value: &'a Value, path: &[&str], context: &str) -> Option<&'a str> {
    let mut cursor = value;
    for segment in path {
        cursor = cursor.get(*segment)?;
    }
    cursor.as_str().or_else(|| {
        panic!(
            "{context}: {} is present but is not a string",
            path.join(".")
        )
    })
}

fn sorted_missing_fields<'a>(
    object: &Map<String, Value>,
    required: impl IntoIterator<Item = &'a str>,
) -> Vec<&'a str> {
    let mut missing = required
        .into_iter()
        .filter(|field| !object.contains_key(*field))
        .collect::<Vec<_>>();
    missing.sort_unstable();
    missing
}

// ---------------------------------------------------------------------------
// Migrated from tests/host-json-drift-gate.sh: fixture directory + baseline
// fixture checks.
//
// Asserts the committed baseline exists, parses as JSON, carries the exact
// REQUIRED_BASELINE_FIELDS set used by the shell gate, and every
// environments[*].bridge matches the documented hash-placeholder/8-hex regex
// without exceeding IFNAMSIZ-1 (15 bytes).
// ---------------------------------------------------------------------------
#[test]
fn baseline_host_json_required_fields_and_ifnames() {
    let fixtures_dir = repo_root().join(HOST_JSON_FIXTURES_DIR);
    assert!(
        fixtures_dir.is_dir(),
        "host-json-drift-gate: fixtures directory missing: {}",
        fixtures_dir.display()
    );

    let baseline_rel = fixture_rel("baseline-host.json");
    assert!(
        repo_path_exists(&baseline_rel),
        "missing baseline fixture: {baseline_rel}"
    );
    let data = parse_repo_json(&baseline_rel);
    let object = as_object(&data, "baseline-host.json");

    let missing = sorted_missing_fields(object, REQUIRED_BASELINE_FIELDS.iter().copied());
    assert!(
        missing.is_empty(),
        "baseline-host.json missing required fields: {missing:?}"
    );

    let ifname_re =
        Regex::new(r"^nl-[a-z][a-z0-9-]*-(XXXXXXXX|[0-9a-f]{8})$").expect("valid ifname regex");
    let environments = data
        .get("environments")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("baseline-host.json: environments must be an array"));
    for env in environments {
        let env = as_object(env, "baseline-host.json: environments[]");
        let bridge = env.get("bridge").and_then(Value::as_str).unwrap_or("");
        assert!(
            ifname_re.is_match(bridge),
            "baseline-host.json: bridge {bridge:?} does not match nl-<env>-(XXXXXXXX|[0-9a-f]{{8}})"
        );
        assert!(
            bridge.len() <= 15,
            "baseline-host.json: bridge {bridge:?} exceeds IFNAMSIZ-1 (15 bytes)"
        );
    }
}

// ---------------------------------------------------------------------------
// Migrated from tests/host-json-drift-gate.sh: broker-canonical ownership
// constants in the committed live-shape fixture.
//
// Asserts tests/fixtures/deny-unknown/host-valid.json exists, parses as JSON,
// and carries the canonical NetworkManager/hosts ownership marker strings that
// must agree with the Rust broker constants.
// ---------------------------------------------------------------------------
#[test]
fn live_shape_fixture_uses_broker_canonical_ownership() {
    assert!(
        repo_path_exists(LIVE_SHAPE_FIXTURE),
        "missing live-shape fixture: {LIVE_SHAPE_FIXTURE}"
    );
    let live_data = parse_repo_json(LIVE_SHAPE_FIXTURE);

    for (key, path, expected) in CANONICAL_LIVE_OWNERSHIP {
        let observed = get_path_str(&live_data, path, "host-valid.json");
        assert_eq!(
            observed,
            Some(*expected),
            "host-valid.json: {key} is {observed:?}, expected {expected:?} \
             (broker-canonical, see packages/nixling-priv-broker/src/ops/nm.rs::DEFAULT_NM_CONF_PATH \
             and packages/nixling-host/src/routes.rs::HOSTS_MANAGED_BEGIN/END)"
        );
    }
}

// ---------------------------------------------------------------------------
// Migrated from tests/host-json-drift-gate.sh: optional smoke-rendered host.json
// top-level field check.
//
// The shell gate skipped this assertion when nl_smoke_bundle_host_json was not
// available. This Rust migration preserves that behavior by checking
// NL_FIXTURES/host.json only when the fixture environment is provided; the
// always-run committed fixture checks above do not require NL_FIXTURES.
// ---------------------------------------------------------------------------
#[test]
fn rendered_smoke_host_json_emits_firewall_policy_when_fixture_available() {
    let Some(fixtures_dir) = env::var_os("NL_FIXTURES") else {
        eprintln!(
            "  (skipping smoke host.json top-level field check — NL_FIXTURES unavailable in this gate context)"
        );
        return;
    };
    let smoke_host_path = std::path::PathBuf::from(fixtures_dir).join("host.json");
    let raw = fs::read_to_string(&smoke_host_path).unwrap_or_else(|err| {
        panic!(
            "smoke host.json ({}): cannot read fixture: {err}",
            smoke_host_path.display()
        )
    });
    let smoke_data: Value = serde_json::from_str(&raw).unwrap_or_else(|err| {
        panic!(
            "smoke host.json ({}): invalid JSON ({err})",
            smoke_host_path.display()
        )
    });
    let smoke_object = as_object(&smoke_data, "smoke host.json");
    assert!(
        smoke_object.contains_key("firewallCoexistencePolicy"),
        "smoke host.json ({}): missing required top-level field emitted by \
         nixos-modules/host-json.nix: [\"firewallCoexistencePolicy\"]. The host schema \
         contract requires firewallCoexistencePolicy to be emitted even though the Rust DTO \
         is Option, so the broker always sees a coexistence policy at apply-time.",
        smoke_host_path.display()
    );
}

// ---------------------------------------------------------------------------
// Migrated from tests/host-json-drift-gate.sh: malicious golden rejection-code
// declarations.
//
// Asserts every named committed malicious fixture exists, parses as JSON, and
// declares the exact `_expectedRejection.code` expected by the shell gate.
// ---------------------------------------------------------------------------
#[test]
fn malicious_host_json_fixtures_declare_expected_rejections() {
    for (filename, expected_code) in EXPECTED_REJECTIONS {
        let rel = fixture_rel(filename);
        assert!(
            repo_path_exists(&rel),
            "missing malicious fixture: {filename}"
        );
        let data = parse_repo_json(&rel);
        let rejection_code = data
            .get("_expectedRejection")
            .and_then(|rejection| rejection.get("code"))
            .and_then(Value::as_str);
        assert_eq!(
            rejection_code,
            Some(*expected_code),
            "{filename}: _expectedRejection.code is {rejection_code:?}, expected {expected_code:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Migrated from tests/host-json-drift-gate.sh: schema cross-check, guarded by
// the same "when docs/reference/schemas/v2/host.json exists" condition.
//
// Asserts every security-sensitive sub-object definition named by the shell gate
// exists and carries `additionalProperties: false`.
// ---------------------------------------------------------------------------
#[test]
fn v2_host_schema_security_definitions_deny_additional_properties() {
    if !repo_path_exists(HOST_SCHEMA) {
        eprintln!("  (v2 host.json schema {HOST_SCHEMA} not present; skipping schema cross-check)");
        return;
    }

    let schema = parse_repo_json(HOST_SCHEMA);
    let defs = schema
        .get("definitions")
        .or_else(|| schema.get("$defs"))
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("v2 host.json schema is missing definitions/$defs object"));

    for sub in [
        "KernelModulesEntry",
        "BridgePortFlags",
        "IfNameMapping",
        "FirewallCoexistencePolicy",
    ] {
        let definition = defs
            .get(sub)
            .unwrap_or_else(|| panic!("v2 host.json schema is missing the {sub} definition"));
        assert_eq!(
            definition.get("additionalProperties"),
            Some(&Value::Bool(false)),
            "v2 host.json schema: {sub}.additionalProperties must be false"
        );
    }
}

// ---------------------------------------------------------------------------
// Migrated from tests/host-json-drift-gate.sh: schema prose parity, guarded by
// the same "when docs/reference/schemas/v2/host.md exists" condition.
//
// Asserts every field name in the shell gate's prose list appears in host.md.
// ---------------------------------------------------------------------------
#[test]
fn v2_host_schema_markdown_documents_layer1_fields() {
    if !repo_path_exists(HOST_SCHEMA_MD) {
        eprintln!("  (v2 host.md prose {HOST_SCHEMA_MD} not present; skipping prose cross-check)");
        return;
    }

    let prose = read_repo_file(HOST_SCHEMA_MD);
    for field in [
        "kernelModules",
        "bridgePortFlags",
        "firewallCoexistence",
        "ifnameMapping",
        "ch",
        "ipv6Sysctls",
    ] {
        assert!(
            prose.contains(field),
            "v2 host.md prose does not document the {field:?} field"
        );
    }
}
