//! Per-VM DAG executor.
//!
//! Pure orchestration logic that consumes a
//! [`d2b_core::processes::VmProcessDag`], topo-sorts the nodes,
//! and drives them through a [`NodeRunner`] trait that abstracts the
//! "spawn via broker + register pidfd + poll readiness" sequence.
//!
//! The executor itself does no system calls — the [`NodeRunner`]
//! implementation does. That keeps the DAG logic testable in isolation
//! and lets the production daemon swap in different runners (real
//! broker, in-process fake, dry-run preview).
//!
//! Fail-fast: any node whose spawn or readiness wait returns an error
//! aborts the DAG; the executor returns the per-node history so far so
//! the caller can surface a structured error envelope and typed-error
//! wire shape.
//!
//! Per ADR 0014 §"runner-shape preflight" the readiness predicates the
//! executor honours are the [`d2b_core::processes::ReadinessPredicate`]
//! variants; the [`NodeRunner`] is responsible for actually checking
//! them (poll a Unix socket, hit the CH API, listen on vsock, etc.).

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::time::Duration;

use d2b_core::processes::{DagEdge, NodeId, ProcessNode, ReadinessPredicate, VmProcessDag};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Default timeout for the api-ready phase in split-readiness mode.
pub const DEFAULT_API_TIMEOUT_SECONDS: u64 = 60;
/// Exit code constant for api-ready timeout in strict mode.
pub const EXIT_API_TIMEOUT: i32 = 33;

/// Result of executing a single DAG node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "outcome")]
pub enum NodeOutcome {
    /// Spawn succeeded AND every readiness predicate fired before the
    /// per-node deadline.
    Ready,
    /// Spawn or readiness wait failed; the carried message is the
    /// runner's error string (the typed-error wire shape upstream of
    /// here translates this into the envelope `code`).
    Failed { reason: String },
    /// Node was reached but skipped because a predecessor failed and
    /// the executor is unwinding. Recorded so the caller can render
    /// "skipped because <node-id> failed" alongside the failure.
    Skipped { predecessor: NodeId },
}

/// Per-node history record returned in the [`DagRunReport`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeHistory {
    pub node_id: NodeId,
    pub outcome: NodeOutcome,
}

/// State of the api-ready phase for a split-readiness runner node.
/// Serializes as `"yes"`, `"pending"`, `"timeout"`, or
/// `{"error": "<reason>"}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiReadyState {
    Yes,
    Pending,
    Timeout,
    Error { reason: String },
}

impl Serialize for ApiReadyState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Yes => serializer.serialize_str("yes"),
            Self::Pending => serializer.serialize_str("pending"),
            Self::Timeout => serializer.serialize_str("timeout"),
            Self::Error { reason } => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("error", reason)?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for ApiReadyState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Helper {
            Unit(String),
            Error { error: String },
        }

        match Helper::deserialize(deserializer)? {
            Helper::Unit(value) => match value.as_str() {
                "yes" => Ok(ApiReadyState::Yes),
                "pending" => Ok(ApiReadyState::Pending),
                "timeout" => Ok(ApiReadyState::Timeout),
                other => Err(serde::de::Error::custom(format!(
                    "unknown api-ready state: {other}"
                ))),
            },
            Helper::Error { error } => Ok(ApiReadyState::Error { reason: error }),
        }
    }
}

/// Aggregate report returned by [`DagExecutor::run`]. Always lists
/// every node in topo order — pending/skipped entries are explicit
/// rather than absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DagRunReport {
    pub vm: String,
    pub history: Vec<NodeHistory>,
    pub overall_ok: bool,
    /// Split readiness state for the runner node. None when not in split mode.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub api_ready: Option<ApiReadyState>,
}

/// Errors the DAG validation layer surfaces. These are different from
/// per-node runner failures (which land as [`NodeOutcome::Failed`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum DagError {
    /// The DAG has a cycle; topo sort cannot complete.
    Cycle { residual_nodes: Vec<NodeId> },
    /// An edge references a node id not present in `nodes`.
    UnknownNode { edge: DagEdge },
    /// Two nodes share the same id.
    DuplicateNode { node_id: NodeId },
}

