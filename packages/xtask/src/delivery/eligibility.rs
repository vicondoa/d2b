use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use super::{
    DeliveryError, Result,
    command::{
        ObservedCheckState, PullRequestState, PullRequestStatus, PullRequestStatusSource,
        RepositoryProbe,
    },
    model::{StackNode, WaveSnapshot},
    seal::verify_seal,
    snapshot::read_snapshot,
};

pub fn check_merge_eligibility<P: RepositoryProbe, S: PullRequestStatusSource>(
    probe: &P,
    status_source: &S,
    seal_path: &Path,
    target_node: &str,
) -> Result<()> {
    verify_seal(probe, seal_path)?;
    let snapshot_path = seal_path
        .parent()
        .ok_or_else(|| DeliveryError::new("seal path has no candidate directory"))?
        .join("snapshot.json");
    let snapshot = read_snapshot(&snapshot_path)?;
    evaluate_merge_eligibility(&snapshot, status_source, target_node)
}

pub fn evaluate_merge_eligibility<S: PullRequestStatusSource>(
    snapshot: &WaveSnapshot,
    status_source: &S,
    target_node: &str,
) -> Result<()> {
    snapshot.validate()?;
    if !snapshot.stack_order_is_unambiguous()? {
        return Err(DeliveryError::new(
            "stack order is ambiguous; merge eligibility fails closed",
        ));
    }
    let nodes = snapshot
        .stack
        .iter()
        .map(|node| (node.id.as_str(), node))
        .collect::<BTreeMap<_, _>>();
    let target = nodes
        .get(target_node)
        .copied()
        .ok_or_else(|| DeliveryError::new(format!("unknown target stack node {target_node}")))?;
    let target_pr = target
        .pr
        .ok_or_else(|| DeliveryError::new(format!("target stack node {} has no PR", target.id)))?;
    let target_status = status_source.status(&target.repository, target_pr)?;
    if target_status.state != PullRequestState::Open {
        return Err(DeliveryError::new(format!(
            "target PR {}#{} is not open",
            target.repository, target_pr
        )));
    }
    if target_status.merge_state != "CLEAN" {
        return Err(DeliveryError::new(format!(
            "target PR {}#{} merge state is not CLEAN",
            target.repository, target_pr
        )));
    }
    verify_required_checks(snapshot, target, &target_status)?;

    for dependency_id in transitive_dependencies(target, &nodes)? {
        let dependency = nodes
            .get(dependency_id.as_str())
            .copied()
            .expect("dependency IDs were resolved");
        let pr = dependency.pr.ok_or_else(|| {
            DeliveryError::new(format!("dependency stack node {} has no PR", dependency.id))
        })?;
        let status = status_source.status(&dependency.repository, pr)?;
        if status.state != PullRequestState::Merged {
            return Err(DeliveryError::new(format!(
                "dependency PR {}#{} is not merged",
                dependency.repository, pr
            )));
        }
    }
    Ok(())
}

fn verify_required_checks(
    snapshot: &WaveSnapshot,
    target: &StackNode,
    status: &PullRequestStatus,
) -> Result<()> {
    let required = snapshot
        .required_checks
        .iter()
        .filter(|check| check.node == target.id)
        .collect::<Vec<_>>();
    if required.is_empty() {
        return Err(DeliveryError::new(format!(
            "target stack node {} has no required checks",
            target.id
        )));
    }
    for check in required {
        let observed = status
            .checks
            .iter()
            .filter(|observed| observed.name == check.name)
            .collect::<Vec<_>>();
        match observed.as_slice() {
            [] => {
                return Err(DeliveryError::new(format!(
                    "required check {} is missing",
                    check.name
                )));
            }
            [observed] if observed.state == ObservedCheckState::Successful => {}
            [observed] if observed.state == ObservedCheckState::Pending => {
                return Err(DeliveryError::new(format!(
                    "required check {} is pending",
                    check.name
                )));
            }
            [_] => {
                return Err(DeliveryError::new(format!(
                    "required check {} failed",
                    check.name
                )));
            }
            _ => {
                return Err(DeliveryError::new(format!(
                    "required check {} is ambiguous",
                    check.name
                )));
            }
        }
    }
    Ok(())
}

