### Changed

- Cut Rust build and test time by reducing development-profile debug data, retaining an explicit full-debug profile, running pull-request work as two required jobs, and replacing the serial HTML API inventory with one compiler-derived workspace rustdoc JSON census; the local comprehensive gate still runs every surface, all compiler and rustdoc warnings now fail the gate, and the dependency set no longer emits an allowed RustSec warning for `anyhow`.
- Run the independent Nix unit shards with bounded parallelism and aggregate every shard failure without removing or skipping any case.
- Materialize contract fixtures directly from `nix eval` output instead of realizing NixOS systems, patched virtual-machine monitors, and source-built guest tools; only the separately pinned video binary command-surface contract remains a realized check.

### Fixed

- Preserve the resource API's external capability seal when the warning-deny gate compiles its intentionally test-only dependency configuration.
