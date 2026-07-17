//! Realm-native state, storage projection, and host OTLP source policies.

use d2b_contract_tests::{read_repo_file, repo_path_exists};
use regex::Regex;

fn require(path: &str) -> String {
    assert!(
        repo_path_exists(path),
        "missing required policy source: {path}"
    );
    read_repo_file(path)
}

fn code_only(content: &str) -> String {
    let trailing = Regex::new(r"[[:space:]]#.*$").expect("trailing comment regex");
    let full_comment = Regex::new(r"^[[:space:]]*#").expect("full comment regex");
    content
        .lines()
        .filter_map(|line| {
            let stripped = trailing.replace(line, "").into_owned();
            (!full_comment.is_match(&stripped)).then_some(stripped)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_contains_all(path: &str, source: &str, needles: &[&str]) {
    for needle in needles {
        assert!(source.contains(needle), "{path} missing `{needle}`");
    }
}

#[test]
fn lifecycle_group_is_local_root_only() {
    let users = code_only(&require("nixos-modules/host-users.nix"));
    let daemon = code_only(&require("nixos-modules/host-daemon.nix"));

    assert_contains_all(
        "nixos-modules/host-users.nix",
        &users,
        &[
            "d2b = { };",
            "lib.genAttrs (cfg.site.launcherUsers or [ ])",
            "extraGroups = [ \"d2b\" ];",
        ],
    );
    assert_contains_all(
        "nixos-modules/host-daemon.nix",
        &daemon,
        &[
            "publicSocketGroup = localRoot.publicGroup;",
            "SocketGroup = publicSocketPrincipal.group;",
            "extraGroups = [ localRoot.publicGroup ];",
        ],
    );
    assert!(
        !users.contains("d2b-launcher") && !users.contains("d2b-launchers"),
        "legacy launcher-group tombstones must not survive the destructive cutover"
    );
}

#[test]
fn activation_storage_repair_is_absent() {
    let activation = code_only(&require("nixos-modules/host-activation.nix"));
    let keys = code_only(&require("nixos-modules/host-keys.nix"));
    let store = code_only(&require("nixos-modules/store.nix"));

    for (path, source) in [
        ("nixos-modules/host-activation.nix", activation.as_str()),
        ("nixos-modules/host-keys.nix", keys.as_str()),
        ("nixos-modules/store.nix", store.as_str()),
    ] {
        for forbidden in [
            "ssh-keygen",
            "setfacl",
            "chown ",
            "chmod ",
            "mkdir ",
            "install -d",
            "/r/",
            "/w/",
        ] {
            assert!(
                !source.contains(forbidden),
                "{path} must not repair broker-owned realm/workload storage: {forbidden}"
            );
        }
    }
    assert!(
        !activation.contains("system.activationScripts"),
        "host activation must not own realm/workload repair scripts"
    );
    assert!(
        keys.contains("system.activationScripts.d2bGenerateKeys")
            && keys.contains("lib.stringAfter [ \"users\" ]"),
        "the compatibility key hook must remain an explicit no-op ordering point"
    );
}

#[test]
fn fixed_storage_anchors_are_activation_only() {
    let activation = code_only(&require("nixos-modules/host-activation.nix"));
    assert_contains_all(
        "nixos-modules/host-activation.nix",
        &activation,
        &[
            "\"d /var/lib/d2b 0750 root d2bd -\"",
            "\"z /var/lib/d2b 0750 root d2bd -\"",
            "\"d /var/cache/d2b 0750 root d2bd -\"",
            "\"z /var/cache/d2b 0750 root d2bd -\"",
        ],
    );
    for forbidden in ["/var/lib/d2b/r/", "/var/cache/d2b/r/", "/run/d2b/r/"] {
        assert!(
            !activation.contains(forbidden),
            "activation crossed a fixed anchor into broker-owned storage: {forbidden}"
        );
    }

    let rows = code_only(&require("nixos-modules/realm-storage-rows.nix"));
    assert_contains_all(
        "nixos-modules/realm-storage-rows.nix",
        &rows,
        &[
            "creator = brokerActor realmId;",
            "recursive = false;",
            "repairPolicy ? \"broker-reconcile\"",
            "repairPolicy = \"broker-fail-closed\";",
        ],
    );
}

#[test]
fn store_sync_export() {
    let host = code_only(&require("nixos-modules/components/observability/host.nix"));
    let rows = code_only(&require("nixos-modules/realm-observability-rows.nix"));

    assert_contains_all(
        "nixos-modules/components/observability/host.nix",
        &host,
        &[
            "row.kind == \"bounded-projection\"",
            "rows.paths)).path;",
            "storeSyncExportGlob = \"${storeSyncExportDir}/store-sync-*.jsonl\";",
            "\"filelog/store_sync_audit\"",
            "type = \"json_parser\";",
            "\"logs/store_sync_audit\"",
            "exporters = [ \"otlp\" ];",
            "{ key = \"service.name\"; value = \"d2b-store-sync\";",
            "{ key = \"source\"; value = \"store-sync-audit\";",
        ],
    );
    assert_contains_all(
        "nixos-modules/realm-observability-rows.nix",
        &rows,
        &[
            "\"path:observability-store-sync-projection:${workloadId}\"",
            "\"${auditRoot}/projections/store-sync\"",
            "creator = \"realm-broker\";",
            "repairOwner = \"realm-broker\";",
            "readers = [ \"d2b-host-otel-collector\" ];",
        ],
    );
    for forbidden in [
        "audit/broker",
        "broker-*",
        "priv.sock",
        "target_vm",
        "target_env",
        "$state_dir",
        "$obs_dir",
        "/observability/store-sync",
        "export_dir=",
        "setfacl -m u:d2b-host-otel-collector",
    ] {
        assert!(
            !host.contains(forbidden),
            "host collector retains a non-canonical StoreSync surface: {forbidden}"
        );
    }
}

#[test]
fn host_otlp_ingest_socket_isolation() {
    let host = code_only(&require("nixos-modules/components/observability/host.nix"));
    let rows = code_only(&require("nixos-modules/realm-observability-rows.nix"));

    assert_contains_all(
        "nixos-modules/components/observability/host.nix",
        &host,
        &[
            "hostEgressSocket = rows.endpoints.hostEgress.path;",
            "hostOtlpSocket = rows.endpoints.hostIngest.path;",
            "otelIngestDir = builtins.dirOf hostOtlpSocket;",
            "endpoint = \"unix://${hostEgressSocket}\";",
            "endpoint = hostOtlpSocket;",
            "transport = \"unix\";",
            "rm -f ${hostOtlpSocket}",
            "ReadWritePaths = [ otelIngestDir ];",
            "UMask = ingestUmask;",
        ],
    );
    assert_contains_all(
        "nixos-modules/realm-observability-rows.nix",
        &rows,
        &[
            "bridgeRoleRoot = \"${runRoot}/roles/${bridgeRoleId}\";",
            "path = \"${bridgeRoleRoot}/host-egress.sock\";",
            "path = \"${runRoot}/sockets/ingest/host-otlp.sock\";",
        ],
    );
    assert!(
        !host.contains("ReadWritePaths = [ otelRuntimeDir ];"),
        "collector bind authority must not reach the shared egress directory"
    );
}

#[test]
fn host_journald_cursor_provisioned() {
    let host = code_only(&require("nixos-modules/components/observability/host.nix"));
    assert_contains_all(
        "nixos-modules/components/observability/host.nix",
        &host,
        &[
            "install -d -m 0700 -o d2b-host-otel-collector -g d2b-host-otel-collector ${journaldStorageDir}",
            "create_directory = false;",
        ],
    );
    assert!(
        !host.contains("create_directory = true;"),
        "journald cursor storage must be provisioned before collector startup"
    );
}
