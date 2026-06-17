//! Observability / wave-evidence policy + doc/source lints (the "H-group"),
//! migrated from the `tests/*-eval.sh` bash gates. Each test reads the real
//! repo files (via the `nixling_contract_tests` repo-file helpers) and asserts
//! a structural / documentation invariant over the native SigNoz/OpenTelemetry
//! observability modules, the host-activation/secrets wiring, and the
//! readiness-wave evidence schema. This crate runs only from
//! `tests/tools/rust-workspace-checks.sh` against the real checkout (it is excluded
//! from the hermetic Nix sandbox workspace build), so repo-file access is
//! sound here.
//!
//! Migrated gates:
//!   * tests/loki-label-cardinality-eval.sh -> loki_native_otel_resource_attributes
//!   * tests/tempo-budget-eval.sh -> tempo_stack_signoz_backend_and_collector +
//!     tempo_guest_collector_shape + tempo_security_observability_and_retired_backends
//!   * tests/wave-evidence-schema-eval.sh -> wave_evidence_schema_cross_check

use std::collections::BTreeSet;

use nixling_contract_tests::{read_repo_file, repo_path_exists};
use regex::Regex;

/// Whether any single line of `content` matches `pattern`. This mirrors `grep`'s
/// per-line evaluation faithfully (so a `.*`/`\s*` in the pattern can never span
/// a newline boundary, as it could with a whole-file `Regex::is_match`).
fn any_line_matches(content: &str, pattern: &str) -> bool {
    let re = Regex::new(pattern).expect("valid regex");
    content.lines().any(|line| re.is_match(line))
}

/// The three live observability module sources the Loki/Tempo compatibility
/// gates scan, in the order the bash `FILES=(...)` array declared them.
const OBS_HOST: &str = "nixos-modules/components/observability/host.nix";
const OBS_STACK: &str = "nixos-modules/components/observability/stack.nix";
const OBS_GUEST: &str = "nixos-modules/components/observability/guest.nix";

