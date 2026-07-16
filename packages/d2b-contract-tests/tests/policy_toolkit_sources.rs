//! Exact source and ownership policy for the client and provider toolkit
//! distributions.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

use d2b_contract_tests::{read_repo_file, repo_root};
use regex::Regex;
use serde_json::Value;

const INVENTORY_PATH: &str = "docs/reference/toolkit-source-contract.json";
const COORDINATION_PATH: &str = "docs/adr/0045-toolkit-sibling-coordination.json";

fn string_array(value: &Value, context: &str) -> Vec<String> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("{context} must be an array"))
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .unwrap_or_else(|| panic!("{context} entries must be strings"))
                .to_owned()
        })
        .collect()
}

fn object<'a>(value: &'a Value, context: &str) -> &'a serde_json::Map<String, Value> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("{context} must be an object"))
}

fn lowercase_sha256(value: &str) -> bool {
    lowercase_hex(value, 64)
}

fn lowercase_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state = [
        0x6a09e667_u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_length = u64::try_from(bytes.len())
        .expect("SHA-256 input length fits u64")
        .checked_mul(8)
        .expect("SHA-256 bit length fits u64");
    let mut padded = bytes.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_length.to_be_bytes());

    for chunk in padded.chunks_exact(64) {
        let mut words = [0_u32; 64];
        for (index, word) in words[..16].iter_mut().enumerate() {
            let offset = index * 4;
            *word = u32::from_be_bytes(
                chunk[offset..offset + 4]
                    .try_into()
                    .expect("four-byte SHA-256 word"),
            );
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for index in 0..64 {
            let sum1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ ((!e) & g);
            let temporary1 = h
                .wrapping_add(sum1)
                .wrapping_add(choice)
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let sum0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temporary2 = sum0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temporary1);
            d = c;
            c = b;
            b = a;
            a = temporary1.wrapping_add(temporary2);
        }
        for (target, value) in state.iter_mut().zip([a, b, c, d, e, f, g, h].into_iter()) {
            *target = target.wrapping_add(value);
        }
    }

    let mut digest = [0_u8; 32];
    for (index, word) in state.into_iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    digest
}

