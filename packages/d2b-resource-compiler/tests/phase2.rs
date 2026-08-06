//! Hermetic Phase 2 scenarios for the Provider artifact compiler.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use d2b_contracts::v3::{
    ArtifactDigest, ArtifactDigestSet, ArtifactId, CanonicalJsonValue, CompatibilityRange,
    ComponentDescriptor, ComponentType, PolicyEvaluation, ProviderManifest, ResourceTypeName,
    RevocationState, SignatureState, TrustEvidence, canonical_json_bytes,
    execution_policy::{BoundedToken, ExecutionDomain},
    identity::SchemaFingerprint,
    provider::{BinaryRef, ComponentExecution, UpgradeDisposition, UpgradePolicy},
    resource_schema::SchemaVersion,
};
use d2b_core::provider_artifact::{
    AnchoredDir, Argv, Envp, ExecutableFile, LaunchError, LayoutDir, LayoutError, LayoutPath,
    ProcessLauncher, ReadableFile,
};
use d2b_resource_compiler::{
    ArtifactCatalogEntry, CatalogDigests, Diagnostic, StaticPublisherKeys, compile_artifact,
    executable_set_digest, sha256_digest,
};
use ring::signature::{Ed25519KeyPair, KeyPair};

const ARTIFACT: &str = "provider-test";
const PUBLISHER: &str = "first-party";
const SIGNATURE_ID: &str = "test-key";
const MANIFEST_PATH: &str = "share/d2b/provider/provider-manifest.json";
const SIGNATURE_PATH: &str = "share/d2b/provider/provider-manifest.json.sig";
const SCHEMA_PATH: &str = "share/d2b/provider/config-schema.json";

#[derive(Clone)]
enum Node {
    File { bytes: Vec<u8>, mode: u32 },
    Directory,
    Symlink,
    Fifo,
    Socket,
    Device,
}

#[derive(Default, Clone)]
struct MemoryDir {
    nodes: BTreeMap<String, Node>,
}

impl MemoryDir {
    fn file(mut self, path: &str, bytes: Vec<u8>, mode: u32) -> Self {
        self.nodes
            .insert(path.to_owned(), Node::File { bytes, mode });
        self
    }

    fn node(mut self, path: &str, node: Node) -> Self {
        self.nodes.insert(path.to_owned(), node);
        self
    }

    fn names(&self, dir: &str) -> Vec<OsString> {
        let prefix = format!("{dir}/");
        let mut names = BTreeSet::new();
        for path in self.nodes.keys() {
            if let Some(rest) = path.strip_prefix(&prefix)
                && let Some(name) = rest.split('/').next()
            {
                names.insert(name.to_owned());
            }
        }
        names.into_iter().map(OsString::from).collect()
    }
}

#[derive(Clone)]
struct MemoryFile {
    bytes: Vec<u8>,
    offset: usize,
}

impl ReadableFile for MemoryFile {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_prefix(&mut self, output: &mut [u8]) -> Result<usize, LayoutError> {
        let remaining = &self.bytes[self.offset..];
        let count = remaining.len().min(output.len());
        output[..count].copy_from_slice(&remaining[..count]);
        self.offset += count;
        Ok(count)
    }

    fn read_to_digest(self) -> Result<[u8; 32], LayoutError> {
        Ok(sha256_digest(&self.bytes)
            .as_str()
            .strip_prefix("sha256:")
            .map(decode_hex)
            .expect("contract digest"))
    }
}

#[derive(Clone)]
struct MemoryExecutable;

impl ExecutableFile for MemoryExecutable {}

impl AnchoredDir for MemoryDir {
    type Readable = MemoryFile;
    type Executable = MemoryExecutable;

    fn open_readable(&self, path: LayoutPath) -> Result<Self::Readable, LayoutError> {
        let path = path.as_str();
        let Some(node) = self.nodes.get(path) else {
            return Err(LayoutError::Absent);
        };
        match node {
            Node::File { bytes, mode } => {
                if path.starts_with("bin/") && mode & 0o111 == 0 {
                    return Err(LayoutError::NotExecutable);
                }
                Ok(MemoryFile {
                    bytes: bytes.clone(),
                    offset: 0,
                })
            }
            Node::Symlink => Err(LayoutError::SymlinkRefused),
            Node::Directory | Node::Fifo | Node::Socket | Node::Device => {
                Err(LayoutError::NotRegular)
            }
        }
    }

