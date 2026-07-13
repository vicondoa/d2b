use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use super::{
    DELIVERY_SCHEMA_VERSION, DeliveryError, Result,
    command::{
        PullRequestStatus, PullRequestStatusSource, RepositoryProbe, StackGraphSource, TrackedBlob,
    },
    model::{
        AUTHORITATIVE_MANIFEST_PATH, AuthorityBinding, DeliveryManifest, Fingerprint,
        FingerprintSpec, GhStackGraph, PullRequestState, RepositoryPolicy, RepositoryRecord,
        SNAPSHOT_ARTIFACT_KIND, SnapshotRequest, StackNode, WaveSnapshot, canonical_digest,
        prospective_content_id, validate_hash_for_format, validate_repo_relative_path,
        validate_repository_id,
    },
    storage::{
        MAX_JSON_BYTES, StateLayout, acquire_candidate_lock, ensure_external_path,
        read_verified_json, sha256_bytes, write_immutable_json,
    },
};

#[derive(Clone, Debug)]
pub(crate) struct SnapshotContext {
    pub snapshot: WaveSnapshot,
    pub digest: String,
    pub layout: StateLayout,
    pub repository_roots: BTreeMap<String, PathBuf>,
    pub external_exclusions: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CurrentVerification {
    RecordedObjects,
    ExactRefs,
}

struct LoadedAuthority {
    manifest: DeliveryManifest,
    binding: AuthorityBinding,
}

pub fn create_snapshot<P: RepositoryProbe, G: StackGraphSource, S: PullRequestStatusSource>(
    probe: &P,
    graph_source: &G,
    status_source: &S,
    request: &SnapshotRequest,
) -> Result<PathBuf> {
    validate_repository_id(&request.authority_repository)?;
    validate_repo_relative_path(&request.manifest_path)?;
    if request.manifest_path != Path::new(AUTHORITATIVE_MANIFEST_PATH) {
        return Err(DeliveryError::new(format!(
            "authoritative delivery manifest must be read from {AUTHORITATIVE_MANIFEST_PATH}"
        )));
    }
    let preliminary_roots = canonicalize_roots(probe, &request.repository_roots)?;
    let authority = load_authority(probe, request, &preliminary_roots)?;
    authority.manifest.validate()?;
    if authority.manifest.authority_repository != request.authority_repository {
        return Err(DeliveryError::new(
            "checked-in manifest authority_repository does not match invocation",
        ));
    }
    let repository_roots = exact_manifest_roots(&authority.manifest, preliminary_roots)?;
    reject_checkout_paths_in_manifest(&authority.manifest, &repository_roots)?;
    let root_paths = external_exclusions(probe, &repository_roots)?;
    let lock_key = canonical_digest(
        b"d2b-delivery-candidate-lock-v1\0",
        &(
            &authority.binding.manifest_sha256,
            &authority.manifest.program,
            &authority.manifest.wave,
        ),
    )?;
    let (_state_root, _lock) = acquire_candidate_lock(
        &root_paths,
        request.state_root.as_deref(),
        &authority.manifest.wave,
        &lock_key,
    )?;

    verify_clean(probe, &repository_roots)?;
    verify_repository_identities(probe, &repository_roots)?;
    let collected = collect_candidate(
        probe,
        graph_source,
        status_source,
        &authority,
        &repository_roots,
    )?;
    let (_candidate_state_root, _candidate_lock) = acquire_candidate_lock(
        &root_paths,
        request.state_root.as_deref(),
        &authority.manifest.wave,
        &collected.candidate_id,
    )?;
    verify_clean(probe, &repository_roots)?;
    verify_collection_unchanged(
        probe,
        graph_source,
        status_source,
        &collected,
        &repository_roots,
    )?;

    let layout = StateLayout::create(
        &root_paths,
        request.state_root.as_deref(),
        &collected.wave,
        &collected.candidate_id,
    )?;
    let path = layout.snapshot();
    write_immutable_json(&path, &collected)?;
    Ok(path)
}

pub fn read_snapshot(path: &Path) -> Result<WaveSnapshot> {
    let (snapshot, _digest): (WaveSnapshot, String) = read_verified_json(path)?;
    snapshot.validate()?;
    Ok(snapshot)
}

pub(crate) fn load_snapshot_context<P: RepositoryProbe>(
    probe: &P,
    repository_roots: &BTreeMap<String, PathBuf>,
    snapshot_path: &Path,
    verification: CurrentVerification,
) -> Result<SnapshotContext> {
    let (snapshot, digest): (WaveSnapshot, String) = read_verified_json(snapshot_path)?;
    snapshot.validate()?;
    let roots = canonicalize_roots(probe, repository_roots)?;
    let expected_ids = snapshot
        .repository_set
        .iter()
        .map(|repository| repository.id.as_str())
        .collect::<BTreeSet<_>>();
    let actual_ids = roots.keys().map(String::as_str).collect::<BTreeSet<_>>();
    if actual_ids != expected_ids {
        return Err(DeliveryError::new(
            "invocation repository mapping does not exactly match snapshot repository set",
        ));
    }
    let root_paths = external_exclusions(probe, &roots)?;
    ensure_external_path(snapshot_path, &root_paths)?;
    let layout =
        StateLayout::from_snapshot_path(snapshot_path, &snapshot.wave, &snapshot.candidate_id)?;
    ensure_external_path(&layout.root, &root_paths)?;
    verify_snapshot_objects(probe, &snapshot, &roots)?;
    if verification == CurrentVerification::ExactRefs {
        verify_clean(probe, &roots)?;
        verify_exact_refs(probe, &snapshot, &roots)?;
        verify_clean(probe, &roots)?;
        verify_exact_refs(probe, &snapshot, &roots)?;
    }
    Ok(SnapshotContext {
        snapshot,
        digest,
        layout,
        repository_roots: roots,
        external_exclusions: root_paths,
    })
}

fn load_authority<P: RepositoryProbe>(
    probe: &P,
    request: &SnapshotRequest,
    roots: &BTreeMap<String, PathBuf>,
) -> Result<LoadedAuthority> {
    let root = roots
        .get(&request.authority_repository)
        .ok_or_else(|| DeliveryError::new("authority repository mapping is missing"))?;
    let commit_oid = probe.resolve_commit(root, &request.authority_ref)?;
    let tree_oid = probe.tree_for_commit(root, &commit_oid)?;
    let blob = probe.tracked_blob(root, &commit_oid, &request.manifest_path)?;
    if blob.bytes.len() > MAX_JSON_BYTES {
        return Err(DeliveryError::new(format!(
            "checked-in delivery manifest exceeds {MAX_JSON_BYTES} bytes"
        )));
    }
    let manifest: DeliveryManifest = serde_json::from_slice(&blob.bytes).map_err(|error| {
        DeliveryError::new(format!(
            "checked-in delivery manifest is invalid JSON: {error}"
        ))
    })?;
    manifest.validate()?;
    if manifest.authority_repository != request.authority_repository {
        return Err(DeliveryError::new(
            "manifest authority repository differs from invocation",
        ));
    }
    let policy = manifest
        .repository(&request.authority_repository)
        .ok_or_else(|| DeliveryError::new("manifest omits its authority repository"))?;
    if policy.integration_ref != request.authority_ref {
        return Err(DeliveryError::new(
            "authority ref must be the manifest repository integration_ref",
        ));
    }
    Ok(LoadedAuthority {
        manifest,
        binding: AuthorityBinding {
            repository: request.authority_repository.clone(),
            ref_name: request.authority_ref.clone(),
            commit_oid,
            tree_oid,
            manifest_path: path_string(&request.manifest_path)?,
            manifest_blob_oid: blob.oid,
            manifest_sha256: sha256_bytes(&blob.bytes),
        },
    })
}

fn collect_candidate<P: RepositoryProbe, G: StackGraphSource, S: PullRequestStatusSource>(
    probe: &P,
    graph_source: &G,
    status_source: &S,
    authority: &LoadedAuthority,
    roots: &BTreeMap<String, PathBuf>,
) -> Result<WaveSnapshot> {
    let mut graphs = BTreeMap::new();
    for policy in &authority.manifest.repositories {
        let root = root_for(roots, &policy.id)?;
        let graph = graph_source.graph(&policy.id, root)?;
        verify_graph_policy(&authority.manifest, policy, &graph)?;
        graphs.insert(policy.id.clone(), graph);
    }
    let refs = resolve_candidate_refs(
        probe,
        &authority.manifest,
        &authority.binding,
        roots,
        &graphs,
    )?;

    let mut repository_set = Vec::new();
    let mut stack = Vec::new();
    for policy in &authority.manifest.repositories {
        let root = root_for(roots, &policy.id)?;
        let graph = graphs
            .get(&policy.id)
            .expect("graph collected for every repository");
        let resolved = refs
            .get(&policy.id)
            .expect("refs collected for every repository");
        let object_format = probe.object_format(root)?;
        if object_format != policy.object_format {
            return Err(DeliveryError::new(format!(
                "repository {} object format differs from authoritative manifest",
                policy.id
            )));
        }
        let trunk_oid = ref_oid(resolved, &policy.trunk_ref)?;
        let integration_oid = ref_oid(resolved, &policy.integration_ref)?;
        let trunk_tree_oid = probe.tree_for_commit(root, trunk_oid)?;
        let integration_tree_oid = probe.tree_for_commit(root, integration_oid)?;
        let graph_bytes = serde_json::to_vec(graph)?;
        repository_set.push(RepositoryRecord {
            id: policy.id.clone(),
            object_format,
            trunk_ref: policy.trunk_ref.clone(),
            trunk_oid: trunk_oid.clone(),
            trunk_tree_oid,
            integration_ref: policy.integration_ref.clone(),
            integration_oid: integration_oid.clone(),
            integration_tree_oid,
            stack_graph_sha256: sha256_bytes(&graph_bytes),
        });

        let policies = authority
            .manifest
            .stack_nodes
            .iter()
            .filter(|node| node.repository == policy.id)
            .map(|node| (node.branch.as_str(), node))
            .collect::<BTreeMap<_, _>>();
        let mut previous_node: Option<String> = None;
        let mut previous_ref = policy.trunk_ref.as_str();
        for branch in &graph.branches {
            let node_policy = policies.get(branch.name.as_str()).copied().ok_or_else(|| {
                DeliveryError::new(format!(
                    "gh-stack branch {} is absent from authoritative stack_nodes",
                    branch.name
                ))
            })?;
            let head_oid = ref_oid(resolved, &branch.name)?;
            let base_oid = ref_oid(resolved, previous_ref)?;
            if &branch.head != head_oid || &branch.base != base_oid {
                return Err(DeliveryError::new(format!(
                    "gh-stack OIDs for {} do not match exact local refs",
                    branch.name
                )));
            }
            if !probe.is_ancestor(root, base_oid, head_oid)? {
                return Err(DeliveryError::new(format!(
                    "stack base {} is not an ancestor of {}",
                    previous_ref, branch.name
                )));
            }
            let head_tree_oid = probe.tree_for_commit(root, head_oid)?;
            let prospective_merge_tree_oid =
                probe.prospective_merge_tree(root, base_oid, head_oid)?;
            let status = status_source.status(&policy.id, node_policy.pr_number)?;
            let snapshot_state = if branch.is_merged {
                PullRequestState::Merged
            } else {
                PullRequestState::Open
            };
            verify_pr_identity_fields(
                node_policy.pr_number,
                &policy.id,
                previous_ref,
                base_oid,
                &branch.name,
                head_oid,
                snapshot_state,
                &status,
            )?;
            let mut depends_on = node_policy.external_dependencies.clone();
            if let Some(previous) = &previous_node {
                depends_on.push(previous.clone());
            }
            depends_on.sort();
            depends_on.dedup();
            stack.push(StackNode {
                id: node_policy.id.clone(),
                repository: policy.id.clone(),
                pr_number: node_policy.pr_number,
                expected_base_ref: previous_ref.to_owned(),
                expected_base_oid: base_oid.clone(),
                head_ref: branch.name.clone(),
                head_oid: head_oid.clone(),
                head_tree_oid: head_tree_oid.clone(),
                prospective_merge_tree_oid: prospective_merge_tree_oid.clone(),
                prospective_content_id: prospective_content_id(
                    &policy.id,
                    object_format,
                    base_oid,
                    head_oid,
                    &head_tree_oid,
                    &prospective_merge_tree_oid,
                )?,
                snapshot_state,
                depends_on,
            });
            previous_node = Some(node_policy.id.clone());
            previous_ref = &branch.name;
        }
    }
    repository_set.sort();
    stack.sort_by(|left, right| {
        let left_repository = repository_set
            .iter()
            .position(|repository| repository.id == left.repository)
            .unwrap_or(usize::MAX);
        let right_repository = repository_set
            .iter()
            .position(|repository| repository.id == right.repository)
            .unwrap_or(usize::MAX);
        left_repository.cmp(&right_repository).then_with(|| {
            graphs[&left.repository]
                .branches
                .iter()
                .position(|branch| branch.name == left.head_ref)
                .cmp(
                    &graphs[&right.repository]
                        .branches
                        .iter()
                        .position(|branch| branch.name == right.head_ref),
                )
        })
    });

    let repository_records = repository_set
        .iter()
        .map(|repository| (repository.id.as_str(), repository))
        .collect::<BTreeMap<_, _>>();
    let generated_artifacts = fingerprint_specs(
        probe,
        &authority.manifest.generated_artifacts,
        roots,
        &repository_records,
    )?;
    let dependency_fingerprints = fingerprint_specs(
        probe,
        &authority.manifest.dependency_fingerprints,
        roots,
        &repository_records,
    )?;
    let contract_fingerprints = fingerprint_specs(
        probe,
        &authority.manifest.contract_fingerprints,
        roots,
        &repository_records,
    )?;
    let mut required_validations = authority.manifest.required_validations.clone();
    required_validations.sort();
    let mut required_checks = authority.manifest.required_checks.clone();
    required_checks.sort();
    let mut snapshot = WaveSnapshot {
        artifact_kind: SNAPSHOT_ARTIFACT_KIND.to_owned(),
        schema_version: DELIVERY_SCHEMA_VERSION,
        program: authority.manifest.program.clone(),
        wave: authority.manifest.wave.clone(),
        candidate_id: "0".repeat(64),
        content_id: "0".repeat(64),
        authority: authority.binding.clone(),
        repository_set,
        stack,
        required_validations,
        required_checks,
        generated_artifacts,
        dependency_fingerprints,
        contract_fingerprints,
    };
    snapshot.content_id = snapshot.recompute_content_id()?;
    snapshot.candidate_id = snapshot.recompute_candidate_id()?;
    snapshot.validate()?;
    Ok(snapshot)
}

fn verify_collection_unchanged<
    P: RepositoryProbe,
    G: StackGraphSource,
    S: PullRequestStatusSource,
>(
    probe: &P,
    graph_source: &G,
    status_source: &S,
    snapshot: &WaveSnapshot,
    roots: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    verify_repository_identities(probe, roots)?;
    for repository in &snapshot.repository_set {
        let root = root_for(roots, &repository.id)?;
        let graph = graph_source.graph(&repository.id, root)?;
        if sha256_bytes(&serde_json::to_vec(&graph)?) != repository.stack_graph_sha256 {
            return Err(DeliveryError::new(format!(
                "gh-stack graph changed while collecting {}",
                repository.id
            )));
        }
    }
    for node in &snapshot.stack {
        let status = status_source.status(&node.repository, node.pr_number)?;
        verify_pr_identity(node, &status)?;
    }
    verify_exact_refs(probe, snapshot, roots)
}

fn verify_repository_identities<P: RepositoryProbe>(
    probe: &P,
    roots: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    for (expected, root) in roots {
        if probe.repository_identity(root)? != *expected {
            return Err(DeliveryError::new(format!(
                "repository identity changed while collecting {expected}"
            )));
        }
    }
    Ok(())
}

fn verify_snapshot_objects<P: RepositoryProbe>(
    probe: &P,
    snapshot: &WaveSnapshot,
    roots: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    let authority_root = root_for(roots, &snapshot.authority.repository)?;
    let authority_tree = probe.tree_for_commit(authority_root, &snapshot.authority.commit_oid)?;
    if authority_tree != snapshot.authority.tree_oid {
        return Err(DeliveryError::new(
            "authority commit tree does not match snapshot",
        ));
    }
    let manifest_blob = probe.tracked_blob(
        authority_root,
        &snapshot.authority.commit_oid,
        Path::new(&snapshot.authority.manifest_path),
    )?;
    if manifest_blob.oid != snapshot.authority.manifest_blob_oid
        || sha256_bytes(&manifest_blob.bytes) != snapshot.authority.manifest_sha256
    {
        return Err(DeliveryError::new(
            "checked-in delivery manifest changed from snapshot authority",
        ));
    }
    let manifest: DeliveryManifest =
        serde_json::from_slice(&manifest_blob.bytes).map_err(|error| {
            DeliveryError::new(format!("bound delivery manifest is invalid JSON: {error}"))
        })?;
    manifest.validate()?;
    verify_manifest_snapshot(&manifest, snapshot)?;

    for repository in &snapshot.repository_set {
        let root = root_for(roots, &repository.id)?;
        let format = probe.object_format(root)?;
        if format != repository.object_format {
            return Err(DeliveryError::new(format!(
                "repository {} object format changed",
                repository.id
            )));
        }
        if probe.tree_for_commit(root, &repository.trunk_oid)? != repository.trunk_tree_oid
            || probe.tree_for_commit(root, &repository.integration_oid)?
                != repository.integration_tree_oid
        {
            return Err(DeliveryError::new(format!(
                "repository {} recorded commit/tree binding is invalid",
                repository.id
            )));
        }
    }
    verify_stack_objects(probe, snapshot, roots)?;
    verify_fingerprint_set(probe, &snapshot.generated_artifacts, snapshot, roots)?;
    verify_fingerprint_set(probe, &snapshot.dependency_fingerprints, snapshot, roots)?;
    verify_fingerprint_set(probe, &snapshot.contract_fingerprints, snapshot, roots)
}

fn verify_manifest_snapshot(manifest: &DeliveryManifest, snapshot: &WaveSnapshot) -> Result<()> {
    if manifest.program != snapshot.program
        || manifest.wave != snapshot.wave
        || manifest.authority_repository != snapshot.authority.repository
    {
        return Err(DeliveryError::new(
            "snapshot identity differs from checked-in delivery manifest",
        ));
    }
    let mut validations = manifest.required_validations.clone();
    validations.sort();
    let mut checks = manifest.required_checks.clone();
    checks.sort();
    if validations != snapshot.required_validations || checks != snapshot.required_checks {
        return Err(DeliveryError::new(
            "snapshot required matrix differs from checked-in delivery manifest",
        ));
    }
    verify_fingerprint_specs_match(
        &manifest.generated_artifacts,
        &snapshot.generated_artifacts,
        "generated artifacts",
    )?;
    verify_fingerprint_specs_match(
        &manifest.dependency_fingerprints,
        &snapshot.dependency_fingerprints,
        "dependency fingerprints",
    )?;
    verify_fingerprint_specs_match(
        &manifest.contract_fingerprints,
        &snapshot.contract_fingerprints,
        "contract fingerprints",
    )?;
    let policy_by_id = manifest
        .repositories
        .iter()
        .map(|policy| (policy.id.as_str(), policy))
        .collect::<BTreeMap<_, _>>();
    for repository in &snapshot.repository_set {
        let policy = policy_by_id.get(repository.id.as_str()).ok_or_else(|| {
            DeliveryError::new(format!(
                "snapshot repository {} is absent from manifest",
                repository.id
            ))
        })?;
        if policy.object_format != repository.object_format
            || policy.trunk_ref != repository.trunk_ref
            || policy.integration_ref != repository.integration_ref
        {
            return Err(DeliveryError::new(format!(
                "snapshot repository {} policy differs from manifest",
                repository.id
            )));
        }
    }
    if policy_by_id.len() != snapshot.repository_set.len() {
        return Err(DeliveryError::new(
            "snapshot repository set is incomplete relative to manifest",
        ));
    }
    let node_policies = manifest
        .stack_nodes
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    for node in &snapshot.stack {
        let policy = node_policies.get(node.id.as_str()).ok_or_else(|| {
            DeliveryError::new(format!("snapshot node {} is absent from manifest", node.id))
        })?;
        if policy.repository != node.repository
            || policy.branch != node.head_ref
            || policy.pr_number != node.pr_number
        {
            return Err(DeliveryError::new(format!(
                "snapshot node {} differs from configured branch/PR",
                node.id
            )));
        }
        if !policy
            .external_dependencies
            .iter()
            .all(|dependency| node.depends_on.contains(dependency))
        {
            return Err(DeliveryError::new(format!(
                "snapshot node {} omits an external dependency",
                node.id
            )));
        }
    }
    if node_policies.len() != snapshot.stack.len() {
        return Err(DeliveryError::new(
            "snapshot stack is incomplete relative to manifest",
        ));
    }
    Ok(())
}

fn verify_fingerprint_specs_match(
    specs: &[FingerprintSpec],
    fingerprints: &[Fingerprint],
    label: &str,
) -> Result<()> {
    let expected = specs
        .iter()
        .map(|spec| {
            (
                spec.name.as_str(),
                spec.repository.as_str(),
                spec.path.as_str(),
            )
        })
        .collect::<BTreeSet<_>>();
    let actual = fingerprints
        .iter()
        .map(|fingerprint| {
            (
                fingerprint.name.as_str(),
                fingerprint.repository.as_str(),
                fingerprint.path.as_str(),
            )
        })
        .collect::<BTreeSet<_>>();
    if expected != actual || expected.len() != specs.len() || actual.len() != fingerprints.len() {
        return Err(DeliveryError::new(format!(
            "snapshot {label} differ from checked-in manifest"
        )));
    }
    Ok(())
}

fn verify_stack_objects<P: RepositoryProbe>(
    probe: &P,
    snapshot: &WaveSnapshot,
    roots: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    for repository in &snapshot.repository_set {
        let root = root_for(roots, &repository.id)?;
        let nodes = snapshot
            .stack
            .iter()
            .filter(|node| node.repository == repository.id)
            .collect::<Vec<_>>();
        let mut expected_base_ref = repository.trunk_ref.as_str();
        let mut expected_base_oid = repository.trunk_oid.as_str();
        for node in nodes {
            if node.expected_base_ref != expected_base_ref
                || node.expected_base_oid != expected_base_oid
                || probe.tree_for_commit(root, &node.head_oid)? != node.head_tree_oid
                || !probe.is_ancestor(root, &node.expected_base_oid, &node.head_oid)?
                || probe.prospective_merge_tree(root, &node.expected_base_oid, &node.head_oid)?
                    != node.prospective_merge_tree_oid
            {
                return Err(DeliveryError::new(format!(
                    "snapshot stack node {} has invalid recorded Git identity",
                    node.id
                )));
            }
            let expected_content = prospective_content_id(
                &node.repository,
                repository.object_format,
                &node.expected_base_oid,
                &node.head_oid,
                &node.head_tree_oid,
                &node.prospective_merge_tree_oid,
            )?;
            if expected_content != node.prospective_content_id {
                return Err(DeliveryError::new(format!(
                    "snapshot stack node {} prospective content identity changed",
                    node.id
                )));
            }
            expected_base_ref = &node.head_ref;
            expected_base_oid = &node.head_oid;
        }
    }
    Ok(())
}

fn verify_exact_refs<P: RepositoryProbe>(
    probe: &P,
    snapshot: &WaveSnapshot,
    roots: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    for repository in &snapshot.repository_set {
        let root = root_for(roots, &repository.id)?;
        let mut expected = BTreeMap::from([
            (repository.trunk_ref.as_str(), repository.trunk_oid.as_str()),
            (
                repository.integration_ref.as_str(),
                repository.integration_oid.as_str(),
            ),
        ]);
        for node in snapshot
            .stack
            .iter()
            .filter(|node| node.repository == repository.id)
        {
            expected.insert(node.head_ref.as_str(), node.head_oid.as_str());
        }
        for (reference, oid) in expected {
            if probe.resolve_commit(root, reference)? != oid {
                return Err(DeliveryError::new(format!(
                    "repository {} ref {} moved from snapshot",
                    repository.id, reference
                )));
            }
        }
    }
    Ok(())
}

fn verify_graph_policy(
    manifest: &DeliveryManifest,
    repository: &RepositoryPolicy,
    graph: &GhStackGraph,
) -> Result<()> {
    graph.validate()?;
    if graph.trunk != repository.trunk_ref {
        return Err(DeliveryError::new(format!(
            "gh-stack trunk for {} differs from authoritative manifest",
            repository.id
        )));
    }
    if graph
        .branches
        .last()
        .is_none_or(|branch| branch.name != repository.integration_ref)
    {
        return Err(DeliveryError::new(format!(
            "gh-stack terminal node for {} is not integration_ref {}",
            repository.id, repository.integration_ref
        )));
    }
    let configured = manifest
        .stack_nodes
        .iter()
        .filter(|node| node.repository == repository.id)
        .map(|node| (node.branch.as_str(), node.pr_number))
        .collect::<BTreeSet<_>>();
    let observed = graph
        .branches
        .iter()
        .map(|branch| {
            (
                branch.name.as_str(),
                branch.pr.as_ref().map_or(0, |pr| pr.number),
            )
        })
        .collect::<BTreeSet<_>>();
    if configured != observed {
        return Err(DeliveryError::new(format!(
            "gh-stack graph for {} does not match configured branches/PRs",
            repository.id
        )));
    }
    Ok(())
}

fn resolve_candidate_refs<P: RepositoryProbe>(
    probe: &P,
    manifest: &DeliveryManifest,
    authority: &AuthorityBinding,
    roots: &BTreeMap<String, PathBuf>,
    graphs: &BTreeMap<String, GhStackGraph>,
) -> Result<BTreeMap<String, BTreeMap<String, String>>> {
    let mut all = BTreeMap::new();
    for repository in &manifest.repositories {
        let root = root_for(roots, &repository.id)?;
        let graph = graphs
            .get(&repository.id)
            .expect("graph exists for repository");
        let mut references = BTreeSet::from([
            repository.trunk_ref.as_str(),
            repository.integration_ref.as_str(),
        ]);
        for branch in &graph.branches {
            references.insert(branch.name.as_str());
        }
        let mut resolved = BTreeMap::new();
        for reference in references {
            let oid = if repository.id == authority.repository && reference == authority.ref_name {
                authority.commit_oid.clone()
            } else {
                probe.resolve_commit(root, reference)?
            };
            validate_hash_for_format(&oid, repository.object_format, "resolved candidate ref")?;
            resolved.insert(reference.to_owned(), oid);
        }
        all.insert(repository.id.clone(), resolved);
    }
    Ok(all)
}

fn fingerprint_specs<P: RepositoryProbe>(
    probe: &P,
    specs: &[FingerprintSpec],
    roots: &BTreeMap<String, PathBuf>,
    repositories: &BTreeMap<&str, &RepositoryRecord>,
) -> Result<Vec<Fingerprint>> {
    let mut fingerprints = Vec::with_capacity(specs.len());
    for spec in specs {
        validate_repo_relative_path(Path::new(&spec.path))?;
        let repository = repositories.get(spec.repository.as_str()).ok_or_else(|| {
            DeliveryError::new(format!(
                "fingerprint {} references unknown repository {}",
                spec.name, spec.repository
            ))
        })?;
        let root = root_for(roots, &spec.repository)?;
        let blob = probe.tracked_blob(root, &repository.integration_oid, Path::new(&spec.path))?;
        fingerprints.push(fingerprint(spec, blob));
    }
    fingerprints.sort();
    Ok(fingerprints)
}

fn fingerprint(spec: &FingerprintSpec, blob: TrackedBlob) -> Fingerprint {
    Fingerprint {
        name: spec.name.clone(),
        repository: spec.repository.clone(),
        path: spec.path.clone(),
        git_blob_oid: blob.oid,
        sha256: sha256_bytes(&blob.bytes),
    }
}

fn verify_fingerprint_set<P: RepositoryProbe>(
    probe: &P,
    fingerprints: &[Fingerprint],
    snapshot: &WaveSnapshot,
    roots: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    let repositories = snapshot
        .repository_set
        .iter()
        .map(|repository| (repository.id.as_str(), repository))
        .collect::<BTreeMap<_, _>>();
    for fingerprint in fingerprints {
        let repository = repositories
            .get(fingerprint.repository.as_str())
            .ok_or_else(|| DeliveryError::new("fingerprint repository is absent"))?;
        let root = root_for(roots, &fingerprint.repository)?;
        let blob = probe.tracked_blob(
            root,
            &repository.integration_oid,
            Path::new(&fingerprint.path),
        )?;
        if blob.oid != fingerprint.git_blob_oid || sha256_bytes(&blob.bytes) != fingerprint.sha256 {
            return Err(DeliveryError::new(format!(
                "tracked fingerprint changed: {}",
                fingerprint.name
            )));
        }
    }
    Ok(())
}

pub(crate) fn verify_pr_identity(node: &StackNode, status: &PullRequestStatus) -> Result<()> {
    verify_pr_identity_fields(
        node.pr_number,
        &node.repository,
        &node.expected_base_ref,
        &node.expected_base_oid,
        &node.head_ref,
        &node.head_oid,
        node.snapshot_state,
        status,
    )
}

#[allow(clippy::too_many_arguments)]
fn verify_pr_identity_fields(
    pr_number: u64,
    repository: &str,
    base_ref: &str,
    base_oid: &str,
    head_ref: &str,
    head_oid: &str,
    state: PullRequestState,
    status: &PullRequestStatus,
) -> Result<()> {
    if status.repository != repository
        || status.number != pr_number
        || status.head_repository != repository
        || status.base_ref != base_ref
        || status.base_oid != base_oid
        || status.head_ref != head_ref
        || status.head_oid != head_oid
        || status.state != state
    {
        return Err(DeliveryError::new(format!(
            "live PR {repository}#{pr_number} does not exactly match snapshot base/head identity"
        )));
    }
    Ok(())
}

fn verify_clean<P: RepositoryProbe>(probe: &P, roots: &BTreeMap<String, PathBuf>) -> Result<()> {
    for (repository, root) in roots {
        if probe.is_dirty(root)? {
            return Err(DeliveryError::new(format!(
                "repository {repository} has a dirty worktree"
            )));
        }
    }
    Ok(())
}

fn canonicalize_roots<P: RepositoryProbe>(
    probe: &P,
    roots: &BTreeMap<String, PathBuf>,
) -> Result<BTreeMap<String, PathBuf>> {
    if roots.is_empty() || roots.len() > super::model::MAX_REPOSITORIES {
        return Err(DeliveryError::new(
            "repository mapping count is empty or exceeds the bound",
        ));
    }

    let mut canonical = BTreeMap::new();
    let mut seen_paths = BTreeSet::new();
    for (id, root) in roots {
        validate_repository_id(id)?;
        let root = probe.canonical_root(root)?;
        let observed_identity = probe.repository_identity(&root)?;
        if observed_identity != *id {
            return Err(DeliveryError::new(format!(
                "checkout origin identity {observed_identity} does not match logical repository ID {id}"
            )));
        }
        if !seen_paths.insert(root.clone()) {
            return Err(DeliveryError::new(
                "two logical repository IDs map to the same checkout root",
            ));
        }
        canonical.insert(id.clone(), root);
    }
    Ok(canonical)
}

fn external_exclusions<P: RepositoryProbe>(
    probe: &P,
    roots: &BTreeMap<String, PathBuf>,
) -> Result<Vec<PathBuf>> {
    let mut exclusions = BTreeSet::new();
    for root in roots.values() {
        exclusions.insert(root.clone());
        exclusions.insert(probe.git_common_dir(root)?);
    }
    Ok(exclusions.into_iter().collect())
}

fn exact_manifest_roots(
    manifest: &DeliveryManifest,
    roots: BTreeMap<String, PathBuf>,
) -> Result<BTreeMap<String, PathBuf>> {
    let expected = manifest
        .repositories
        .iter()
        .map(|repository| repository.id.as_str())
        .collect::<BTreeSet<_>>();
    let actual = roots.keys().map(String::as_str).collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(DeliveryError::new(
            "invocation repository mapping must exactly match authoritative manifest",
        ));
    }
    Ok(roots)
}

