use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
};

use nixling_contract_tests::{read_repo_file, repo_root};
use nixling_core::{
    processes::ProcessesJson,
    storage::{StorageJson, StoragePathKind},
    sync::{LockKind, SyncJson},
};
use regex::Regex;

#[test]
fn storage_and_sync_emitters_are_wired_into_private_bundle() {
    let default_nix = read_repo_file("nixos-modules/default.nix");
    assert!(
        default_nix.contains("./storage-json.nix"),
        "default.nix must import storage-json.nix"
    );
    assert!(
        default_nix.contains("./sync-json.nix"),
        "default.nix must import sync-json.nix"
    );

    let bundle_nix = read_repo_file("nixos-modules/bundle.nix");
    for needle in [
        "storagePath = \"/etc/nixling/storage.json\";",
        "syncPath = \"/etc/nixling/sync.json\";",
        "key = \"/etc/nixling/storage.json\";",
        "key = \"/etc/nixling/sync.json\";",
    ] {
        assert!(
            bundle_nix.contains(needle),
            "bundle.nix missing storage/sync wiring: {needle}"
        );
    }

    let bundle_doc = read_repo_file("docs/reference/manifest-bundle.md");
    assert!(bundle_doc.contains("`storage.json`"));
    assert!(bundle_doc.contains("`sync.json`"));
}

#[test]
fn storage_and_sync_schemas_are_committed_and_closed() {
    let storage_schema = read_repo_file("docs/reference/schemas/v2/storage.json");
    let sync_schema = read_repo_file("docs/reference/schemas/v2/sync.json");
    for (name, schema) in [
        ("storage.json", storage_schema.as_str()),
        ("sync.json", sync_schema.as_str()),
    ] {
        assert!(
            schema.contains("\"additionalProperties\": false"),
            "{name} must deny unknown fields"
        );
    }
    assert!(storage_schema.contains("\"adoption-quarantined\""));
    assert!(storage_schema.contains("\"tamper-evident-segmented\""));
    assert!(sync_schema.contains("\"ofd\""));
    assert!(sync_schema.contains("\"scm-rights\""));
    assert!(sync_schema.contains("\"explicit-fd-mapping\""));
}

#[test]
fn storage_lifecycle_report_schema_and_reference_are_committed() {
    let schema = read_repo_file("docs/reference/schemas/v2/storage-lifecycle-report.json");
    let reference = read_repo_file("docs/reference/storage-lifecycle-report.md");
    let xtask = read_repo_file("packages/xtask/src/main.rs");
    let schema: serde_json::Value =
        serde_json::from_str(&schema).expect("storage lifecycle report schema parses as JSON");

    assert_eq!(schema["title"], "StorageLifecycleReport");
    assert!(schema["properties"].get("schemaVersion").is_some());

    let issue_variants = schema["definitions"]["StorageLifecycleIssue"]["oneOf"]
        .as_array()
        .expect("StorageLifecycleIssue oneOf variants");
    let legacy_variant = issue_variants
        .iter()
        .find(|variant| {
            variant["properties"]["kind"]["enum"]
                .as_array()
                .is_some_and(|values| {
                    values
                        .iter()
                        .any(|value| value == "legacy-bundle-contracts-unavailable")
                })
        })
        .expect("legacy bundle issue variant in schema");
    assert!(legacy_variant["properties"].get("bundleVersion").is_some());
    assert!(legacy_variant["properties"].get("bundle_version").is_none());

    for kind in ["missing-restart-policy", "adoptable-missing-cgroup-leaf"] {
        let role_variant = issue_variants
            .iter()
            .find(|variant| {
                variant["properties"]["kind"]["enum"]
                    .as_array()
                    .is_some_and(|values| values.iter().any(|value| value == kind))
            })
            .unwrap_or_else(|| panic!("{kind} issue variant in schema"));
        assert!(role_variant["properties"].get("roleId").is_some());
        assert!(role_variant["properties"].get("role_id").is_none());
    }

    assert!(reference.contains("/var/lib/nixling/daemon-state/storage-lifecycle-report.json"));
    assert!(reference.contains("./schemas/v2/storage-lifecycle-report.json"));
    assert!(xtask.contains("\"storage-lifecycle-report.json\""));
}