    fn open_executable(&self, path: LayoutPath) -> Result<Self::Executable, LayoutError> {
        match self.nodes.get(path.as_str()) {
            Some(Node::File { .. }) => Ok(MemoryExecutable),
            Some(Node::Symlink) => Err(LayoutError::SymlinkRefused),
            Some(_) => Err(LayoutError::NotRegular),
            None => Err(LayoutError::Absent),
        }
    }

    fn entries(&self, dir: LayoutDir) -> Result<Vec<OsString>, LayoutError> {
        let dir = dir.as_str();
        let exists = self
            .nodes
            .get(dir)
            .is_some_and(|node| matches!(node, Node::Directory))
            || self
                .nodes
                .keys()
                .any(|path| path.starts_with(&format!("{dir}/")));
        if !exists {
            return Err(LayoutError::Absent);
        }
        Ok(self.names(dir))
    }
}

#[derive(Default)]
struct MemoryLauncher {
    error: Option<LaunchError>,
}

impl ProcessLauncher for MemoryLauncher {
    type Executable = MemoryExecutable;

    fn exec_from(
        &self,
        _file: Self::Executable,
        _argv: &Argv,
        _envp: &Envp,
    ) -> Result<std::convert::Infallible, LaunchError> {
        Err(self.error.unwrap_or(LaunchError::PermissionDenied))
    }
}

fn decode_hex(value: &str) -> [u8; 32] {
    let mut output = [0_u8; 32];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        output[index] = (hex_nibble(chunk[0]) << 4) | hex_nibble(chunk[1]);
    }
    output
}

fn hex_nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => panic!("invalid test digest"),
    }
}

fn elf() -> Vec<u8> {
    let mut bytes = vec![0_u8; 64];
    bytes[0..4].copy_from_slice(b"\x7fELF");
    bytes[4] = 2;
    bytes[5] = 1;
    bytes[6] = 1;
    bytes[16..18].copy_from_slice(&3_u16.to_le_bytes());
    bytes
}