// ===========================================================================
// Migrated from tests/loki-label-cardinality-eval.sh.
//
// The retired Loki label gate now guards the native SigNoz/OpenTelemetry
// resource-attribute contract: the live collector configs (host/stack/guest)
// must emit only a bounded allowlist of resource-attribute keys, must include
// the required identity keys, must avoid secret/high-cardinality/value-leaking
// keys, must carry the upsert + identity tokens, and must NOT promote the
// StoreSync `target_vm`/`target_env` JSON content into resource attributes.
// ===========================================================================
#[test]
fn loki_native_otel_resource_attributes() {
    // Bounded allowlist + required identity keys — verbatim ports of the bash
    // ALLOWED_RESOURCE_KEYS / REQUIRED_RESOURCE_KEYS arrays.
    let allowed_resource_keys: BTreeSet<&str> = [
        "deployment.environment",
        "host.name",
        "service.name",
        "service.namespace",
        "source",
        "vm.env",
        "vm.name",
        "vm.role",
    ]
    .into_iter()
    .collect();
    let required_resource_keys = ["service.name", "vm.env", "vm.name", "vm.role"];

    // (1) every live observability module must exist.
    for rel in [OBS_HOST, OBS_STACK, OBS_GUEST] {
        assert!(
            repo_path_exists(rel),
            "loki-label-cardinality-eval: missing file: {rel}"
        );
    }
    let files: Vec<(&str, String)> = [OBS_HOST, OBS_STACK, OBS_GUEST]
        .into_iter()
        .map(|rel| (rel, read_repo_file(rel)))
        .collect();

    // (2) no retired Loki/Alloy/Grafana/Tempo/Prometheus stack surfaces remain
    // in the live observability modules.
    let retired_stack_re = r"loki\.source|services\.(alloy|loki|tempo|prometheus|grafana)";
    for (rel, content) in &files {
        assert!(
            !any_line_matches(content, retired_stack_re),
            "loki-label-cardinality-eval: retired Loki/Alloy/Grafana stack reference \
             remains in live observability module {rel}"
        );
    }

    // (3) collect every distinct OTel resource-attribute key declared as
    // `key = "<name>"` across the three modules.
    let key_re = Regex::new(r#"key\s*=\s*"([^"]+)""#).expect("valid key regex");
    let mut observed_keys: BTreeSet<String> = BTreeSet::new();
    for (_, content) in &files {
        for line in content.lines() {
            for caps in key_re.captures_iter(line) {
                observed_keys.insert(caps[1].to_string());
            }
        }
    }
    assert!(
        !observed_keys.is_empty(),
        "loki-label-cardinality-eval: no OTel resource attribute keys found"
    );

    // (4) every observed key sits inside the bounded allowlist.
    for key in &observed_keys {
        assert!(
            allowed_resource_keys.contains(key.as_str()),
            "loki-label-cardinality-eval: resource attribute key '{key}' is outside the \
             bounded allowlist"
        );
    }

    // (5) every required identity key is present (exact match, mirroring the
    // bash `grep -Fxq`).
    for key in required_resource_keys {
        assert!(
            observed_keys.contains(key),
            "loki-label-cardinality-eval: required resource attribute key missing: {key}"
        );
    }

    // (6) no `key = "..."` value embeds a secret / high-cardinality /
    // value-leaking token.
    let forbidden_value_re = r#"key\s*=\s*"[^"]*(secret|password|token|private[_-]?key|argv|cmdline|command[_-]?line|stdout|stderr|/nix/store)[^"]*""#;
    for (rel, content) in &files {
        assert!(
            !any_line_matches(content, forbidden_value_re),
            "loki-label-cardinality-eval: forbidden sensitive/high-cardinality resource \
             attribute key found in {rel}"
        );
    }

    // (7) the collector configs carry the upsert action + identity tokens
    // somewhere across the three modules.
    for token in [
        r#"action = "upsert""#,
        "vm.name",
        "vm.env",
        "vm.role",
        "service.name",
        "source",
    ] {
        assert!(
            files.iter().any(|(_, content)| content.contains(token)),
            "loki-label-cardinality-eval: collector config missing {token}"
        );
    }

    // (8) StoreSync target_vm/target_env must stay in JSON content, never
    // promoted into a resource-attribute `key = ...`.
    for (rel, content) in &files {
        assert!(
            !any_line_matches(content, r"target_vm.*key\s*=")
                && !any_line_matches(content, r"target_env.*key\s*="),
            "loki-label-cardinality-eval: StoreSync target_vm/target_env promoted to a \
             resource attribute in {rel}"
        );
    }
}

// ===========================================================================
// Migrated from tests/tempo-budget-eval.sh (part 1 of 3): the stack backend +
// OTel collector pipeline shape.
//
// The retired Tempo retention gate now asserts the native SigNoz pipeline: the
// stack enables ClickHouse + a Keeper coordinator, declares the SigNoz units,
// carries the SigNoz collector processors/exporters, and routes ingress through
// per-source receiver pipelines.
// ===========================================================================
#[test]
fn tempo_stack_signoz_backend_and_collector() {
    assert!(
        repo_path_exists(OBS_STACK),
        "tempo-budget-eval: missing file: {OBS_STACK}"
    );
    let stack = read_repo_file(OBS_STACK);

    // stack enables ClickHouse + a Keeper coordinator.
    assert!(
        any_line_matches(&stack, r"services\.clickhouse"),
        "tempo-budget-eval: stack must enable ClickHouse for native SigNoz"
    );
    assert!(
        any_line_matches(&stack, r"systemd\.services\.clickhouse-keeper"),
        "tempo-budget-eval: stack must enable ZooKeeper or a ClickHouse Keeper equivalent"
    );

    // stack declares the SigNoz units.
    for unit in [
        "signoz",
        "signoz-otel-collector",
        "signoz-schema-migrate-sync",
    ] {
        assert!(
            stack.contains(&format!("systemd.services.{unit}")),
            "tempo-budget-eval: stack must declare {unit}.service"
        );
    }

    // collector config carries the SigNoz processor/exporter tokens.
    for token in [
        "signozspanmetrics/delta",
        "memory_limiter",
        "batch",
        "clickhousetraces",
        "clickhouselogsexporter",
        "signozclickhousemetrics",
        "metadataexporter",
    ] {
        assert!(
            stack.contains(token),
            "tempo-budget-eval: collector config missing {token}"
        );
    }

    // collector routes ingress through per-source receiver pipelines.
    for token in [
        "ingress.sources",
        "sourceReceivers",
        "sourceProcessors",
        "sourcePipelines",
        "nixling-otel-vsock-in-${sourceName}",
        "resource/${sourceName}",
    ] {
        assert!(
            stack.contains(token),
            "tempo-budget-eval: collector missing source-specific ingress token {token}"
        );
    }
}

// ===========================================================================
// ADR 0033 follow-up: host.name / deployment.environment identity scheme.
//
// The central collector stamps `host.name` with the per-source name (the
// host's name for host telemetry, the VM's name for workloads), and encodes
// `deployment.environment` as "<host>" for host telemetry and "<host>-<env>"
// for workload VMs (e.g. ddbus, ddbus-work, ddbus-personal).
// ===========================================================================
#[test]
fn signoz_resource_identity_scheme() {
    assert!(
        repo_path_exists(OBS_STACK),
        "identity-scheme: missing file: {OBS_STACK}"
    );
    let stack = read_repo_file(OBS_STACK);

    // host.name is the per-source name (host's name or the VM's name).
    assert!(
        any_line_matches(
            &stack,
            r#"key[[:space:]]*=[[:space:]]*"host.name";[[:space:]]*value[[:space:]]*=[[:space:]]*source\.vmName"#
        ),
        "identity-scheme: host.name must be the per-source name (source.vmName)"
    );

    // deployment.environment: "<host>" for host, "<host>-<env>" for VMs.
    for pat in [
        r#"if source\.role == "host""#,
        r#"then cfg\.hostName"#,
        r#"else "\$\{cfg\.hostName\}-\$\{source\.envName\}""#,
    ] {
        assert!(
            any_line_matches(&stack, pat),
            "identity-scheme: deployment.environment must be host-aware (missing /{pat}/)"
        );
    }
}

// ===========================================================================
// Migrated from tests/tempo-budget-eval.sh (part 2 of 3): the guest collector
// shape.
//
// The guest collector must resolve its `lib.mkIf` / `lib.optionalAttrs`
// conditionals before YAML serialization, keep self-metrics on a dedicated
// `resource/self` pipeline (never the workload resource processor), preserve
// the application `service.name` (i.e. the workload resource block must NOT
// rewrite it), and wire the journald receiver + access + severity + restart-safe
// cursor under `scrapeJournal`.
// ===========================================================================
#[test]
fn tempo_guest_collector_shape() {
    assert!(
        repo_path_exists(OBS_GUEST),
        "tempo-budget-eval: missing file: {OBS_GUEST}"
    );
    let guest = read_repo_file(OBS_GUEST);

    // node-metrics conditionals are resolved before YAML serialization.
    assert!(
        guest.contains("} // lib.optionalAttrs cfg.scrapeNodeMetrics {")
            && !guest.contains("hostmetrics = lib.mkIf cfg.scrapeNodeMetrics"),
        "tempo-budget-eval: guest collector must not serialize lib.mkIf wrappers into OTel YAML"
    );

    // self-metrics use a dedicated resource/self pipeline (not the workload
    // resource processor, and not a stringified `pipelines.metrics/self` key).
    assert!(
        guest.contains(r#""resource/self""#)
            && guest.contains(r#"pipelines."metrics/self""#)
            && !any_line_matches(&guest, r#""pipelines.metrics/self""#),
        "tempo-budget-eval: guest collector self-metrics must not share the workload \
         resource processor"
    );

    // workload resource processor block preserves application service.name
    // (i.e. the block exists and does NOT rewrite service.name).
    let block = sed_range(&guest, r"resource.attributes = \[", r"\];");
    assert!(
        !block.is_empty(),
        "tempo-budget-eval: guest workload resource processor block was not found"
    );
    assert!(
        !block.contains("service.name"),
        "tempo-budget-eval: guest workload resource processor must preserve application \
         service.name"
    );

    // journald receiver is wired into the logs pipeline under scrapeJournal.
    assert!(
        guest.contains("lib.optionalAttrs cfg.scrapeJournal {")
            && guest.contains("journald = {")
            && guest.contains(r#"lib.optional cfg.scrapeJournal "journald""#),
        "tempo-budget-eval: guest collector must add the journald receiver to the logs \
         pipeline under scrapeJournal"
    );

    // journald collection grants journal read access + journalctl on PATH.
    assert!(
        guest.contains(r#"extraGroups = lib.optional cfg.scrapeJournal "systemd-journal""#)
            && guest.contains("path = lib.optional cfg.scrapeJournal pkgs.systemd"),
        "tempo-budget-eval: guest journald collection must grant systemd-journal access \
         and journalctl on PATH"
    );

    // journald logs carry severity + persist a restart-safe read cursor.
    for token in [
        r#"type = "severity_parser""#,
        r#"parse_from = "body.PRIORITY""#,
        r#"error = "3""#,
        r#"storage = "file_storage/journald""#,
        r#""file_storage/journald" = {"#,
        r#"directory = "/var/lib/otel/journald""#,
        "create_directory = true",
        r#"extensions = lib.optional cfg.scrapeJournal "file_storage/journald""#,
    ] {
        assert!(
            guest.contains(token),
            "tempo-budget-eval: guest journald receiver must map PRIORITY->severity and \
             bind+enable a file_storage cursor (missing: {token})"
        );
    }
}

// ===========================================================================
// Migrated from tests/tempo-budget-eval.sh (part 3 of 3): collector
// self-telemetry, ClickHouse/secrets security posture, host-activation vsock
// ACL wiring, loopback bind posture, retired-backend denylist, host options,
// and the ADR Spec-corrections record.
// ===========================================================================
#[test]
fn tempo_security_observability_and_retired_backends() {
    let host_opts_rel = "nixos-modules/options-observability.nix";
    let adr_rel = "docs/adr/0026-native-signoz-observability.md";
    let secrets_rel = "nixos-modules/observability-host-secrets.nix";
    let host_activation_rel = "nixos-modules/host-activation.nix";

    for rel in [OBS_STACK, OBS_HOST, OBS_GUEST, host_opts_rel, adr_rel] {
        assert!(
            repo_path_exists(rel),
            "tempo-budget-eval: missing file: {rel}"
        );
    }

    let stack = read_repo_file(OBS_STACK);
    let host = read_repo_file(OBS_HOST);
    let guest = read_repo_file(OBS_GUEST);
    let host_opts = read_repo_file(host_opts_rel);
    let adr = read_repo_file(adr_rel);
    let secrets = read_repo_file(secrets_rel);
    let host_activation = read_repo_file(host_activation_rel);

    // collector self-telemetry tokens are present across stack/host/guest.
    for token in [
        "prometheus/self",
        "nixling-host-otel-collector",
        "nixling-guest-otel-collector",
        "metrics.readers",
    ] {
        assert!(
            stack.contains(token) || host.contains(token) || guest.contains(token),
            "tempo-budget-eval: collector self-telemetry token missing: {token}"
        );
    }

    // collector pipelines are source-specific, not a shared otlp receiver.
    assert!(
        stack.contains("pipelines = sourcePipelines")
            && !stack.contains(r#"receivers = [ "otlp" ]"#),
        "tempo-budget-eval: collector must route through source-specific receiver pipelines"
    );

    // ClickHouse passwords are URL-encoded before DSN interpolation.
    assert!(
        stack.contains("@uri")
            && !stack.contains("password=$pw\"")
            && !stack.contains("password=$SIGNOZ_CLICKHOUSE_PASSWORD"),
        "tempo-budget-eval: ClickHouse passwords embedded in DSN query strings must be \
         URL-encoded"
    );

    // ClickHouse default user keeps a local-only auth method.
    assert!(
        stack.contains(r#"<password remove="1"/>"#)
            && stack.contains("<no_password/>")
            && stack.contains("<ip>127.0.0.1</ip>"),
        "tempo-budget-eval: ClickHouse default user must not be left without an auth method"
    );

    // SigNoz OTel collector runs the static nixling config (no OpAMP manager).
    assert!(
        !stack.contains("--manager-config") && !stack.contains("conf/opamp.yaml"),
        "tempo-budget-eval: SigNoz OTel collector must not enable OpAMP manager mode for \
         static nixling receivers"
    );

    // observability secrets are readable through the read-only virtiofs share
    // but protected by a host parent dir.
    assert!(
        secrets.contains(r#"chmod 0444 "$file""#) && secrets.contains("root:root 0700"),
        "tempo-budget-eval: observability secrets must be readable through the read-only \
         virtiofs share"
    );

    // workload OTLP relays inherit/connect to the obs VM vsock socket ACLs.
    assert!(
        any_line_matches(&host_activation, r"otel_obs_connect_uids=.*vsock-relay")
            && host_activation.contains(r#"setfacl -d -m "u:$obs_uid:rw" "$obs_state_dir""#)
            && host_activation.contains(r#"setfacl -m "u:$obs_uid:rw,m::rw" "$obs_vsock""#),
        "tempo-budget-eval: observed workload relay UIDs must get effective obs vsock socket ACLs"
    );

    // backend binds are loopback-oriented and only the SigNoz UI port is opened.
    assert!(
        stack.contains("127.0.0.1")
            && stack.contains("networking.firewall.allowedTCPPorts = [ cfg.signoz.listenPort ]"),
        "tempo-budget-eval: stack must keep backend ports loopback-only and open only the \
         SigNoz UI port"
    );

    // no retired backend is declared in the stack.
    for retired in [
        r"services\.grafana",
        r"services\.prometheus",
        r"services\.loki",
        r"services\.tempo",
        r"services\.alloy",
    ] {
        assert!(
            !any_line_matches(&stack, retired),
            "tempo-budget-eval: stack still declares retired backend matching /{retired}/"
        );
    }

    // host options expose the SigNoz option surface.
    for option in [
        "signoz = {",
        "listenPort",
        "otlpGrpcPort",
        "otlpHttpPort",
        "adminEmail",
    ] {
        assert!(
            host_opts.contains(option),
            "tempo-budget-eval: host options missing {option}"
        );
    }

    // ADR records the manifestVersion Spec corrections.
    assert!(
        adr.contains("Spec corrections") && adr.contains("manifestVersion"),
        "tempo-budget-eval: ADR must record manifestVersion Spec corrections"
    );
}

/// Faithful port of `sed -n '/<start>/,/<end>/p'`: collect every line from a
/// line matching `start_pat` through the next line matching `end_pat`
/// (inclusive), re-entering the range if `start_pat` matches again after a
/// close. Returns the joined block (empty when `start_pat` never matches).
fn sed_range(content: &str, start_pat: &str, end_pat: &str) -> String {
    let start_re = Regex::new(start_pat).expect("valid sed start regex");
    let end_re = Regex::new(end_pat).expect("valid sed end regex");
    let mut out: Vec<&str> = Vec::new();
    let mut active = false;
    for line in content.lines() {
        if active {
            out.push(line);
            if end_re.is_match(line) {
                active = false;
            }
        } else if start_re.is_match(line) {
            active = true;
            out.push(line);
        }
    }
    out.join("\n")
}

// ===========================================================================
// Migrated from tests/wave-evidence-schema-eval.sh.
//
// Asserts every readiness wave declared in
// nixos-modules/options-daemon.nix:readinessWaveSpecs has a matching per-wave
// inventory row in docs/reference/wave-evidence-schema.md, and that the JSON
// Schema companion declares exactly the three required fields the validator
// enforces (wave/timestamp/operatorSignature).
// ===========================================================================
#[test]
fn wave_evidence_schema_cross_check() {
    let options_rel = "nixos-modules/options-daemon.nix";
    let doc_rel = "docs/reference/wave-evidence-schema.md";
    let schema_rel = "docs/reference/wave-evidence-schema.json";

    for rel in [options_rel, doc_rel, schema_rel] {
        assert!(
            repo_path_exists(rel),
            "wave-evidence-schema-eval: missing or unreadable: {rel}"
        );
    }

    let options = read_repo_file(options_rel);
    let doc = read_repo_file(doc_rel);
    let schema = read_repo_file(schema_rel);

    // Extract wave keys from the readinessWaveSpecs block — a faithful port of
    // the bash awk parser: enter at `readinessWaveSpecs = {`, exit at a line of
    // exactly two leading spaces + `};`, and capture 4-space-indented
    // `<name> = {` rows in between.
    let enter_re = Regex::new(r"readinessWaveSpecs = \{").expect("valid enter regex");
    let exit_re = Regex::new(r"^  \};").expect("valid exit regex");
    let key_re =
        Regex::new(r"^    ([A-Za-z][A-Za-z0-9_]*)\s*=\s*\{").expect("valid wave-key regex");

    let mut in_block = false;
    let mut waves: Vec<String> = Vec::new();
    for line in options.lines() {
        if enter_re.is_match(line) {
            in_block = true;
            continue;
        }
        if in_block && exit_re.is_match(line) {
            in_block = false;
        }
        if in_block {
            if let Some(caps) = key_re.captures(line) {
                waves.push(caps[1].to_string());
            }
        }
    }

    assert!(
        !waves.is_empty(),
        "wave-evidence-schema-eval: failed to parse any waves from {options_rel} \
         (expected readinessWaveSpecs block with 4-space-indented '<name> = {{' rows)"
    );

    // Every declared wave has a per-wave inventory row of the form
    // `| `<wave>` |` in the doc.
    let mut missing: Vec<String> = Vec::new();
    for wave in &waves {
        let row_re = format!(r"^\|\s+`{}`\s+\|", regex::escape(wave));
        if !any_line_matches(&doc, &row_re) {
            missing.push(wave.clone());
        }
    }
    assert!(
        missing.is_empty(),
        "wave-evidence-schema-eval: {} wave(s) declared in {options_rel} have no per-wave \
         inventory row in {doc_rel}: {}",
        missing.len(),
        missing.join(", ")
    );

    // The JSON Schema companion declares exactly the three required fields the
    // validator enforces (sorted), mirroring the bash `jq -r '.required | sort
    // | join(",")'` check.
    let schema_value: serde_json::Value = serde_json::from_str(&schema).unwrap_or_else(|err| {
        panic!("wave-evidence-schema-eval: {schema_rel} is not valid JSON: {err}")
    });
    let required = schema_value
        .get("required")
        .and_then(|v| v.as_array())
        .unwrap_or_else(|| {
            panic!("wave-evidence-schema-eval: {schema_rel} has no .required array")
        });
    let mut required_sorted: Vec<String> = required
        .iter()
        .map(|v| {
            v.as_str()
                .unwrap_or_else(|| {
                    panic!(
                        "wave-evidence-schema-eval: {schema_rel} .required has a non-string entry"
                    )
                })
                .to_string()
        })
        .collect();
    required_sorted.sort();
    assert_eq!(
        required_sorted,
        vec![
            "operatorSignature".to_string(),
            "timestamp".to_string(),
            "wave".to_string(),
        ],
        "wave-evidence-schema-eval: {schema_rel} .required must be \
         [operatorSignature, timestamp, wave]"
    );
}