#[test]
fn rendered_storage_contract_covers_process_writable_paths_when_fixture_available() {
    let Some(dir) = env::var_os("NL_FIXTURES").map(PathBuf::from) else {
        eprintln!("  (skipping rendered storage/sync contract check; NL_FIXTURES unset)");
        return;
    };
    let storage: StorageJson = read_json(&dir, "storage.json");
    let sync: SyncJson = read_json(&dir, "sync.json");
    let processes: ProcessesJson = read_json(&dir, "processes.json");

    storage
        .validate_unique_ids()
        .expect("rendered storage contract must have unique ids");
    sync.validate_lock_order()
        .expect("rendered sync contract must have valid lock order");

    let storage_paths: BTreeSet<&str> = storage
        .paths
        .iter()
        .map(|path| path.path_template.as_str())
        .collect();
    for dag in &processes.vms {
        for node in &dag.nodes {
            let restart = storage.restart_policies.iter().find(|policy| {
                policy.vm.as_str() == dag.vm && policy.role_id.as_str() == node.id.0
            });
            assert!(
                restart.is_some(),
                "missing restart policy for {}:{}",
                dag.vm,
                node.id.0
            );
            for writable in &node.profile.mount_policy.writable_paths {
                assert!(
                    storage_paths.contains(writable.path.as_str()),
                    "storage.json missing writable path for {}:{} -> {}",
                    dag.vm,
                    node.id.0,
                    writable.path
                );
            }
        }
    }

    assert!(
        storage
            .paths
            .iter()
            .any(|path| path.path_template.as_str() == "/etc/nixling/storage.json"),
        "storage.json must describe itself as a private bundle artifact"
    );
    assert!(
        storage
            .paths
            .iter()
            .any(|path| path.path_template.as_str() == "/etc/nixling/sync.json"),
        "storage.json must describe sync.json as a private bundle artifact"
    );
    assert!(
        storage
            .paths
            .iter()
            .any(|path| path.kind == StoragePathKind::UnixSocket),
        "storage.json should include role/readiness Unix socket paths"
    );
    assert!(
        storage
            .paths
            .iter()
            .all(|path| { !path.path_template.as_str().starts_with("/run/udev/") }),
        "storage.json must not claim broker-owned storage authority over foreign /run/udev state"
    );
    assert!(
        sync.locks
            .iter()
            .any(|lock| lock.kind == LockKind::Ofd && lock.cloexec_required),
        "sync.json must include at least one O_CLOEXEC OFD lock"
    );
}

fn read_json<T: serde::de::DeserializeOwned>(dir: &std::path::Path, name: &str) -> T {
    let path = dir.join(name);
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|err| {
        panic!(
            "failed to parse {} as {}: {err}",
            path.display(),
            std::any::type_name::<T>()
        )
    })
}

#[test]
fn broker_storage_and_sync_requests_stay_opaque_only() {
    let broker_wire = read_repo_file("packages/nixling-ipc/src/broker_wire.rs");
    let request_re =
        Regex::new(r"(?s)pub struct (\w+Request) \{(?P<body>.*?)\n\}").expect("request regex");
    let field_re =
        Regex::new(r"(?m)^\s*(?:pub(?:\([^)]+\))?\s+)?([A-Za-z0-9_]+)\s*:").expect("field regex");
    let forbidden_fields = BTreeSet::from([
        "acl",
        "cleanup",
        "cleanup_policy",
        "cmd",
        "command",
        "fd_passing_policy",
        "group",
        "mode",
        "owner",
        "path",
        "path_template",
        "repair_policy",
    ]);

    let allowed = BTreeSet::from([
        ("OpenCgroupDirRequest", "path_class"),
        ("PrepareDirRequest", "path_class"),
        ("RunActivationRequest", "mode"),
    ]);
    let mut violations = Vec::new();
    for cap in request_re.captures_iter(&broker_wire) {
        let request = cap.get(1).expect("request name").as_str();
        let body = cap.name("body").expect("request body").as_str();
        for cap in field_re.captures_iter(body) {
            let field = &cap[1];
            let normalized = field.to_ascii_lowercase();
            if forbidden_fields.contains(normalized.as_str())
                && !allowed.contains(&(request, field))
            {
                violations.push(format!("{request}.{field}"));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "broker IPC storage/sync policy violation: mutating broker requests must carry opaque \
         ids/classes only, not caller-supplied raw storage paths, owners, ACLs, cleanup commands, \
         or lock policy. Add/extend storage.json or sync.json rows and resolve them in the broker \
         instead. Violations: {violations:?}"
    );
}

#[test]
fn tmpfiles_host_mutable_paths_are_covered_by_storage_contract_roots() {
    let tmpfiles_paths = literal_nixling_tmpfiles_paths();
    assert!(
        !tmpfiles_paths.is_empty(),
        "policy-paths: static tmpfiles inventory found no literal nixling tmpfiles paths"
    );

    let covered_roots = rendered_storage_roots_or_static_fallback();
    let missing: Vec<_> = tmpfiles_paths
        .iter()
        .filter(|path| {
            !covered_roots
                .iter()
                .any(|root| path == &root.as_str() || path.starts_with(&format!("{root}/")))
        })
        .cloned()
        .collect();

    assert!(
        missing.is_empty(),
        "policy-paths: tmpfiles host-mutable paths are not covered by storage.json roots/paths: \
         {missing:?}. This check inventories literal systemd.tmpfiles.rules from nixos-modules \
         (docs/ and tests/ are excluded); interpolated/evaluated rules are validated when \
         NL_FIXTURES provides rendered storage.json."
    );
}

#[test]
fn host_mutation_sources_are_registered_with_storage_or_sync_policy() {
    let discovered = host_mutation_sources();
    let registered = registered_host_mutation_sources();
    let unregistered: Vec<_> = discovered
        .iter()
        .filter(|path| !registered.contains_key(path.as_str()))
        .cloned()
        .collect();

    assert!(
        unregistered.is_empty(),
        "policy-paths: host-mutable path/lock mutation contexts must be registered with a \
         storage.json/sync.json contract row and one repair owner. This scan matches mutation \
         contexts (tmpfiles, activation snippets, mkdir/chmod/chown/setfacl, fs::write, \
         File::create/OpenOptions, create_dir*) near nixling host paths/locks; docs/ and tests/ \
         are excluded so prose/fixtures do not satisfy or fail the gate. Add contract coverage \
         before adding new mutation sources. Unregistered sources: {unregistered:?}"
    );

    let stale: Vec<_> = registered
        .keys()
        .filter(|path| !discovered.contains(**path))
        .copied()
        .collect();
    assert!(
        stale.is_empty(),
        "policy-paths: registered host mutation sources must remain live scan matches, so the \
         storage/sync ownership allowlist cannot accumulate stale entries. Stale registrations: \
         {stale:?}"
    );

    assert!(
        read_repo_file("AGENTS.md").contains("single repair owner"),
        "AGENTS.md must document the durable single repair owner rule for host-mutable paths/locks"
    );
}

fn literal_nixling_tmpfiles_paths() -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    let rule_re = Regex::new(
        r#"(?m)^\s*"?[a-zA-Z][!+=~^-]*\s+((?:/var/lib|/run|/etc)/nixling(?:/[^ "'\t\n]*)?)"#,
    )
    .expect("tmpfiles path regex");
    for path in collect_repo_files("nixos-modules", "nix") {
        let text = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("policy-paths: cannot read {}: {err}", path.display()));
        if !text.contains("tmpfiles.rules") {
            continue;
        }
        let code = stripped_code_text(&path, &text);
        for cap in rule_re.captures_iter(&code) {
            paths.insert(cap[1].trim_end_matches('/').to_owned());
        }
    }
    paths
}