fn transitive_dependencies(
    target: &StackNode,
    nodes: &BTreeMap<&str, &StackNode>,
) -> Result<Vec<String>> {
    let mut pending = target.depends_on.clone();
    let mut seen = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let node = nodes.get(id.as_str()).copied().ok_or_else(|| {
            DeliveryError::new(format!(
                "stack node {} references unknown dependency {}",
                target.id, id
            ))
        })?;
        pending.extend(node.depends_on.iter().cloned());
    }
    Ok(seen.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::delivery::{
        DELIVERY_SCHEMA_VERSION,
        command::{ObservedCheck, PullRequestStatus},
        model::{RepositoryRecord, RequiredCheck, RequiredValidation, RootRepository},
    };

    struct FakeStatus {
        statuses: BTreeMap<(String, u64), PullRequestStatus>,
    }

    impl PullRequestStatusSource for FakeStatus {
        fn status(&self, repository: &str, pr: u64) -> Result<PullRequestStatus> {
            self.statuses
                .get(&(repository.to_owned(), pr))
                .cloned()
                .ok_or_else(|| DeliveryError::new("missing fake PR status"))
        }
    }

    fn snapshot() -> WaveSnapshot {
        WaveSnapshot {
            schema_version: DELIVERY_SCHEMA_VERSION,
            wave: "w1".to_owned(),
            root_repository: RootRepository {
                name: "example/d2b".to_owned(),
                root: "/external/repository".to_owned(),
                base_commit: "0".repeat(40),
                head_commit: "1".repeat(40),
                tree_hash: "2".repeat(40),
            },
            repository_set: vec![RepositoryRecord {
                name: "example/d2b".to_owned(),
                root: "/external/repository".to_owned(),
                head_commit: "1".repeat(40),
                tree_hash: "2".repeat(40),
            }],
            stack: vec![
                StackNode {
                    id: "root".to_owned(),
                    repository: "example/d2b".to_owned(),
                    branch: "root".to_owned(),
                    pr: Some(1),
                    head_commit: "1".repeat(40),
                    depends_on: vec![],
                },
                StackNode {
                    id: "leaf".to_owned(),
                    repository: "example/d2b".to_owned(),
                    branch: "leaf".to_owned(),
                    pr: Some(2),
                    head_commit: "3".repeat(40),
                    depends_on: vec!["root".to_owned()],
                },
            ],
            required_validations: vec![RequiredValidation {
                id: "unit".to_owned(),
                command_sha256: "4".repeat(64),
            }],
            required_checks: vec![
                RequiredCheck {
                    node: "leaf".to_owned(),
                    name: "unit".to_owned(),
                },
                RequiredCheck {
                    node: "root".to_owned(),
                    name: "unit".to_owned(),
                },
            ],
            generated_artifacts: vec![],
            dependency_fingerprints: vec![],
            contract_fingerprints: vec![],
        }
    }

    fn open_success() -> PullRequestStatus {
        PullRequestStatus {
            state: PullRequestState::Open,
            merge_state: "CLEAN".to_owned(),
            checks: vec![ObservedCheck {
                name: "unit".to_owned(),
                state: ObservedCheckState::Successful,
            }],
        }
    }

    fn merged() -> PullRequestStatus {
        PullRequestStatus {
            state: PullRequestState::Merged,
            merge_state: "UNKNOWN".to_owned(),
            checks: vec![],
        }
    }

    #[test]
    fn eligible_when_checks_pass_and_dependency_merged() {
        let source = FakeStatus {
            statuses: BTreeMap::from([
                (("example/d2b".to_owned(), 1), merged()),
                (("example/d2b".to_owned(), 2), open_success()),
            ]),
        };
        evaluate_merge_eligibility(&snapshot(), &source, "leaf").expect("eligible");
    }

    #[test]
    fn failed_and_pending_checks_fail_closed() {
        for state in [ObservedCheckState::Failed, ObservedCheckState::Pending] {
            let mut target = open_success();
            target.checks[0].state = state;
            let source = FakeStatus {
                statuses: BTreeMap::from([
                    (("example/d2b".to_owned(), 1), merged()),
                    (("example/d2b".to_owned(), 2), target),
                ]),
            };
            evaluate_merge_eligibility(&snapshot(), &source, "leaf").expect_err("check must fail");
        }
    }

    #[test]
    fn unmerged_dependency_fails_closed() {
        let source = FakeStatus {
            statuses: BTreeMap::from([
                (("example/d2b".to_owned(), 1), open_success()),
                (("example/d2b".to_owned(), 2), open_success()),
            ]),
        };
        let error = evaluate_merge_eligibility(&snapshot(), &source, "leaf").expect_err("unmerged");
        assert!(error.to_string().contains("not merged"));
    }

    #[test]
    fn ambiguous_stack_fails_closed() {
        let mut snapshot = snapshot();
        snapshot.stack[1].depends_on.clear();
        let source = FakeStatus {
            statuses: BTreeMap::from([
                (("example/d2b".to_owned(), 1), open_success()),
                (("example/d2b".to_owned(), 2), open_success()),
            ]),
        };
        let error = evaluate_merge_eligibility(&snapshot, &source, "leaf").expect_err("ambiguous");
        assert!(error.to_string().contains("ambiguous"));
    }
}