fn schema() -> Vec<u8> {
    let value = CanonicalJsonValue::parse(br#"{"type":"object"}"#).unwrap();
    canonical_json_bytes(&value).unwrap()
}

fn fingerprint() -> SchemaFingerprint {
    SchemaFingerprint::parse(
        "sha256:0000000000000000000000000000000000000000000000000000000000000001",
    )
    .unwrap()
}

fn digest(value: &str) -> ArtifactDigest {
    ArtifactDigest::parse(value).unwrap()
}

fn manifest(
    artifact_id: &str,
    executable_digest: ArtifactDigest,
    config_digest: ArtifactDigest,
    execution: ComponentExecution,
    package_digest: ArtifactDigest,
) -> ProviderManifest {
    let component = ComponentDescriptor::new(
        BoundedToken::parse("test-controller").unwrap(),
        ComponentType::Controller,
        [ResourceTypeName::parse("Volume").unwrap()],
        [],
        [ExecutionDomain::System],
        1,
        config_digest.clone(),
        [],
        false,
    )
    .unwrap()
    .with_execution(execution);
    ProviderManifest::new(
        ArtifactId::parse(artifact_id).unwrap(),
        ArtifactDigestSet {
            package: package_digest,
            executable: executable_digest,
            manifest: ArtifactDigest::parse(
                "sha256:0000000000000000000000000000000000000000000000000000000000000001",
            )
            .unwrap(),
            config: config_digest,
            schema: digest(
                "sha256:0000000000000000000000000000000000000000000000000000000000000001",
            ),
            service: digest(
                "sha256:0000000000000000000000000000000000000000000000000000000000000001",
            ),
        },
        TrustEvidence {
            publisher: BoundedToken::parse(PUBLISHER).unwrap(),
            root_epoch: 1,
            publisher_trusted: true,
            signature: SignatureState::Valid,
            revocation: RevocationState::Clear,
            emergency_deny: false,
            provenance: PolicyEvaluation::Accepted,
            sbom: PolicyEvaluation::Accepted,
            license: PolicyEvaluation::Accepted,
            vulnerability: PolicyEvaluation::Accepted,
            conformance: PolicyEvaluation::Accepted,
            support_channel: BoundedToken::parse("stable").unwrap(),
        },
        CompatibilityRange {
            api_major: 3,
            api_minor: 0,
            descriptor_fingerprint: fingerprint(),
            state_schema_version: SchemaVersion::new(1, 0).unwrap(),
        },
        [component],
        [],
        [],
        UpgradePolicy {
            drain_before_upgrade: true,
            max_automatic_disposition: UpgradeDisposition::InPlace,
            preserves_durable_state: true,
        },
    )
    .unwrap()
}

fn pem(public_key: &[u8]) -> Vec<u8> {
    let mut der = vec![
        0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
    ];
    der.extend_from_slice(public_key);
    format!(
        "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
        STANDARD.encode(der)
    )
    .into_bytes()
}

fn fixture(
    execution: ComponentExecution,
) -> (
    ArtifactCatalogEntry,
    MemoryDir,
    StaticPublisherKeys,
    Vec<u8>,
) {
    fixture_for_artifact(ARTIFACT, execution)
}

fn fixture_for_artifact(
    artifact_id: &str,
    execution: ComponentExecution,
) -> (
    ArtifactCatalogEntry,
    MemoryDir,
    StaticPublisherKeys,
    Vec<u8>,
) {
    let binary = elf();
    let binary_digest = sha256_digest(&binary);
    let mut executable_digests = BTreeMap::new();
    executable_digests.insert("test-controller".to_owned(), binary_digest);
    let executable_set = executable_set_digest(&executable_digests).unwrap();
    let schema = schema();
    let schema_digest = sha256_digest(&schema);
    let package_digest = sha256_digest(b"selected-output-nar");
    let provider_manifest = manifest(
        artifact_id,
        executable_set.clone(),
        schema_digest.clone(),
        execution,
        package_digest.clone(),
    );
    let manifest_bytes = canonical_json_bytes(&provider_manifest).unwrap();
    let manifest_digest = sha256_digest(&manifest_bytes);
    let keypair = Ed25519KeyPair::from_seed_unchecked(&[7_u8; 32]).unwrap();
    let signature = keypair.sign(&manifest_bytes).as_ref().to_vec();
    let mut keys = StaticPublisherKeys::default();
    keys.insert_key(PUBLISHER, SIGNATURE_ID, pem(keypair.public_key().as_ref()));
    let tree = MemoryDir::default()
        .file(MANIFEST_PATH, manifest_bytes.clone(), 0o644)
        .file(SIGNATURE_PATH, signature, 0o644)
        .file(SCHEMA_PATH, schema, 0o644)
        .file("bin/test-controller", binary, 0o755)
        .node("share/d2b/provider", Node::Directory)
        .node("bin", Node::Directory);
    let entry = ArtifactCatalogEntry::new(
        ArtifactId::parse(artifact_id).unwrap(),
        "/nix/store/test-provider",
        PUBLISHER,
        SIGNATURE_ID,
        CatalogDigests::new(
            package_digest,
            executable_set,
            manifest_digest,
            schema_digest,
        ),
    );
    (entry, tree, keys, manifest_bytes)
}

fn compile(
    entry: &ArtifactCatalogEntry,
    tree: &MemoryDir,
    keys: &StaticPublisherKeys,
) -> Result<d2b_resource_compiler::CompiledArtifact, Diagnostic> {
    compile_artifact(entry, tree, keys)
}

fn kind(result: Result<d2b_resource_compiler::CompiledArtifact, Diagnostic>) -> &'static str {
    result.unwrap_err().code()
}

#[test]
fn nix_build_required_outputs_missing() {
    let (entry, mut tree, keys, _) = fixture(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse("test-controller").unwrap(),
    });
    tree.nodes.remove(MANIFEST_PATH);
    assert_eq!(
        kind(compile(&entry, &tree, &keys)),
        "provider-required-output-absent"
    );
}

#[test]
fn nix_build_layout_entry_unexpected() {
    let (entry, tree, keys, _) = fixture(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse("test-controller").unwrap(),
    });
    let tree = tree.file("share/d2b/provider/stray", b"unpinned".to_vec(), 0o644);
    assert_eq!(
        kind(compile(&entry, &tree, &keys)),
        "provider-layout-entry-unexpected"
    );
}

#[test]
fn nix_build_required_output_not_regular() {
    let (entry, tree, keys, _) = fixture(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse("test-controller").unwrap(),
    });
    let tree = tree.node(SCHEMA_PATH, Node::Symlink);
    assert_eq!(
        kind(compile(&entry, &tree, &keys)),
        "provider-required-output-not-regular"
    );
}

#[test]
fn nix_build_required_output_special_nodes_are_rejected_without_blocking() {
    for node in [Node::Fifo, Node::Socket, Node::Device] {
        let (entry, tree, keys, _) = fixture(ComponentExecution::Launchable {
            binary_ref: BinaryRef::parse("test-controller").unwrap(),
        });
        let tree = tree.node(SCHEMA_PATH, node);
        assert_eq!(
            kind(compile(&entry, &tree, &keys)),
            "provider-required-output-not-regular"
        );
    }
}