fn hex_sha256(bytes: &[u8]) -> String {
    sha256(bytes)
        .into_iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn bytes_fingerprint(domain: &str, id: &str, paths: &[String]) -> String {
    let root = repo_root();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(domain.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(id.as_bytes());
    bytes.push(0);
    for rel in paths {
        let path_bytes = rel.as_bytes();
        let contents = fs::read(root.join(rel))
            .unwrap_or_else(|error| panic!("failed to read {rel}: {error}"));
        bytes.extend_from_slice(&(path_bytes.len() as u64).to_be_bytes());
        bytes.extend_from_slice(path_bytes);
        bytes.extend_from_slice(&(contents.len() as u64).to_be_bytes());
        bytes.extend_from_slice(&contents);
    }
    hex_sha256(&bytes)
}

fn file_sha256(rel: &str) -> String {
    let contents = fs::read(repo_root().join(rel))
        .unwrap_or_else(|error| panic!("failed to read {rel}: {error}"));
    hex_sha256(&contents)
}

fn files_below(rel: &str) -> Vec<String> {
    fn collect(root: &Path, current: &Path, files: &mut Vec<String>) {
        let mut entries = fs::read_dir(current)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", current.display()))
            .map(|entry| entry.expect("valid directory entry").path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                collect(root, &path, files);
            } else if path.is_file() {
                files.push(
                    path.strip_prefix(root)
                        .expect("path below repository root")
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }

    let root = repo_root();
    let mut files = Vec::new();
    collect(&root, &root.join(rel), &mut files);
    files.sort();
    files
}

fn exact_source_groups() -> BTreeMap<String, Vec<String>> {
    let root = repo_root();
    let mut contracts = vec![
        "packages/d2b-contracts/Cargo.toml".to_owned(),
        "packages/d2b-contracts/src/lib.rs".to_owned(),
        "packages/d2b-contracts/tests/component_session_v2.rs".to_owned(),
    ];
    contracts.extend(files_below("packages/d2b-contracts/proto/v2"));
    contracts.extend(files_below(
        "packages/d2b-contracts/src/generated_v2_services",
    ));
    for directory in ["packages/d2b-contracts/src", "packages/d2b-contracts/tests"] {
        for entry in fs::read_dir(root.join(directory)).expect("read d2b-contracts directory") {
            let path = entry.expect("valid directory entry").path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if path.is_file() && name.starts_with("v2_") && name.ends_with(".rs") {
                contracts.push(
                    path.strip_prefix(&root)
                        .expect("contract path below root")
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    contracts.sort();
    contracts.dedup();

    let public_artifacts = [
        "docs/reference/component-session-v2-schema.json",
        "docs/reference/component-session-v2-vectors.json",
        "docs/reference/component-session-v2.md",
        "docs/reference/d2b-contracts-features.md",
        "docs/reference/provider-contract-v2-fixture.json",
        "docs/reference/provider-contract-v2.md",
        "docs/reference/provider-contract-v2.schema.json",
        "docs/reference/toolkit-source-contract.md",
        "docs/reference/v2-foundation-crates.md",
        "docs/reference/v2-identity-vectors.json",
        "docs/reference/v2-identity.md",
        "docs/reference/v2-services-schema.json",
        "docs/reference/v2-services.json",
        "docs/reference/v2-services.md",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();

    BTreeMap::from([
        (
            "workspace-manifest".to_owned(),
            vec!["packages/Cargo.toml".to_owned()],
        ),
        ("contracts-v2".to_owned(), contracts),
        ("client".to_owned(), files_below("packages/d2b-client")),
        (
            "session-runtime".to_owned(),
            files_below("packages/d2b-session"),
        ),
        (
            "unix-session".to_owned(),
            files_below("packages/d2b-session-unix"),
        ),
        (
            "provider-runtime".to_owned(),
            files_below("packages/d2b-provider"),
        ),
        (
            "provider-toolkit".to_owned(),
            files_below("packages/d2b-provider-toolkit"),
        ),
        ("public-contract-artifacts".to_owned(), public_artifacts),
    ])
}

fn parse_features(manifest: &str) -> BTreeMap<String, Vec<String>> {
    let section = manifest
        .split_once("[features]")
        .expect("manifest must declare [features]")
        .1
        .split_once("\n[")
        .map_or_else(
            || manifest.split_once("[features]").unwrap().1,
            |(features, _)| features,
        );
    let quoted = Regex::new(r#""([^"]+)""#).expect("valid quoted-value regex");
    let mut features = BTreeMap::new();
    let mut pending = String::new();
    for raw_line in section.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if pending.is_empty() {
            pending.push_str(line);
        } else {
            pending.push(' ');
            pending.push_str(line);
        }
        if !pending.contains(']') {
            continue;
        }
        let (name, values) = pending
            .split_once('=')
            .unwrap_or_else(|| panic!("invalid feature assignment: {pending}"));
        features.insert(
            name.trim().to_owned(),
            quoted
                .captures_iter(values)
                .map(|capture| capture[1].to_owned())
                .collect(),
        );
        pending.clear();
    }
    assert!(pending.is_empty(), "unterminated feature assignment");
    features
}

fn selected_contract_features(manifest: &str) -> Vec<String> {
    let line = manifest
        .lines()
        .find(|line| line.trim_start().starts_with("d2b-contracts ="))
        .expect("package must depend on d2b-contracts");
    assert!(
        line.contains("default-features = false"),
        "d2b-contracts defaults must be disabled: {line}"
    );
    let quoted = Regex::new(r#""([^"]+)""#).expect("valid quoted-value regex");
    let features = line
        .split_once("features = [")
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(features, _)| features)
        .expect("d2b-contracts dependency must select features");
    quoted
        .captures_iter(features)
        .map(|capture| capture[1].to_owned())
        .collect()
}

#[test]
fn toolkit_source_inventory_is_exact_and_fingerprinted() {
    let inventory: Value =
        serde_json::from_str(&read_repo_file(INVENTORY_PATH)).expect("valid toolkit inventory");
    assert_eq!(inventory["schemaVersion"], 1);
    let policy = object(&inventory["fingerprintPolicy"], "fingerprintPolicy");
    assert_eq!(policy["algorithm"], "sha256");
    assert_eq!(policy["digestEncoding"], "lowercase-hex");
    assert_eq!(policy["lengthEncoding"], "u64-big-endian");
    assert_eq!(policy["pathEncoding"], "utf-8");
    assert_eq!(policy["sourceGroupDomain"], "d2b-toolkit-source-group-v1");
    assert_eq!(policy["distributionDomain"], "d2b-toolkit-distribution-v1");

    let expected_groups = exact_source_groups();
    let mut actual_groups = BTreeMap::new();
    for group in inventory["sourceGroups"]
        .as_array()
        .expect("sourceGroups array")
    {
        let group = object(group, "source group");
        let id = group["id"].as_str().expect("source group id").to_owned();
        let entries = group["files"].as_array().expect("source group files");
        let mut paths = Vec::new();
        for entry in entries {
            let entry = object(entry, "source file");
            let path = entry["path"].as_str().expect("source file path").to_owned();
            let digest = entry["sha256"].as_str().expect("source file digest");
            assert!(lowercase_sha256(digest), "{path} has malformed SHA-256");
            assert_eq!(file_sha256(&path), digest, "{path} source digest drifted");
            paths.push(path);
        }
        let mut sorted = paths.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(paths, sorted, "{id} paths must be sorted and unique");
        let fingerprint = group["fingerprint"]
            .as_str()
            .expect("source group fingerprint");
        assert_eq!(
            bytes_fingerprint("d2b-toolkit-source-group-v1", &id, &paths),
            fingerprint,
            "{id} source-group fingerprint drifted"
        );
        assert!(actual_groups.insert(id, paths).is_none());
    }
    assert_eq!(
        actual_groups, expected_groups,
        "toolkit source ownership changed; update the exact inventory intentionally"
    );

    let all_paths = actual_groups
        .values()
        .flatten()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut api_surfaces = BTreeSet::new();
    for surface in inventory["apiSurfaces"]
        .as_array()
        .expect("apiSurfaces array")
    {
        let surface = object(surface, "API surface");
        let id = surface["id"].as_str().expect("API surface id");
        assert!(
            api_surfaces.insert(id.to_owned()),
            "duplicate API surface {id}"
        );
        let paths = string_array(&surface["paths"], "API surface paths");
        assert!(!paths.is_empty(), "{id} API surface must own source paths");
        for path in paths {
            assert!(
                all_paths.contains(&path),
                "{id} API surface path is outside the fingerprinted source: {path}"
            );
        }
    }
    assert_eq!(
        api_surfaces,
        BTreeSet::from([
            "client-resolution-and-services".to_owned(),
            "client-session-and-streams".to_owned(),
            "component-session-runtime".to_owned(),
            "provider-agent-and-conformance".to_owned(),
            "provider-runtime".to_owned(),
            "public-contract-artifacts".to_owned(),
            "redaction".to_owned(),
            "unix-session-transport".to_owned(),
        ])
    );

    let expected_distributions = BTreeMap::from([
        (
            "d2b-client-toolkit",
            vec![
                "workspace-manifest",
                "contracts-v2",
                "session-runtime",
                "unix-session",
                "client",
                "public-contract-artifacts",
            ],
        ),
        (
            "d2b-provider-toolkit",
            vec![
                "workspace-manifest",
                "contracts-v2",
                "session-runtime",
                "provider-runtime",
                "provider-toolkit",
                "public-contract-artifacts",
            ],
        ),
    ]);
    let mut actual_distributions = BTreeMap::new();
    for distribution in inventory["distributions"]
        .as_array()
        .expect("distributions array")
    {
        let distribution = object(distribution, "distribution");
        let id = distribution["id"]
            .as_str()
            .expect("distribution id")
            .to_owned();
        let groups = string_array(&distribution["sourceGroups"], "distribution sourceGroups");
        let mut paths = BTreeSet::new();
        for group in &groups {
            paths.extend(
                actual_groups
                    .get(group)
                    .unwrap_or_else(|| panic!("{id} references unknown source group {group}"))
                    .iter()
                    .cloned(),
            );
        }
        let paths = paths.into_iter().collect::<Vec<_>>();
        assert_eq!(
            bytes_fingerprint("d2b-toolkit-distribution-v1", &id, &paths),
            distribution["fingerprint"]
                .as_str()
                .expect("distribution fingerprint"),
            "{id} distribution fingerprint drifted"
        );
        actual_distributions.insert(id, groups);
    }
    let expected_distributions = expected_distributions
        .into_iter()
        .map(|(id, groups)| {
            (
                id.to_owned(),
                groups.into_iter().map(str::to_owned).collect::<Vec<_>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(actual_distributions, expected_distributions);
}

#[test]
fn toolkit_feature_and_generated_binding_inventory_matches_sources() {
    let inventory: Value =
        serde_json::from_str(&read_repo_file(INVENTORY_PATH)).expect("valid toolkit inventory");

    let contract_features = object(
        &inventory["contractFeatureFamilies"],
        "contractFeatureFamilies",
    )
    .iter()
    .map(|(name, values)| (name.clone(), string_array(values, name)))
    .collect::<BTreeMap<_, _>>();
    assert_eq!(
        parse_features(&read_repo_file("packages/d2b-contracts/Cargo.toml")),
        contract_features,
        "d2b-contracts feature inventory drifted"
    );

    let mut packages = BTreeSet::new();
    for package in inventory["packageFeatures"]
        .as_array()
        .expect("packageFeatures array")
    {
        let package = object(package, "package feature record");
        let name = package["package"].as_str().expect("package name");
        let manifest = package["manifest"].as_str().expect("package manifest");
        assert!(packages.insert(name.to_owned()), "duplicate package {name}");
        let source = read_repo_file(manifest);
        assert!(
            source.contains("publish = false"),
            "{name} must remain non-publishable"
        );
        let expected = object(&package["features"], "package features")
            .iter()
            .map(|(feature, values)| (feature.clone(), string_array(values, feature)))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            parse_features(&source),
            expected,
            "{name} package feature inventory drifted"
        );
        assert_eq!(
            selected_contract_features(&source),
            string_array(&package["contractFeatures"], "contractFeatures"),
            "{name} d2b-contracts selection drifted"
        );
    }
    assert_eq!(
        packages,
        BTreeSet::from([
            "d2b-client".to_owned(),
            "d2b-provider".to_owned(),
            "d2b-provider-toolkit".to_owned(),
            "d2b-session".to_owned(),
            "d2b-session-unix".to_owned(),
        ])
    );

    let proto_paths = files_below("packages/d2b-contracts/proto/v2");
    let generated_paths = files_below("packages/d2b-contracts/src/generated_v2_services");
    let mut inventory_proto = Vec::new();
    let mut inventory_generated = Vec::new();
    for binding in inventory["generatedBindings"]
        .as_array()
        .expect("generatedBindings array")
    {
        let binding = object(binding, "generated binding");
        let stem = binding["stem"].as_str().expect("binding stem");
        let proto = binding["proto"].as_str().expect("binding proto");
        assert_eq!(
            proto,
            format!("packages/d2b-contracts/proto/v2/{stem}.proto")
        );
        let generated = string_array(&binding["rust"], "binding rust paths");
        let expected = if stem == "common" {
            vec![format!(
                "packages/d2b-contracts/src/generated_v2_services/{stem}.rs"
            )]
        } else {
            vec![
                format!("packages/d2b-contracts/src/generated_v2_services/{stem}.rs"),
                format!("packages/d2b-contracts/src/generated_v2_services/{stem}_ttrpc.rs"),
            ]
        };
        assert_eq!(generated, expected, "{stem} generated binding ownership");
        inventory_proto.push(proto.to_owned());
        inventory_generated.extend(generated);
    }
    inventory_proto.sort();
    inventory_generated.sort();
    assert_eq!(inventory_proto, proto_paths);
    assert_eq!(inventory_generated, generated_paths);
}

#[test]
fn toolkit_runtime_crates_do_not_duplicate_serialized_protocol_dtos() {
    let serialized_derive =
        Regex::new(r"(?s)#\s*\[\s*derive\s*\([^)]*\b(Serialize|Deserialize|Message)\b")
            .expect("valid serialized derive regex");
    for root in [
        "packages/d2b-client/src",
        "packages/d2b-provider/src",
        "packages/d2b-provider-toolkit/src",
        "packages/d2b-session/src",
        "packages/d2b-session-unix/src",
    ] {
        for rel in files_below(root) {
            assert!(
                !rel.ends_with(".proto"),
                "protocol sources belong only to d2b-contracts: {rel}"
            );
            let source = read_repo_file(&rel);
            assert!(
                !serialized_derive.is_match(&source),
                "{rel} defines a serialized DTO; use d2b-contracts instead"
            );
            for forbidden in [
                "include_proto!",
                "prost::Message",
                "mod generated_v2_services",
            ] {
                assert!(
                    !source.contains(forbidden),
                    "{rel} duplicates generated protocol ownership via {forbidden}"
                );
            }
        }
    }

    let client_services = read_repo_file("packages/d2b-client/src/service.rs");
    assert!(
        client_services.contains("use d2b_contracts::v2_services")
            && client_services.contains("_ttrpc"),
        "d2b-client must consume canonical generated service bindings"
    );
    let provider_server = read_repo_file("packages/d2b-provider-toolkit/src/server.rs");
    assert!(
        provider_server.contains("d2b_contracts::{")
            && provider_server.contains("provider_runtime_ttrpc"),
        "provider toolkit must consume canonical provider service bindings"
    );
}

#[test]
fn sibling_coordination_graph_has_disjoint_repository_ownership() {
    let graph: Value = serde_json::from_str(&read_repo_file(COORDINATION_PATH))
        .expect("valid toolkit sibling coordination graph");
    assert_eq!(graph["schemaVersion"], 1);
    assert_eq!(graph["protocolSource"], INVENTORY_PATH);

    let gates = graph["contractGates"]
        .as_array()
        .expect("contractGates array");
    let gate_ids = gates
        .iter()
        .map(|gate| {
            object(gate, "contract gate")["id"]
                .as_str()
                .expect("contract gate id")
                .to_owned()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        gate_ids,
        BTreeSet::from([
            "client-provider-foundation".to_owned(),
            "core-control-services".to_owned(),
            "edge-user-desktop-services".to_owned(),
        ])
    );

    let expected = BTreeMap::from([
        ("client-toolkit-distribution", "vicondoa/d2b-toolkit"),
        (
            "provider-toolkit-distribution",
            "vicondoa/d2b-provider-toolkit",
        ),
        ("wlterm", "vicondoa/d2b-wlterm"),
        ("wlcontrol", "vicondoa/d2b-wlcontrol"),
        ("weezterm", "vicondoa/weezterm"),
    ]);
    let mut components = BTreeMap::new();
    let mut repositories = BTreeSet::new();
    for component in graph["components"].as_array().expect("components array") {
        let component = object(component, "component");
        let id = component["id"].as_str().expect("component id");
        let repository = component["repository"]
            .as_str()
            .expect("component repository");
        assert!(
            repositories.insert(repository.to_owned()),
            "repository ownership overlaps for {repository}"
        );
        assert_eq!(component["protocolPolicy"], "canonical-only");
        let revision = component["auditedRevision"]
            .as_str()
            .expect("audited revision");
        assert!(
            revision == "new-repository" || lowercase_hex(revision, 40),
            "{id} auditedRevision must be a full Git object ID or new-repository"
        );
        let ownership = object(&component["ownership"], "component ownership");
        assert!(
            !string_array(&ownership["paths"], "ownership paths").is_empty(),
            "{id} must own an explicit path set"
        );
        for gate in string_array(&component["dependsOn"], "component dependsOn") {
            assert!(
                gate_ids.contains(&gate),
                "{id} references unknown contract gate {gate}"
            );
        }
        assert!(components.insert(id, repository).is_none());
    }
    assert_eq!(components, expected);
}
