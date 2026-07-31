### Changed

- Cut Rust build and test time by reducing development-profile debug data, retaining an explicit full-debug profile, running pull-request work as two required jobs, and replacing the serial HTML API inventory with one compiler-derived workspace rustdoc JSON census; the local comprehensive gate still runs every surface.
- Run the independent Nix unit shards with bounded parallelism and aggregate every shard failure without removing or skipping any case.
- Preserve both rendered contract fixture roots and selectively export the expensive patched virtual-machine and source-built guest-tool derivations with source-complete cache keys, so warm runs no longer rebuild them.
