//! Current security-matrix and boundary-evidence coverage.
//!
//! This closes only the ResourceType/Provider and already-implemented
//! boundaries.  Cutover/reset rows remain owned by U4 and are deliberately not
//! synthesized here.

use std::fs;

use d2b_contract_tests::{read_repo_file, repo_path_exists, repo_root};
use d2b_contracts::v3::identity::STANDARD_RESOURCE_TYPES;

const NON_PROVIDER_PREFIXED: &[&str] = &[
    "d2b-provider",
    "d2b-provider-supervisor",
    "d2b-provider-toolkit",
];

fn section<'a>(source: &'a str, heading: &str, next_heading: &str) -> &'a str {
    let start = source
        .find(heading)
        .unwrap_or_else(|| panic!("missing security-matrix heading {heading}"))
        + heading.len();
    let rest = &source[start..];
    let end = rest.find(next_heading).unwrap_or(rest.len());
    &rest[..end]
}

fn current_provider_names() -> Vec<String> {
    let mut names = fs::read_dir(repo_root().join("packages"))
        .expect("read packages directory")
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let file_type = entry.file_type().ok()?;
            if !file_type.is_dir()
                || !name.starts_with("d2b-provider-")
                || NON_PROVIDER_PREFIXED.contains(&name.as_str())
                || !entry.path().join("Cargo.toml").is_file()
            {
                return None;
            }
            Some(name.trim_start_matches("d2b-provider-").to_owned())
        })
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn has_named_test(path: &str, name: &str) -> bool {
    read_repo_file(path).contains(&format!("fn {name}"))
}

#[test]
fn resource_threat_matrix_covers_the_current_standard_catalog() {
    let security = read_repo_file("docs/specs/ADR-046-security-and-threat-model.md");
    let matrix = section(
        &security,
        "## Per-ResourceType threat matrix",
        "## Per-Provider-family threat matrix",
    );
    for resource_type in STANDARD_RESOURCE_TYPES {
        assert!(
            matrix.contains(&format!("| `{resource_type}` |")),
            "security matrix is missing current ResourceType {resource_type}"
        );
    }
    assert!(matrix.contains("The nineteen standard ResourceTypes"));
}

#[test]
fn provider_threat_matrix_covers_current_crates_and_dossiers() {
    let security = read_repo_file("docs/specs/ADR-046-security-and-threat-model.md");
    let matrix = section(
        &security,
        "## Per-Provider-family threat matrix",
        "## Forbidden designs",
    );
    for provider in current_provider_names() {
        assert!(
            matrix.contains(&format!("`{provider}`")),
            "security matrix is missing current Provider {provider}"
        );
        let dossier = format!("docs/specs/providers/ADR-046-provider-{provider}.md");
        assert!(
            repo_path_exists(&dossier),
            "missing Provider dossier {dossier}"
        );
        let dossier_text = read_repo_file(&dossier);
        assert!(
            dossier_text.to_ascii_lowercase().contains("security")
                || dossier_text.to_ascii_lowercase().contains("invariant"),
            "Provider dossier lacks a security/invariant section: {dossier}"
        );
    }
    assert_eq!(current_provider_names().len(), 27);
}

#[test]
fn quarantine_not_kill_evidence_covers_existing_adoption_boundaries() {
    let evidence = [
        (
            "packages/d2b-provider-system-minijail/tests/conformance.rs",
            "a_reused_pid_without_a_matching_start_time_is_quarantined",
        ),
        (
            "packages/d2b-provider-system-minijail/tests/execution_parents.rs",
            "a_candidate_whose_wait_owner_disagrees_is_quarantined",
        ),
        (
            "packages/d2b-provider-system-systemd/tests/conformance.rs",
            "adoption_never_opens_a_pidfd_for_an_ambiguous_scope",
        ),
        (
            "packages/d2b-provider-runtime-cloud-hypervisor/tests/adoption.rs",
            "stale_start_time_or_generation_is_quarantined_before_pidfd",
        ),
        (
            "packages/d2b-provider-runtime-azure-container-apps/tests/provider_lifecycle.rs",
            "ambiguous_adoption_fails_closed",
        ),
        (
            "packages/d2b-provider-volume-local/tests/layout_conformance.rs",
            "ambiguous_ownership_quarantines_rather_than_deleting_or_reusing",
        ),
    ];
    for (path, test) in evidence {
        assert!(repo_path_exists(path), "missing quarantine evidence {path}");
        assert!(
            has_named_test(path, test),
            "missing quarantine test {path}::{test}"
        );
    }
}

#[test]
fn marker_tamper_evidence_covers_missing_replacement_and_binding_vectors() {
    let marker_tests =
        read_repo_file("packages/d2b-provider-volume-local/tests/marker_fail_closed.rs");
    for test in [
        "missing_but_previously_provisioned_root_is_never_silently_recreated",
        "root_created_before_marker_commit_is_not_adopted",
        "every_visible_provision_crash_state_is_classified",
        "replacement_and_marker_mismatch_are_rejected",
    ] {
        assert!(marker_tests.contains(&format!("fn {test}")));
    }
    let layout_tests =
        read_repo_file("packages/d2b-provider-volume-local/tests/layout_conformance.rs");
    assert!(layout_tests.contains("a_symlink_on_a_no_follow_walk_fails_closed"));
    assert!(layout_tests.contains("foreign_child_acl_is_preserved_or_reported"));
}

#[test]
fn privileged_audit_fail_closed_evidence_is_durable_before_success() {
    let sink = read_repo_file("packages/d2b-audit/src/sink.rs");
    assert!(
        sink.contains("if class == AuditWriteClass::Privileged && state.writer.sync().is_err()")
    );
    assert!(sink.contains("Err(AuditSinkError::Unavailable)"));
    assert!(sink.contains("fn privileged_success_requires_every_segment_durability_step"));
    assert!(sink.contains("fn post_write_failure_rolls_back_chain_and_allows_retry"));
}

#[test]
fn minijail_process_ownership_has_parent_reap_and_poll_only_evidence() {
    let table = read_repo_file("packages/d2bd/src/supervisor/pidfd_table.rs");
    assert!(table.contains("wait_terminated_echild_uses_broker_reap_log"));
    assert!(table.contains("TerminatedByBroker"));
    assert!(table.contains("waitid(P_PIDFD)"));
    let readiness = read_repo_file("packages/d2bd/src/supervisor/readiness_liveness.rs");
    assert!(readiness.contains("peek_for"));
    assert!(readiness.contains("poll_pollin"));
    let adapter = read_repo_file("packages/d2b-provider-supervisor/tests/production_adapter.rs");
    assert!(adapter.contains("broker_backend_uses_the_production_spawn_wire_and_pidfd_handoff"));
    assert!(adapter.contains("WaitReapOwner::Local"));
}
