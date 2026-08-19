### Fixed

- Keep Bazel test runners local while compiling supported Rust actions with
  BuildBuddy, and retry the complete graph locally after a remote deadline.
- Serialize the redb durability fault-injection test binary so its process-wide
  test hooks cannot race under CI.
- Allow the cold realized Nix check to use Bazel's eternal timeout instead of
  failing after the standard 15-minute test budget.
