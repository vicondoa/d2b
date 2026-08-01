# `d2b-provider-network-local` integration fixtures

This directory holds the heavier container, Host, Guest, cross-process, and
provider-system fixtures for this crate. They cannot run at the hermetic layer
that `tests/` occupies.

`host_fabric.rs` declares the container target and records the production
scenario boundary. It becomes executable when the core adapter and scaffolded
closed broker handlers land; no alternate host mutation path is permitted.