#[test]
fn nix_build_manifest_signature_invalid_has_four_codes() {
    let (entry, tree, _, _) = fixture(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse("test-controller").unwrap(),
    });
    let keys = StaticPublisherKeys::default();
    assert_eq!(
        kind(compile(&entry, &tree, &keys)),
        "provider-signature-publisher-unregistered"
    );

    let (entry, tree, _, _) = fixture(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse("test-controller").unwrap(),
    });
    let mut keys = StaticPublisherKeys::default();
    keys.register_publisher(PUBLISHER);
    assert_eq!(
        kind(compile(&entry, &tree, &keys)),
        "provider-signature-id-unresolvable"
    );

    let (entry, mut tree, keys, _) = fixture(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse("test-controller").unwrap(),
    });
    tree.nodes.insert(
        SIGNATURE_PATH.to_owned(),
        Node::File {
            bytes: vec![0; 63],
            mode: 0o644,
        },
    );
    assert_eq!(
        kind(compile(&entry, &tree, &keys)),
        "provider-signature-malformed"
    );

    let (entry, mut tree, keys, _) = fixture(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse("test-controller").unwrap(),
    });
    tree.nodes.insert(
        SIGNATURE_PATH.to_owned(),
        Node::File {
            bytes: vec![0; 64],
            mode: 0o644,
        },
    );
    assert_eq!(
        kind(compile(&entry, &tree, &keys)),
        "provider-signature-verification-failed"
    );
}

#[test]
fn nix_build_manifest_not_canonical() {
    let (entry, mut tree, keys, manifest_bytes) = fixture(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse("test-controller").unwrap(),
    });
    let mut noncanonical = manifest_bytes;
    noncanonical.push(b'\n');
    let keypair = Ed25519KeyPair::from_seed_unchecked(&[7_u8; 32]).unwrap();
    let signature = keypair.sign(&noncanonical).as_ref().to_vec();
    tree.nodes.insert(
        MANIFEST_PATH.to_owned(),
        Node::File {
            bytes: noncanonical,
            mode: 0o644,
        },
    );
    tree.nodes.insert(
        SIGNATURE_PATH.to_owned(),
        Node::File {
            bytes: signature,
            mode: 0o644,
        },
    );
    let diagnostic = compile(&entry, &tree, &keys).unwrap_err();
    assert_eq!(diagnostic.code(), "provider-manifest-not-canonical");
    assert!(
        diagnostic
            .message()
            .contains("d2b-provider-toolkit manifest emit --out <path>")
    );
}

#[test]
fn nix_build_executable_not_elf() {
    let (entry, tree, keys, _) = fixture(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse("test-controller").unwrap(),
    });
    let tree = tree.file("bin/test-controller", b"#!/bin/sh\necho no".to_vec(), 0o755);
    assert_eq!(
        kind(compile(&entry, &tree, &keys)),
        "provider-executable-not-elf"
    );
}

#[test]
fn nix_build_executable_not_executable() {
    let (entry, tree, keys, _) = fixture(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse("test-controller").unwrap(),
    });
    let tree = tree.file("bin/test-controller", elf(), 0o644);
    assert_eq!(
        kind(compile(&entry, &tree, &keys)),
        "provider-executable-not-executable"
    );
}

#[test]
fn nix_build_executable_set_empty() {
    let (entry, mut tree, keys, _) =
        fixture_for_artifact("system-core", ComponentExecution::InProcessBootstrap);
    tree.nodes.remove("bin/test-controller");
    tree.nodes.insert("bin".to_owned(), Node::Directory);
    assert_eq!(
        kind(compile(&entry, &tree, &keys)),
        "provider-executable-set-empty"
    );
}

#[test]
fn nix_build_executable_name_invalid() {
    let (entry, mut tree, keys, _) = fixture(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse("test-controller").unwrap(),
    });
    tree.nodes.insert(
        "bin/Bad_Name".to_owned(),
        Node::File {
            bytes: elf(),
            mode: 0o755,
        },
    );
    assert_eq!(
        kind(compile(&entry, &tree, &keys)),
        "provider-executable-name-invalid"
    );
}

#[test]
fn nix_build_executable_not_regular_file() {
    let (entry, tree, keys, _) = fixture(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse("test-controller").unwrap(),
    });
    let tree = tree.node("bin/test-controller", Node::Directory);
    assert_eq!(
        kind(compile(&entry, &tree, &keys)),
        "provider-executable-not-regular"
    );
}