fn rendered_storage_roots_or_static_fallback() -> BTreeSet<String> {
    if let Some(dir) = env::var_os("NL_FIXTURES").map(PathBuf::from) {
        let storage: StorageJson = read_json(&dir, "storage.json");
        let mut roots: BTreeSet<String> = storage
            .roots
            .iter()
            .map(|root| root.path.as_str().to_owned())
            .collect();
        roots.extend(
            storage
                .paths
                .iter()
                .map(|path| path.path_template.as_str().to_owned()),
        );
        return roots;
    }

    eprintln!(
        "  (policy-paths: NL_FIXTURES unset; tmpfiles coverage uses the narrow static \
         fallback roots from storage-json.nix rather than fully evaluated rules)"
    );
    let storage_nix = read_repo_file("nixos-modules/storage-json.nix");
    assert!(
        storage_nix.contains("path = toString cfg.site.stateDir;")
            && storage_nix.contains("path = \"/run/nixling\";")
            && storage_nix.contains("path = \"/etc/nixling\";"),
        "policy-paths: storage-json.nix must declare state, runtime, and /etc nixling roots"
    );
    BTreeSet::from([
        "/etc/nixling".to_owned(),
        "/run/nixling".to_owned(),
        "/var/lib/nixling".to_owned(),
    ])
}

