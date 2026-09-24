# U45 d2b-provider-transport-unix

## Refusals honored
- #S7 [refused, honored] transport-unix has no caller in the repo — declared provider pinned by policy: xtask/src/provider_crate_policy.rs:275-279 pins `src/portal.rs` + `tests/transport.rs`; nixos-modules/provider-runtime-contracts.nix:213 lists it in the provider matrix; committed binding schema `packages/d2b-provider-transport-unix/BUILD.bazel` + runtime contract exists; nothing deleted

## Census (one honest sentence)
Read the crate head census from the workspace census packet: 8 src files (admission 129, audit 31, identity 113, lib 23, metrics 23, portal 387, service 28 = 734) + tests/transport.rs 235 = 969 lines at HEAD; workspace-wide grep for `d2b_provider_transport_unix::` surface = the crate's own portal.rs/service.rs self-references only; per U1 census every provider crate not on the README-only ratchet requires an integration/*.rs (provider_crate_policy) which pins tests/transport.rs — nothing removable.

net: 0 lines, 0 deps (all U1 rows honored with no new evidence; refusals stand)