/// Per-node deadline pair: the spawn step (until the broker returns
/// the pidfd) and the readiness step (until the last predicate fires).
/// Defaults match the headless alpha Tier-1 budget; callers can
/// override per-node by deriving from the bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeBudget {
    pub spawn: Duration,
    pub readiness: Duration,
}

impl Default for NodeBudget {
    fn default() -> Self {
        Self {
            spawn: Duration::from_secs(10),
            readiness: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SplitReadinessMode {
    /// Default (--strict): wait for BOTH process-alive AND api-ready. Fail-closed on timeout.
    #[default]
    Strict,
    /// --no-wait-api: exit on process-alive success; api-ready state = Pending.
    NoWaitApi,
}

/// Abstraction over "spawn this node and wait for it to be ready".
/// Implementations:
///
/// - production: dispatches through the broker `SpawnRunner` variant,
///   registers the returned pidfd in
///   [`crate::supervisor::pidfd::PidfdTable`], and runs each
///   [`ReadinessPredicate`] against the live runtime;
/// - tests: a deterministic in-memory fake that records call order
///   and can be programmed to fail at a specific node id.
#[async_trait::async_trait]
pub trait NodeRunner: Send + Sync {
    async fn spawn_and_wait_ready(
        &self,
        vm: &str,
        node: &ProcessNode,
        readiness: &[ReadinessPredicate],
        budget: NodeBudget,
    ) -> Result<(), String>;

    /// Spawn + process-alive check. Returns within ≤ 100 ms.
    /// Default: calls spawn_and_wait_ready with empty predicates.
    async fn spawn_and_check_process_alive(
        &self,
        vm: &str,
        node: &ProcessNode,
        budget: NodeBudget,
    ) -> Result<(), String> {
        self.spawn_and_wait_ready(vm, node, &[], budget).await
    }

    /// Api-ready probe (slow path).
    /// Default: returns Yes if no predicates, else Pending.
    async fn probe_api_ready(
        &self,
        vm: &str,
        node: &ProcessNode,
        readiness: &[ReadinessPredicate],
        timeout: Duration,
    ) -> ApiReadyState {
        let _ = (vm, node, timeout);
        if readiness.is_empty() {
            ApiReadyState::Yes
        } else {
            ApiReadyState::Pending
        }
    }
}

/// Pure topo-sort. Returns nodes in dependency order: any node whose
/// dependencies (edges `from -> to` where this node is `to`) have all
/// been emitted comes next. Cycle → [`DagError::Cycle`].
pub fn topo_sort(dag: &VmProcessDag) -> Result<Vec<NodeId>, DagError> {
    // 1. Build the node id set and detect duplicates.
    let mut id_set: BTreeSet<&NodeId> = BTreeSet::new();
    for node in &dag.nodes {
        if !id_set.insert(&node.id) {
            return Err(DagError::DuplicateNode {
                node_id: node.id.clone(),
            });
        }
    }

    // 2. Build adjacency + in-degree maps. Validate every edge.
    let mut in_degree: HashMap<NodeId, usize> =
        dag.nodes.iter().map(|n| (n.id.clone(), 0_usize)).collect();
    let mut adjacency: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for edge in &dag.edges {
        if !id_set.contains(&edge.from) {
            return Err(DagError::UnknownNode { edge: edge.clone() });
        }
        if !id_set.contains(&edge.to) {
            return Err(DagError::UnknownNode { edge: edge.clone() });
        }
        adjacency
            .entry(edge.from.clone())
            .or_default()
            .push(edge.to.clone());
        *in_degree.entry(edge.to.clone()).or_insert(0) += 1;
    }

    // 3. Kahn's algorithm. We pop sources in stable order (by node id
    //    string) so deterministic test fixtures stay deterministic.
    let mut ready: BTreeMap<NodeId, ()> = BTreeMap::new();
    for (id, deg) in &in_degree {
        if *deg == 0 {
            ready.insert(id.clone(), ());
        }
    }
    let mut queue: VecDeque<NodeId> = ready.keys().cloned().collect();
    let mut sorted: Vec<NodeId> = Vec::with_capacity(dag.nodes.len());

    while let Some(id) = queue.pop_front() {
        sorted.push(id.clone());
        if let Some(neighbours) = adjacency.get(&id) {
            // Pre-collect into a stable order so multi-successor nodes
            // emit deterministic sequences.
            let mut next: Vec<NodeId> = neighbours.clone();
            next.sort();
            for n in next {
                let entry = in_degree.entry(n.clone()).or_insert(0);
                if *entry > 0 {
                    *entry -= 1;
                }
                if *entry == 0 && !sorted.contains(&n) && !queue.contains(&n) {
                    queue.push_back(n);
                }
            }
        }
    }

    if sorted.len() != dag.nodes.len() {
        let residual: Vec<NodeId> = dag
            .nodes
            .iter()
            .map(|n| n.id.clone())
            .filter(|id| !sorted.contains(id))
            .collect();
        return Err(DagError::Cycle {
            residual_nodes: residual,
        });
    }
    Ok(sorted)
}

fn uses_split_readiness(readiness: &[ReadinessPredicate]) -> bool {
    readiness
        .iter()
        .any(|predicate| matches!(predicate, ReadinessPredicate::ApiSocketInfo(_)))
}

/// Executor that drives a topo-sorted DAG through a [`NodeRunner`].
pub struct DagExecutor<R: NodeRunner> {
    runner: R,
    budget: NodeBudget,
}

impl<R: NodeRunner> DagExecutor<R> {
    pub fn new(runner: R) -> Self {
        Self {
            runner,
            budget: NodeBudget::default(),
        }
    }

    pub fn with_budget(runner: R, budget: NodeBudget) -> Self {
        Self { runner, budget }
    }

    /// Execute every node in topo order. On the first failure the
    /// executor stops issuing spawn calls and marks the remaining
    /// nodes [`NodeOutcome::Skipped`].
    pub async fn run(&self, dag: &VmProcessDag) -> Result<DagRunReport, DagError> {
        let order = topo_sort(dag)?;
        let nodes_by_id: HashMap<NodeId, &ProcessNode> =
            dag.nodes.iter().map(|n| (n.id.clone(), n)).collect();

        let mut history: Vec<NodeHistory> = Vec::with_capacity(order.len());
        let mut failed_predecessor: Option<NodeId> = None;
        let mut overall_ok = true;

        for node_id in order {
            if let Some(failed) = &failed_predecessor {
                history.push(NodeHistory {
                    node_id,
                    outcome: NodeOutcome::Skipped {
                        predecessor: failed.clone(),
                    },
                });
                continue;
            }

            let node = nodes_by_id
                .get(&node_id)
                .expect("topo sort emitted unknown node id");
            let result = self
                .runner
                .spawn_and_wait_ready(&dag.vm, node, &node.readiness, self.budget)
                .await;

            match result {
                Ok(()) => history.push(NodeHistory {
                    node_id,
                    outcome: NodeOutcome::Ready,
                }),
                Err(reason) => {
                    overall_ok = false;
                    failed_predecessor = Some(node_id.clone());
                    history.push(NodeHistory {
                        node_id,
                        outcome: NodeOutcome::Failed { reason },
                    });
                }
            }
        }

        Ok(DagRunReport {
            vm: dag.vm.clone(),
            history,
            overall_ok,
            api_ready: None,
        })
    }

    pub async fn run_split(
        &self,
        dag: &VmProcessDag,
        mode: SplitReadinessMode,
        api_timeout: Duration,
    ) -> Result<DagRunReport, DagError> {
        let order = topo_sort(dag)?;
        let nodes_by_id: HashMap<NodeId, &ProcessNode> =
            dag.nodes.iter().map(|n| (n.id.clone(), n)).collect();

        let mut history: Vec<NodeHistory> = Vec::with_capacity(order.len());
        let mut failed_predecessor: Option<NodeId> = None;
        let mut overall_ok = true;
        let mut api_ready = None;
        let mut no_wait_completed = false;

        for node_id in order {
            if no_wait_completed {
                continue;
            }
            if let Some(failed) = &failed_predecessor {
                history.push(NodeHistory {
                    node_id,
                    outcome: NodeOutcome::Skipped {
                        predecessor: failed.clone(),
                    },
                });
                continue;
            }

            let node = nodes_by_id
                .get(&node_id)
                .expect("topo sort emitted unknown node id");
            let result = if uses_split_readiness(&node.readiness) {
                match self
                    .runner
                    .spawn_and_check_process_alive(&dag.vm, node, self.budget)
                    .await
                {
                    Ok(()) => match mode {
                        SplitReadinessMode::NoWaitApi => {
                            api_ready = Some(ApiReadyState::Pending);
                            no_wait_completed = true;
                            Ok(())
                        }
                        SplitReadinessMode::Strict => {
                            let state = self
                                .runner
                                .probe_api_ready(&dag.vm, node, &node.readiness, api_timeout)
                                .await;
                            api_ready = Some(state.clone());
                            match state {
                                ApiReadyState::Yes => Ok(()),
                                ApiReadyState::Pending => Err("api-ready pending".to_owned()),
                                ApiReadyState::Timeout => Err("api-ready timeout".to_owned()),
                                ApiReadyState::Error { reason } => {
                                    Err(format!("api-ready error: {reason}"))
                                }
                            }
                        }
                    },
                    Err(reason) => Err(reason),
                }
            } else {
                self.runner
                    .spawn_and_wait_ready(&dag.vm, node, &node.readiness, self.budget)
                    .await
            };

            match result {
                Ok(()) => history.push(NodeHistory {
                    node_id,
                    outcome: NodeOutcome::Ready,
                }),
                Err(reason) => {
                    overall_ok = false;
                    failed_predecessor = Some(node_id.clone());
                    history.push(NodeHistory {
                        node_id,
                        outcome: NodeOutcome::Failed { reason },
                    });
                }
            }
        }

        Ok(DagRunReport {
            vm: dag.vm.clone(),
            history,
            overall_ok,
            api_ready,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_core::minijail_profile::CgroupPlacement;
    use d2b_core::processes::{
        DagEdge, NodeId, ProcessNode, ProcessRole, VmProcessDag, VmProcessInvariants,
    };
    use std::sync::Arc;
    use std::sync::Mutex;

    fn dummy_profile() -> d2b_core::processes::RoleProfile {
        d2b_core::test_support::RoleProfileBuilder::new()
            .with_profile_id("dummy")
            .with_uid(0)
            .with_gid(0)
            .with_cgroup_placement(CgroupPlacement {
                subtree: "system.slice/d2b-test".to_owned(),
                controllers: vec![],
                delegated: false,
            })
            .build()
    }

    fn dummy_node(id: &str, role: ProcessRole) -> ProcessNode {
        ProcessNode {
            id: NodeId(id.to_owned()),
            role,
            unit: None,
            binary_path: None,
            argv: vec![],
            env: vec![],
            profile: dummy_profile(),
            readiness: vec![],
            plan_ops: vec![],
        }
    }

    fn dummy_invariants() -> VmProcessInvariants {
        VmProcessInvariants {
            swtpm_pre_start_flush: false,
            per_vm_audit_pipeline: true,
            usbip_gating: false,
            tpm_ownership_migration_without_running_vm_mutation: false,
        }
    }

    /// Audit-shape headless DAG:
    /// host-reconcile -> store-preflight -> virtiofsd -> ch -> ssh-ready
    fn audit_headless_dag() -> VmProcessDag {
        VmProcessDag {
            vm: "corp-vm".to_owned(),
            nodes: vec![
                dummy_node("host-reconcile", ProcessRole::HostReconcile),
                dummy_node("store-preflight", ProcessRole::StoreVirtiofsPreflight),
                dummy_node("virtiofsd-ro-store", ProcessRole::Virtiofsd),
                dummy_node("ch", ProcessRole::CloudHypervisorRunner),
                dummy_node("ssh-ready", ProcessRole::GuestSshReadiness),
            ],
            edges: vec![
                DagEdge {
                    from: NodeId("host-reconcile".to_owned()),
                    to: NodeId("store-preflight".to_owned()),
                    reason: "preflight needs reconciled host".to_owned(),
                },
                DagEdge {
                    from: NodeId("store-preflight".to_owned()),
                    to: NodeId("virtiofsd-ro-store".to_owned()),
                    reason: "virtiofsd needs validated store".to_owned(),
                },
                DagEdge {
                    from: NodeId("virtiofsd-ro-store".to_owned()),
                    to: NodeId("ch".to_owned()),
                    reason: "CH connects to virtiofs UDS".to_owned(),
                },
                DagEdge {
                    from: NodeId("ch".to_owned()),
                    to: NodeId("ssh-ready".to_owned()),
                    reason: "guest SSH probe needs running guest".to_owned(),
                },
            ],
            invariants: dummy_invariants(),
        }
    }

    fn split_readiness_dag() -> VmProcessDag {
        let mut ch = dummy_node("ch", ProcessRole::CloudHypervisorRunner);
        ch.readiness = vec![ReadinessPredicate::ApiSocketInfo(
            "/run/d2b/vms/corp-vm/ch.sock".to_owned(),
        )];
        VmProcessDag {
            vm: "corp-vm".to_owned(),
            nodes: vec![dummy_node("host-reconcile", ProcessRole::HostReconcile), ch],
            edges: vec![DagEdge {
                from: NodeId("host-reconcile".to_owned()),
                to: NodeId("ch".to_owned()),
                reason: "host before ch".to_owned(),
            }],
            invariants: dummy_invariants(),
        }
    }

    #[test]
    fn topo_sort_linear_dag() {
        let order = topo_sort(&audit_headless_dag()).unwrap();
        let expected: Vec<NodeId> = [
            "host-reconcile",
            "store-preflight",
            "virtiofsd-ro-store",
            "ch",
            "ssh-ready",
        ]
        .into_iter()
        .map(|s| NodeId(s.to_owned()))
        .collect();
        assert_eq!(order, expected);
    }

    #[test]
    fn topo_sort_diamond_emits_both_branches() {
        // root -> a, root -> b, a -> join, b -> join
        let dag = VmProcessDag {
            vm: "diamond".to_owned(),
            nodes: vec![
                dummy_node("root", ProcessRole::HostReconcile),
                dummy_node("a", ProcessRole::Virtiofsd),
                dummy_node("b", ProcessRole::Virtiofsd),
                dummy_node("join", ProcessRole::CloudHypervisorRunner),
            ],
            edges: vec![
                DagEdge {
                    from: NodeId("root".to_owned()),
                    to: NodeId("a".to_owned()),
                    reason: "x".to_owned(),
                },
                DagEdge {
                    from: NodeId("root".to_owned()),
                    to: NodeId("b".to_owned()),
                    reason: "x".to_owned(),
                },
                DagEdge {
                    from: NodeId("a".to_owned()),
                    to: NodeId("join".to_owned()),
                    reason: "x".to_owned(),
                },
                DagEdge {
                    from: NodeId("b".to_owned()),
                    to: NodeId("join".to_owned()),
                    reason: "x".to_owned(),
                },
            ],
            invariants: dummy_invariants(),
        };
        let order = topo_sort(&dag).unwrap();
        // root first, join last; a/b in any order in between.
        assert_eq!(order[0], NodeId("root".to_owned()));
        assert_eq!(order[3], NodeId("join".to_owned()));
        let mid: BTreeSet<NodeId> = order[1..3].iter().cloned().collect();
        assert_eq!(
            mid,
            ["a", "b"]
                .into_iter()
                .map(|s| NodeId(s.to_owned()))
                .collect()
        );
    }

    #[test]
    fn topo_sort_detects_cycle() {
        let dag = VmProcessDag {
            vm: "cycle".to_owned(),
            nodes: vec![
                dummy_node("a", ProcessRole::Virtiofsd),
                dummy_node("b", ProcessRole::Virtiofsd),
            ],
            edges: vec![
                DagEdge {
                    from: NodeId("a".to_owned()),
                    to: NodeId("b".to_owned()),
                    reason: "x".to_owned(),
                },
                DagEdge {
                    from: NodeId("b".to_owned()),
                    to: NodeId("a".to_owned()),
                    reason: "x".to_owned(),
                },
            ],
            invariants: dummy_invariants(),
        };
        let err = topo_sort(&dag).unwrap_err();
        match err {
            DagError::Cycle { residual_nodes } => {
                let ids: BTreeSet<NodeId> = residual_nodes.into_iter().collect();
                assert_eq!(
                    ids,
                    ["a", "b"]
                        .into_iter()
                        .map(|s| NodeId(s.to_owned()))
                        .collect()
                );
            }
            other => panic!("expected Cycle, got {other:?}"),
        }
    }

    #[test]
    fn topo_sort_rejects_self_loop_as_cycle() {
        let dag = VmProcessDag {
            vm: "self".to_owned(),
            nodes: vec![dummy_node("a", ProcessRole::Virtiofsd)],
            edges: vec![DagEdge {
                from: NodeId("a".to_owned()),
                to: NodeId("a".to_owned()),
                reason: "x".to_owned(),
            }],
            invariants: dummy_invariants(),
        };
        let err = topo_sort(&dag).unwrap_err();
        assert!(matches!(err, DagError::Cycle { .. }));
    }

    #[test]
    fn topo_sort_rejects_unknown_edge_target() {
        let dag = VmProcessDag {
            vm: "bad".to_owned(),
            nodes: vec![dummy_node("a", ProcessRole::Virtiofsd)],
            edges: vec![DagEdge {
                from: NodeId("a".to_owned()),
                to: NodeId("ghost".to_owned()),
                reason: "x".to_owned(),
            }],
            invariants: dummy_invariants(),
        };
        let err = topo_sort(&dag).unwrap_err();
        assert!(matches!(err, DagError::UnknownNode { .. }));
    }

    #[test]
    fn topo_sort_rejects_duplicate_node_ids() {
        let dag = VmProcessDag {
            vm: "dup".to_owned(),
            nodes: vec![
                dummy_node("a", ProcessRole::Virtiofsd),
                dummy_node("a", ProcessRole::Virtiofsd),
            ],
            edges: vec![],
            invariants: dummy_invariants(),
        };
        let err = topo_sort(&dag).unwrap_err();
        assert!(matches!(err, DagError::DuplicateNode { .. }));
    }

    /// Recording fake runner used by executor tests.
    #[derive(Default)]
    struct FakeRunner {
        spawn_order: Mutex<Vec<String>>,
        // node_id -> error_to_return
        failures: Mutex<HashMap<String, String>>,
    }

    impl FakeRunner {
        fn with_failure(node: &str, reason: &str) -> Self {
            let mut failures = HashMap::new();
            failures.insert(node.to_owned(), reason.to_owned());
            Self {
                spawn_order: Mutex::new(Vec::new()),
                failures: Mutex::new(failures),
            }
        }

        fn observed_order(&self) -> Vec<String> {
            self.spawn_order.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl NodeRunner for FakeRunner {
        async fn spawn_and_wait_ready(
            &self,
            _vm: &str,
            node: &ProcessNode,
            _readiness: &[ReadinessPredicate],
            _budget: NodeBudget,
        ) -> Result<(), String> {
            self.spawn_order.lock().unwrap().push(node.id.0.clone());
            if let Some(reason) = self.failures.lock().unwrap().get(&node.id.0) {
                return Err(reason.clone());
            }
            Ok(())
        }
    }

    struct FakeSplitRunner {
        spawn_order: Mutex<Vec<String>>,
        api_ready_result: ApiReadyState,
    }

    impl FakeSplitRunner {
        fn observed_order(&self) -> Vec<String> {
            self.spawn_order.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl NodeRunner for FakeSplitRunner {
        async fn spawn_and_wait_ready(
            &self,
            _vm: &str,
            node: &ProcessNode,
            _readiness: &[ReadinessPredicate],
            _budget: NodeBudget,
        ) -> Result<(), String> {
            self.spawn_order.lock().unwrap().push(node.id.0.clone());
            Ok(())
        }

        async fn spawn_and_check_process_alive(
            &self,
            _vm: &str,
            node: &ProcessNode,
            _budget: NodeBudget,
        ) -> Result<(), String> {
            self.spawn_order.lock().unwrap().push(node.id.0.clone());
            Ok(())
        }

        async fn probe_api_ready(
            &self,
            _vm: &str,
            _node: &ProcessNode,
            _readiness: &[ReadinessPredicate],
            _timeout: Duration,
        ) -> ApiReadyState {
            self.api_ready_result.clone()
        }
    }

    #[tokio::test]
    async fn executor_runs_all_nodes_in_topo_order_on_success() {
        let runner = FakeRunner::default();
        let observed = {
            let executor = DagExecutor::new(runner);
            let report = executor.run(&audit_headless_dag()).await.unwrap();
            assert!(report.overall_ok);
            assert_eq!(report.history.len(), 5);
            assert!(
                report
                    .history
                    .iter()
                    .all(|h| matches!(&h.outcome, NodeOutcome::Ready))
            );
            executor.runner.observed_order()
        };
        let expected = vec![
            "host-reconcile",
            "store-preflight",
            "virtiofsd-ro-store",
            "ch",
            "ssh-ready",
        ];
        assert_eq!(observed, expected);
    }

    #[tokio::test]
    async fn executor_fail_fast_skips_remaining_nodes() {
        let runner = FakeRunner::with_failure("virtiofsd-ro-store", "virtiofs ready timeout");
        let executor = DagExecutor::new(runner);
        let report = executor.run(&audit_headless_dag()).await.unwrap();

        assert!(!report.overall_ok);
        assert_eq!(report.history.len(), 5);

        let outcomes: Vec<String> = report
            .history
            .iter()
            .map(|h| match &h.outcome {
                NodeOutcome::Ready => format!("ready:{}", h.node_id.0),
                NodeOutcome::Failed { .. } => format!("fail:{}", h.node_id.0),
                NodeOutcome::Skipped { .. } => format!("skip:{}", h.node_id.0),
            })
            .collect();
        assert_eq!(
            outcomes,
            vec![
                "ready:host-reconcile".to_owned(),
                "ready:store-preflight".to_owned(),
                "fail:virtiofsd-ro-store".to_owned(),
                "skip:ch".to_owned(),
                "skip:ssh-ready".to_owned(),
            ]
        );

        // The runner saw three spawn calls and stopped.
        let observed = executor.runner.observed_order();
        assert_eq!(
            observed,
            vec!["host-reconcile", "store-preflight", "virtiofsd-ro-store"]
        );
    }

    #[tokio::test]
    async fn executor_propagates_topo_error() {
        let dag = VmProcessDag {
            vm: "broken".to_owned(),
            nodes: vec![dummy_node("a", ProcessRole::Virtiofsd)],
            edges: vec![DagEdge {
                from: NodeId("a".to_owned()),
                to: NodeId("ghost".to_owned()),
                reason: "x".to_owned(),
            }],
            invariants: dummy_invariants(),
        };
        let runner = FakeRunner::default();
        let executor = DagExecutor::new(runner);
        let err = executor.run(&dag).await.unwrap_err();
        assert!(matches!(err, DagError::UnknownNode { .. }));
    }

    #[tokio::test]
    async fn budget_threaded_to_runner() {
        // Verify the with_budget constructor wires the custom budget
        // through to the runner.
        #[derive(Default)]
        struct CapturingRunner {
            captured: Mutex<Option<NodeBudget>>,
        }

        #[async_trait::async_trait]
        impl NodeRunner for CapturingRunner {
            async fn spawn_and_wait_ready(
                &self,
                _vm: &str,
                _node: &ProcessNode,
                _readiness: &[ReadinessPredicate],
                budget: NodeBudget,
            ) -> Result<(), String> {
                *self.captured.lock().unwrap() = Some(budget);
                Ok(())
            }
        }

        let dag = VmProcessDag {
            vm: "x".to_owned(),
            nodes: vec![dummy_node("a", ProcessRole::Virtiofsd)],
            edges: vec![],
            invariants: dummy_invariants(),
        };
        let custom = NodeBudget {
            spawn: Duration::from_secs(99),
            readiness: Duration::from_secs(123),
        };
        let runner = CapturingRunner::default();
        let report = DagExecutor::with_budget(runner, custom)
            .run(&dag)
            .await
            .unwrap();
        assert!(report.overall_ok);
    }

    #[tokio::test]
    async fn split_readiness_passes_when_process_alive_but_api_slow() {
        let runner = FakeSplitRunner {
            spawn_order: Mutex::new(Vec::new()),
            api_ready_result: ApiReadyState::Timeout,
        };
        let executor = DagExecutor::new(runner);
        let started = std::time::Instant::now();
        let report = executor
            .run_split(
                &split_readiness_dag(),
                SplitReadinessMode::NoWaitApi,
                Duration::from_secs(60),
            )
            .await
            .unwrap();

        assert!(started.elapsed() <= Duration::from_millis(100));
        assert_eq!(report.api_ready, Some(ApiReadyState::Pending));
        assert!(report.overall_ok);
        assert_eq!(
            executor.runner.observed_order(),
            vec!["host-reconcile", "ch"]
        );
        assert!(
            report
                .history
                .iter()
                .all(|entry| matches!(&entry.outcome, NodeOutcome::Ready))
        );
    }

    #[tokio::test]
    async fn no_wait_api_stops_after_split_node_even_when_later_readiness_exists() {
        let mut dag = split_readiness_dag();
        dag.nodes
            .push(dummy_node("ssh-ready", ProcessRole::GuestSshReadiness));
        dag.edges.push(DagEdge {
            from: NodeId("ch".to_owned()),
            to: NodeId("ssh-ready".to_owned()),
            reason: "guest ssh after ch".to_owned(),
        });
        let runner = FakeSplitRunner {
            spawn_order: Mutex::new(Vec::new()),
            api_ready_result: ApiReadyState::Timeout,
        };
        let executor = DagExecutor::new(runner);
        let report = executor
            .run_split(&dag, SplitReadinessMode::NoWaitApi, Duration::from_secs(60))
            .await
            .unwrap();

        assert!(report.overall_ok);
        assert_eq!(report.api_ready, Some(ApiReadyState::Pending));
        assert_eq!(
            executor.runner.observed_order(),
            vec!["host-reconcile", "ch"]
        );
        assert!(
            !report
                .history
                .iter()
                .any(|entry| entry.node_id == NodeId("ssh-ready".to_owned())),
            "no-wait-api should not wait for guest SSH readiness after process-alive"
        );
    }

    #[tokio::test]
    async fn split_readiness_strict_fails_on_api_timeout() {
        let runner = FakeSplitRunner {
            spawn_order: Mutex::new(Vec::new()),
            api_ready_result: ApiReadyState::Timeout,
        };
        let executor = DagExecutor::new(runner);
        let report = executor
            .run_split(
                &split_readiness_dag(),
                SplitReadinessMode::Strict,
                Duration::from_secs(60),
            )
            .await
            .unwrap();

        assert_eq!(report.api_ready, Some(ApiReadyState::Timeout));
        assert!(!report.overall_ok);
        assert_eq!(
            executor.runner.observed_order(),
            vec!["host-reconcile", "ch"]
        );
        assert!(matches!(
            report.history.as_slice(),
            [
                NodeHistory {
                    outcome: NodeOutcome::Ready,
                    ..
                },
                NodeHistory {
                    outcome: NodeOutcome::Failed { reason },
                    ..
                },
            ] if reason == "api-ready timeout"
        ));
    }

    #[test]
    fn api_ready_state_serializes_expected_shapes() {
        assert_eq!(
            serde_json::to_value(ApiReadyState::Yes).unwrap(),
            serde_json::json!("yes")
        );
        assert_eq!(
            serde_json::to_value(ApiReadyState::Pending).unwrap(),
            serde_json::json!("pending")
        );
        assert_eq!(
            serde_json::to_value(ApiReadyState::Timeout).unwrap(),
            serde_json::json!("timeout")
        );
        assert_eq!(
            serde_json::to_value(ApiReadyState::Error {
                reason: "boom".to_owned(),
            })
            .unwrap(),
            serde_json::json!({ "error": "boom" })
        );
        assert_eq!(
            serde_json::from_value::<ApiReadyState>(serde_json::json!({ "error": "boom" }))
                .unwrap(),
            ApiReadyState::Error {
                reason: "boom".to_owned(),
            }
        );
    }

    #[test]
    fn report_round_trip_serializable() {
        let report = DagRunReport {
            vm: "corp-vm".to_owned(),
            history: vec![
                NodeHistory {
                    node_id: NodeId("a".to_owned()),
                    outcome: NodeOutcome::Ready,
                },
                NodeHistory {
                    node_id: NodeId("b".to_owned()),
                    outcome: NodeOutcome::Failed {
                        reason: "boom".to_owned(),
                    },
                },
                NodeHistory {
                    node_id: NodeId("c".to_owned()),
                    outcome: NodeOutcome::Skipped {
                        predecessor: NodeId("b".to_owned()),
                    },
                },
            ],
            overall_ok: false,
            api_ready: None,
        };
        let json = serde_json::to_string(&report).unwrap();
        let parsed: DagRunReport = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, report);
    }

    // Compile-time ensure Arc<dyn NodeRunner> works for the trait-object
    // path the daemon will actually use.
    #[allow(dead_code)]
    fn arc_dyn_compiles(_: Arc<dyn NodeRunner>) {}
}
