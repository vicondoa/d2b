# Stop-DAG reconcile owner

> Status: leftover `StopDagOwner` deleted. This page is retired.

The unused `packages/d2bd-runtime/src/supervisor/stop_dag.rs` module is gone.
the owner-local daemon lifecycle tests
pins that deletion. Live teardown stays in the daemon supervisor DAG
and broker ops, not a leftover stop-dag owner.
