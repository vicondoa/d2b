### Fixed

- Keep Bazel test runners local while compiling supported Rust actions with
  BuildBuddy, and retry the complete graph locally after a remote deadline.
- Serialize the redb durability fault-injection test binary so its process-wide
  test hooks cannot race under CI.
- Allow the cold realized Nix check to use Bazel's eternal timeout instead of
  failing after the standard 15-minute test budget.
- Keep resource-compiler test schema discovery runtime-based so strict Rust
  path checks remain valid on remote workers.
- Wire the v3 Cloud Hypervisor and Volume provider crates into d2bd's Bazel
  production and test-support closures.
- Keep broker runtime tests on OS temporary storage so remote Rust artifacts do
  not embed the BuildBuddy execroot.
- Recompute v3 production-closure lock authorities after the workspace merge
  and build Nix host tools from the root Cargo workspace.
