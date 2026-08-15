### Removed

- Removed leftover unused published Nix options `d2b.site.flakePath`, `d2b.realms.<realm>.policy.defaultDeny`, and `ch.exporter.includeTopologyLabels`.
- Removed the historical v0.1.0 host migration script and write-up. Use the current v0-to-v1 and v1-to-v1.1 guides.
- Removed empty flake `apps` and `overlays.default` outputs.
- Removed docs-only `examples/personal-dev` and `examples/work-entra` alias directories. Use `examples/minimal` and `examples/with-entra-id`.
- Removed unused daemon leftover modules: realm access resolver, audit-check, realm stubs, StopDagOwner, and unused virtiofsd/wayland watchdog types. The stop-dag deliverable gate now pins the deletion test in `policy_daemon`.
- Removed unused host leftover helpers: runner-shape preflight, empty `fake` placeholder, and unused `async-trait`. The process-marker pin now retires `packages/d2b-host/src/runner_shape.rs`. The leftover runner-shape preflight test pin is gone.
- Removed the compile-only `d2b-wlproxy-spike` crate, undeclared CLI `human_render.rs`, unused `ProcessNodeBuilder`, unused workspace `rtnetlink`, and the duplicate usbip network-scoping contract test. The runtime-ledger census no longer pins the deleted ProcessNodeBuilder tests.
