//! Contract test: every `Swtpm` DAG node must use `unix-socket-listening`
//! (not `unix-socket-exists`) as its readiness predicate.
//!
//! Background: the swtpm runner creates a Unix socket when the TPM is ready to
//! accept connections. `unix-socket-exists` proves only that a filesystem inode
//! exists — a stale socket from a crashed previous run satisfies it. Using
//! `unix-socket-listening` checks for an active listener and is stale-socket-
//! proof (issue #64).
//!
//! The test reads the RENDERED `processes.json` from the feature-rich
//! `fixture-smoke-full` corpus (NL_FIXTURES_FULL) so it validates the actual
//! emitted artifact, not just the source. It skips cleanly when
//! NL_FIXTURES_FULL is unset (plain `cargo test` pass, non-x86_64 host, or CI
//! runs without the full fixture).

use nixling_contract_tests::load_full_bundle_resolver_from_env;
use nixling_core::processes::{ProcessRole, ReadinessPredicate};

/// Assert that every `Swtpm` node in the full fixture's processes DAG uses
/// `unix-socket-listening` (not `unix-socket-exists`) for readiness.
/// Fails — rather than skips — if no Swtpm node is present in a full
/// fixture that claims to have TPM enabled, so new fixture configurations
/// cannot silently omit this check.
#[test]
fn swtpm_readiness_uses_unix_socket_listening() {
    let test = "swtpm_readiness_uses_unix_socket_listening";
    let Some(resolver) = load_full_bundle_resolver_from_env() else {
        eprintln!("SKIP {test}: NL_FIXTURES_FULL unset (swtpm readiness fixture unavailable)");
        return;
    };

    // Collect every Swtpm node across all VMs in the fixture.
    let swtpm_nodes: Vec<(&str, &str, &[ReadinessPredicate])> = resolver
        .processes
        .vms
        .iter()
        .flat_map(|dag| {
            dag.nodes.iter().filter_map(|node| {
                if node.role == ProcessRole::Swtpm {
                    Some((dag.vm.as_str(), node.id.0.as_str(), node.readiness.as_slice()))
                } else {
                    None
                }
            })
        })
        .collect();

    // The full fixture (corp-full VM) must expose at least one Swtpm node;
    // if it doesn't we fail rather than silently pass, so a fixture rebuild
    // that drops the TPM VM doesn't void this contract.
    assert!(
        !swtpm_nodes.is_empty(),
        "{test}: fixture-smoke-full has no Swtpm nodes — \
         either the corp-full VM lost tpm.enable or the fixture is stale"
    );

    let mut failures: Vec<String> = Vec::new();
    for (vm, node_id, readiness) in &swtpm_nodes {
        for pred in *readiness {
            if matches!(pred, ReadinessPredicate::UnixSocketExists(_)) {
                failures.push(format!(
                    "vm={vm} node={node_id}: readiness uses unix-socket-exists; \
                     must use unix-socket-listening (issue #64 — stale-socket-proof)"
                ));
            }
        }
        // Also assert that at least one unix-socket-listening predicate is
        // present (the Swtpm node must declare positive readiness, not an
        // empty predicate list).
        let has_listening = readiness
            .iter()
            .any(|p| matches!(p, ReadinessPredicate::UnixSocketListening(_)));
        if !has_listening {
            failures.push(format!(
                "vm={vm} node={node_id}: Swtpm readiness list has no unix-socket-listening \
                 predicate (expected tpm.sock to be declared as the readiness gate)"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{test}: {} Swtpm readiness violation(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}