#[test]
fn nix_build_executable_digest_mismatch() {
    let (mut entry, tree, keys, _) = fixture(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse("test-controller").unwrap(),
    });
    let wrong = CatalogDigests::new(
        entry.digests().package().clone(),
        sha256_digest(b"wrong"),
        entry.digests().manifest().clone(),
        entry.digests().config_schema().clone(),
    );
    entry = entry.with_digests(wrong);
    assert_eq!(
        kind(compile(&entry, &tree, &keys)),
        "provider-digest-mismatch"
    );
}

#[test]
fn nix_build_catalog_manifest_disagreement() {
    let (mut entry, tree, keys, _) = fixture(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse("test-controller").unwrap(),
    });
    let wrong = CatalogDigests::new(
        entry.digests().package().clone(),
        entry.digests().executable().clone(),
        sha256_digest(b"wrong"),
        entry.digests().config_schema().clone(),
    );
    entry = entry.with_digests(wrong);
    assert_eq!(
        kind(compile(&entry, &tree, &keys)),
        "provider-digest-mismatch"
    );
}

#[test]
fn nix_build_component_execution_invalid() {
    let (entry, mut tree, keys, _) = fixture(ComponentExecution::InProcessBootstrap);
    tree.nodes.remove("bin/test-controller");
    assert_eq!(
        kind(compile(&entry, &tree, &keys)),
        "provider-component-execution-invalid"
    );
}

#[test]
fn nix_build_executable_declaration_inconsistent() {
    let (entry, mut tree, keys, _) = fixture(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse("test-controller").unwrap(),
    });
    tree.nodes.remove("bin/test-controller");
    assert_eq!(
        kind(compile(&entry, &tree, &keys)),
        "provider-executable-declaration-inconsistent"
    );
}

#[test]
fn nix_build_binary_ref_unresolved() {
    let (entry, mut tree, keys, _) = fixture(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse("test-controller").unwrap(),
    });
    tree.nodes.remove("bin/test-controller");
    tree.nodes.insert(
        "bin/other".to_owned(),
        Node::File {
            bytes: elf(),
            mode: 0o755,
        },
    );
    assert_eq!(
        kind(compile(&entry, &tree, &keys)),
        "provider-binary-ref-unresolved"
    );
}

#[test]
fn nix_build_manifest_binary_ref_wire_compatible() {
    let (entry, tree, keys, manifest_bytes) = fixture(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse("test-controller").unwrap(),
    });
    let parsed: ProviderManifest = serde_json::from_slice(&manifest_bytes).unwrap();
    assert_eq!(canonical_json_bytes(&parsed).unwrap(), manifest_bytes);
    assert!(compile(&entry, &tree, &keys).is_ok());
}

#[test]
fn nix_build_provider_error_redaction_worst_case_bound() {
    let (entry, mut tree, keys, _) = fixture(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse("test-controller").unwrap(),
    });
    for index in 0..1024 {
        tree.nodes.insert(
            format!("share/d2b/provider/extra-{index:04}"),
            Node::File {
                bytes: vec![0; 1],
                mode: 0o644,
            },
        );
    }
    let diagnostic = compile(&entry, &tree, &keys).unwrap_err();
    assert!(diagnostic.message().len() <= 512);
    assert!(!diagnostic.message().contains("/nix/store"));
    assert!(!diagnostic.message().contains('\n'));
    assert!(!diagnostic.message().contains("secret"));
}

#[test]
fn nix_runtime_launcher_anchored_resolution() {
    let (entry, tree, keys, _) = fixture(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse("test-controller").unwrap(),
    });
    let compiled = compile(&entry, &tree, &keys).unwrap();
    let launcher = MemoryLauncher {
        error: Some(LaunchError::PermissionDenied),
    };
    let error = d2b_resource_compiler::launch_component(
        &entry,
        "test-controller",
        compiled.manifest().components()[0].execution(),
        &tree,
        &launcher,
        &Argv::default(),
        &Envp::default(),
    )
    .unwrap_err();
    assert_eq!(error.code(), "provider-launch-permission-denied");
}

#[test]
fn d2b_core_layout_error_mapping_is_closed() {
    let (entry, mut tree, keys, _) = fixture(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse("test-controller").unwrap(),
    });
    tree.nodes.insert(
        SIGNATURE_PATH.to_owned(),
        Node::File {
            bytes: vec![0; 64],
            mode: 0o644,
        },
    );
    let error = compile(&entry, &tree, &keys).unwrap_err();
    assert_eq!(error.kind().exit_code(), 95);
}