fn reject_checkout_paths_in_manifest(
    manifest: &DeliveryManifest,
    roots: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    let rendered_roots = roots
        .values()
        .map(|root| {
            root.to_str()
                .map(str::to_owned)
                .ok_or_else(|| DeliveryError::new("checkout root is not UTF-8"))
        })
        .collect::<Result<Vec<_>>>()?;
    for validation in &manifest.required_validations {
        for argument in &validation.argv {
            if rendered_roots
                .iter()
                .any(|root| argument.contains(root.as_str()))
            {
                return Err(DeliveryError::new(format!(
                    "validation {} argv contains an absolute checkout path",
                    validation.id
                )));
            }
        }
    }
    Ok(())
}

fn root_for<'a>(roots: &'a BTreeMap<String, PathBuf>, id: &str) -> Result<&'a Path> {
    roots
        .get(id)
        .map(PathBuf::as_path)
        .ok_or_else(|| DeliveryError::new(format!("missing checkout mapping for {id}")))
}

fn ref_oid<'a>(refs: &'a BTreeMap<String, String>, reference: &str) -> Result<&'a String> {
    refs.get(reference)
        .ok_or_else(|| DeliveryError::new(format!("ref {reference} was not resolved")))
}

fn path_string(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| DeliveryError::new("delivery path is not valid UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::model::{
        CheckPublisher, CheckPublisherKind, GitObjectFormat, LogicalPath, RepositoryPolicy,
        RequiredCheck, RequiredValidation, StackNodePolicy, ValidationAuthority,
    };

    fn manifest() -> DeliveryManifest {
        DeliveryManifest {
            schema_version: DELIVERY_SCHEMA_VERSION,
            program: "adr0045".to_owned(),
            wave: "w1".to_owned(),
            authority_repository: "github.com/example/d2b".to_owned(),
            repositories: vec![RepositoryPolicy {
                id: "github.com/example/d2b".to_owned(),
                object_format: GitObjectFormat::Sha1,
                trunk_ref: "main".to_owned(),
                integration_ref: "feature".to_owned(),
            }],
            stack_nodes: vec![StackNodePolicy {
                id: "xtask".to_owned(),
                repository: "github.com/example/d2b".to_owned(),
                branch: "feature".to_owned(),
                pr_number: 42,
                external_dependencies: vec![],
            }],
            required_validations: vec![RequiredValidation {
                id: "unit".to_owned(),
                argv: vec!["cargo".to_owned(), "test".to_owned()],
                cwd: LogicalPath {
                    repository: "github.com/example/d2b".to_owned(),
                    path: ".".to_owned(),
                },
                authority: ValidationAuthority::LocalRunner,
                ci_publisher: None,
                ci_signer_workflow: None,
                timeout_seconds: 60,
            }],
            required_checks: vec![RequiredCheck {
                node: "xtask".to_owned(),
                name: "check".to_owned(),
                publisher: CheckPublisher {
                    kind: CheckPublisherKind::CheckRun,
                    app_slug: "github-actions".to_owned(),
                    app_id: 15368,
                    workflow: "Layer 1".to_owned(),
                    workflow_id: 321,
                },
            }],
            generated_artifacts: vec![],
            dependency_fingerprints: vec![FingerprintSpec {
                name: "dependencies".to_owned(),
                repository: "github.com/example/d2b".to_owned(),
                path: "dependencies.txt".to_owned(),
            }],
            contract_fingerprints: vec![FingerprintSpec {
                name: "contract".to_owned(),
                repository: "github.com/example/d2b".to_owned(),
                path: "contract.json".to_owned(),
            }],
        }
    }

    #[test]
    fn empty_authoritative_matrix_fails_closed() {
        let mut without_checks = manifest();
        without_checks.required_checks.clear();
        assert!(without_checks.validate().is_err());
        let mut without_fingerprints = manifest();
        without_fingerprints.contract_fingerprints.clear();
        assert!(without_fingerprints.validate().is_err());
    }

    #[test]
    fn gh_stack_graph_must_match_configured_branches_and_prs() {
        let graph = GhStackGraph {
            trunk: "main".to_owned(),
            prefix: String::new(),
            current_branch: "other".to_owned(),
            branches: vec![super::super::model::GhStackBranch {
                name: "other".to_owned(),
                head: "a".repeat(40),
                base: "b".repeat(40),
                is_current: true,
                is_merged: false,
                is_queued: false,
                needs_rebase: false,
                pr: Some(super::super::model::GhStackPr {
                    number: 99,
                    url: String::new(),
                    state: "OPEN".to_owned(),
                }),
            }],
        };
        let error = verify_graph_policy(&manifest(), &manifest().repositories[0], &graph)
            .expect_err("graph mismatch");
        assert!(error.to_string().contains("gh-stack"));
    }
}