fn host_mutation_sources() -> BTreeSet<String> {
    let mutation_re = Regex::new(
        r"(fs::(?:write|copy|rename|remove_(?:file|dir|dir_all))|std::os::unix::fs::symlink|symlink|hard_link|File::create|OpenOptions::new|create_dir(?:_all)?|set_permissions|chmod|chown|setfacl|systemd\.tmpfiles\.rules|tmpfiles\.rules|activationScripts|install\s+-[dm]|mkdir\s+-p)",
    )
    .expect("mutation context regex");
    let surface_re = Regex::new(
        r"(/var/lib/nixling(?:/|\b)|/run/nixling(?:/|\b)|/etc/nixling(?:/|\b)|cfg\.site\.stateDir|cfg\.store\.stateDir|\.lock|locks/)",
    )
    .expect("surface regex");

    let mut found = BTreeSet::new();
    for rel in [
        ("nixos-modules", "nix"),
        ("packages/nixling-priv-broker/src", "rs"),
        ("packages/nixlingd/src", "rs"),
        ("packages/nixling-host/src", "rs"),
        ("packages/nixling-host-activation-helper/src", "rs"),
    ] {
        for path in collect_repo_files(rel.0, rel.1) {
            let rel_path = path
                .strip_prefix(repo_root())
                .expect("repo-relative path")
                .to_string_lossy()
                .into_owned();
            if rel_path.starts_with("nixos-modules/options-")
                || rel_path == "nixos-modules/processes-json.nix"
                || rel_path == "nixos-modules/storage-json.nix"
                || rel_path == "nixos-modules/sync-json.nix"
            {
                continue;
            }
            let text = fs::read_to_string(&path).unwrap_or_else(|err| {
                panic!(
                    "policy-paths: cannot read mutation source {}: {err}",
                    path.display()
                )
            });
            let lines: Vec<String> = text
                .lines()
                .map(|line| stripped_code_line(&path, line).to_owned())
                .collect();
            for (idx, line) in lines.iter().enumerate() {
                if !mutation_re.is_match(line) {
                    continue;
                }
                let start = idx.saturating_sub(4);
                let end = (idx + 5).min(lines.len());
                let window = lines[start..end].join("\n");
                if surface_re.is_match(&window) {
                    found.insert(rel_path.clone());
                    break;
                }
            }
        }
    }
    found
}

fn stripped_code_text(path: &Path, text: &str) -> String {
    text.lines()
        .map(|line| stripped_code_line(path, line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn stripped_code_line<'a>(path: &Path, line: &'a str) -> &'a str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("nix") => line.split('#').next().unwrap_or(line),
        Some("rs") => line.split("//").next().unwrap_or(line),
        _ => line,
    }
}

fn registered_host_mutation_sources() -> BTreeMap<&'static str, &'static str> {
    BTreeMap::from([
        (
            "nixos-modules/components/audio/host.nix",
            "storage root:path:run-root",
        ),
        (
            "nixos-modules/bundle.nix",
            "storage paths:private bundle artifacts",
        ),
        (
            "nixos-modules/components/observability/guest.nix",
            "storage root:path:run-root",
        ),
        (
            "nixos-modules/components/observability/host.nix",
            "storage root:path:run-root",
        ),
        (
            "nixos-modules/gateway-vm.nix",
            "storage root:path:state-root",
        ),
        (
            "nixos-modules/guest-control.nix",
            "storage paths:nodeWritablePaths/readinessSocketPaths",
        ),
        (
            "nixos-modules/host-activation.nix",
            "storage roots:path:state-root,path:run-root",
        ),
        (
            "nixos-modules/host-broker.nix",
            "storage root:path:state-root",
        ),
        (
            "nixos-modules/host-ssh-host-keys.nix",
            "storage root:path:state-root",
        ),
        (
            "nixos-modules/host.nix",
            "storage roots:path:state-root,path:run-root",
        ),
        (
            "nixos-modules/observability-host-secrets.nix",
            "storage root:path:state-root",
        ),
        ("nixos-modules/store.nix", "storage root:path:state-root"),
        (
            "packages/nixling-host-activation-helper/src/main.rs",
            "storage root:path:state-root",
        ),
        (
            "packages/nixling-host/src/cgroup.rs",
            "sync lock:cgroup-delegation",
        ),
        (
            "packages/nixling-host/src/ownership_matrix.rs",
            "storage root:path:state-root",
        ),
        (
            "packages/nixling-priv-broker/src/audit.rs",
            "storage root:path:state-root audit log subtree",
        ),
        (
            "packages/nixling-priv-broker/src/live_handlers.rs",
            "broker resolves storage/sync opaque ids",
        ),
        (
            "packages/nixling-priv-broker/src/ops/exec_reconcile.rs",
            "storage roots:path:etc-root,path:state-root",
        ),
        (
            "packages/nixling-priv-broker/src/ops/media.rs",
            "storage paths:qemu-media registry/runtime index",
        ),
        (
            "packages/nixling-priv-broker/src/ops/store_sync.rs",
            "storage paths:store-view hardlink farm",
        ),
        (
            "packages/nixling-priv-broker/src/runtime.rs",
            "storage root:path:run-root",
        ),
        ("packages/nixlingd/src/lib.rs", "storage root:path:run-root"),
        (
            "packages/nixlingd/src/typed_error.rs",
            "storage degraded-state reports",
        ),
    ])
}

fn collect_repo_files(rel_dir: &str, extension: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let root = repo_root().join(rel_dir);
    collect_repo_files_inner(&root, extension, &mut out);
    out.sort();
    out
}

fn collect_repo_files_inner(dir: &Path, extension: &str, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("policy-paths: cannot read {}: {err}", dir.display()))
    {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if matches!(name, "target" | "tests" | "fixtures" | "docs") {
                continue;
            }
            collect_repo_files_inner(&path, extension, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some(extension) {
            out.push(path);
        }
    }
}
