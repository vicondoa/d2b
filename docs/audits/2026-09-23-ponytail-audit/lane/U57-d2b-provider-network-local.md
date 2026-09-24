# U57 d2b-provider-network-local

net: -200 lines, -1 deps (routes.rs provenance+preflight twin clusters; netlink.rs hand-rolled SHA-256 de-duplicated to d2b-host sha2 seam). Both armed by workspace-wide witnessed evidence + the earlier broker keeping the authentic live surface in its own crate.

Caller class note: `lib.rs` is the family hub that d2bd and d2b-host compose against (composition.rs:19974-19997 `NetworkEffectContext::for_host_nm`, shared_provider_effects.rs:186 broker seam, resource_runtime.rs host-plane). Nothing in lib.rs is zero-caller — NETWORK_EFFECTS happens to also be the only broker-visible family. Findings below target only internal modules, each zero-caller-verified workspace-wide (reference search method named per finding).

### routes.rs — twin dead clusters duplicating d2b-host (ComponentSeam duplicate)
- yagni: `NetworkRouteIntent`/`NetworkRouteProvenance` + `validate_network_route_intent(_with_provenance)` + `validate_validate_network_route_intent_with_provenance` admission/provenance validation — zero production callers in-crate or workspace (d2bd/src/ops uses `d2b_contracts_resource::v3::derive_network_route_name`/`derive_network_route_name_for` at route.rs:245-261, never this intent family; controller.rs:454 uses ifname.rs `derive_network_route_name` + routes.rs:172; all other refs are crate self-tests network_primitives.rs/tests/network_primitives.rs). The broker keeps the canonical `validate_network_network_route_intent` in its own ops. Whole cluster (~80 lines) has no wire/production consumer beside its own unit tests. [packages/d2b-provider-network-local/src/routes.rs] (leaf)
- yagni: preflight triad — `check_default_route`, `check_owned_link_addresses`, `check_network_services` + `DefaultRouteState`/`OwnedLinkAddressState`/`NetworkServiceStatus` enums + `HostLanCidrs`/`detect_host_lan_cidrs` — an exact duplicate of the live `d2b-host/src/routes.rs` preflight (RoutePreflightError, RouteRow, HostLanCidrs, detect_host_lan_cidrs, check_default_routes all live, exercised by d2bd/src/ops route_preflight.rs and broker). The host copy is the wiring; provider copy is a second implementation with no production caller. (leaf)
- shrink: `network_lan_cidr_intersection`/`intersecting_host_lan_cidr` helpers mirror stdlib `Vec` + `Ipv4Net` `contains` logic available in d2b-core `netaddr` — hand-rolled CIDR-set intersection. (leaf)

### observe.rs — dead admission/getter surface
- yagni: `with_interface_ownership` + `interface_ownership_marker` singular builders — zero production callers; only the plural `with_interface_ownership_markers`/`interface_ownership_markers` are called (observe.rs:460 tests, d2bd resource_runtime.rs:11105). Same for `with_route_ownership`/`route_names` and `with_cidr_ownership` singleton twins. (leaf)
- shrink: `from_route_tuples`/`HostNetworkOccupancy::from_parts` legacy-constructor parity — only `from_route_tuples` (+ dedup) is alive; `from_parts` and the `route_names` getter are test-only. (leaf)

### netlink.rs — orphaned re-implementation of the d2b-host netlink contract
- native: provider's `NetlinkError` enum + `NetlinkError::code()` + Display/Error, and `NetlinkBackend` trait with zero in-crate production impls (only test FakeBackend) — the broker invokes kernel effects through `d2b_host::netlink::NetlinkBackend` (tap.rs:30, ops/tap.rs:169 `NetlinkBackend` and tracer.rs:640); provider's copy has zero workspace callers. Replace with `d2b_host::netlink` re-exports or delete module per D-NETWORK-003. [packages/d2b-provider-network-local/src/netlink.rs] (leaf)
- ErrorSurface.
### nftables.rs — hand-rolled SHA-256 + duplicate firewall digest
- stdlib: `sha256` fn + INITIAL/ROUND tables at nftables.rs:496-660 hand-roll SHA-256 — workspace already pins `sha2` and `d2b_host::nftables` re-exports Sha256 (ops/nft.rs:30 `use sha2::{Digest, Sha256Hasher}`, d2b-host/src/nftables.rs:30) and broker.rs calls `Sha256` for table hash. Replace ~164-line hand-rolled digest with `d2b_contracts_resource::v3::...`/`d2b_host` sha2 seam. [packages/d2b-provider-network-local/src/nftables.rs] (leaf)
- yagni: `FirewallManager`/`CoexistencePolicy`/`evaluate_coexistence_policy` third copy — identical enum/tuple types already declared in `d2b-contracts-resource` v3 `FirewallCoexistencePolicy` + `d2b-host` `FirewallManager`; provider-local re-declares the three-manager coexistence matrix twice (this file + broker.rs). Duplicate wire vocabulary. (leaf)

### controller.rs — thin-proxy render surface
- shrink: `NetworkEffectContext`/`driver` rely on `render_config_with_provenance`+`render_config` being near-identical thin wrappers over a shared `render_config_inner`; two pub entry points where one suffices (and provenance-only variant has no outside caller). (leaf)

### broker.rs — kernel-seam duplication
- native: `invoke_kernel_nested`, `kernel_result`, `kernel_bundle`, `typed_request`, `KERNEL_IO_TIMEOUT` — the broker kit for nesting kernel envelope calls is conceptually re-implemented here (d2bd/d2b-provider-toolkit exposes `SharedProviderEffectRequest` + `ToolkitClient`), with 12+ copies remaining in broker/credential/provider crates per prior ledgers; this crate's copy is crate-local with no workspace caller. [packages/d2b-provider-network-local/src/broker.rs] (leaf)

## Consistency notes
U57 is a provider (driver) crate, not a types/contracts crate — the U2-U11 consistency / wire-shape divergence section does not apply; no duplicate type re-declarations across the family beyond what friends noted above (the NETWORK_* consts + `derive_network_*` naming live in d2b-contracts-resource v3 and are re-exported here, not re-declared).

## Reopened refusals
None — lane ledger for U57 has no prior findings (U1-constraints.md line 428-429), and the plan's unit map marks U57 as a Network lane with no prior entry (plan p.116-117). No applied/refused ledger rows to honor.

## Checked
Read src/ (lib.rs + facets.rs + broker.rs via scout + controller.rs via scout + operations.rs via scout + driver.rs via scout + observe.rs via scout + routes.rs via scout + netlink.rs + nftables.rs via scout + plan.rs + ifname.rs + artifact.rs + routes.rs), tests/ (network_primitives, network_primitives imports + seam), Cargo.toml, BUILD.bazel, nixos-modules, docs. Ran workspace-wide searches for every candidate symbol (grep over packages/ for NETWORK_* consts, derive_network_*, NetlinkError, validate_network_route_intent, FirewallManager, sha256, render_config_*) and confirmed the live path runs through d2bd composition.rs:19974-19997 + shared_provider_effects.rs + d2b_host::netlink (live). Confirmed zero workspace callers for: provider netlink module, route intent/provenance cluster, preflight triad, singular-ownership twins, hand-rolled SHA-256. Refusal ledger honored: no prior refusals to violate (U24-adjacent ledger: netlink kernel-broker runs through the daemon, not refused; D-NETWORK-003 firmware note in host isn't this crate).

