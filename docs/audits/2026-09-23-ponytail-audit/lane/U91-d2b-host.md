# U91 d2b-host

net: -~2,950 lines, -0 deps

Crate-invariant context: d2b-host is the W3 file-disjoint host-prepare API
surface (cgroup/net/nft/host/media + bridges + farm primitives), consumed
only by d2b-broker and d2bd/d2bd-runtime. No prior findings. `src/generated/`
absent from this crate. Every finding below was validated by a scout that
read the whole target file and verified zero-caller claims with a
workspace-wide grep (search method named per claim). Duplicate surfaces
surfaced by ≥2 independent scouts with matching evidence are cited once.

Ranked biggest cut first. leaf = crate-local; family = sibling-crate
cluster; wide = cross-crate contract.

1. [delete] Delete whole `dnsmasq.rs` module (213 LOC) + `pub mod dnsmasq`
   re-export arm in lib.rs:63. The module's own doc claims it renders
   `RenderDnsmasqEnvConf` (broker op); that op does not exist in
   d2b-contracts-broker (`grep RenderDnsmasqEnvConf|render_dnsmasq_env_conf
   packages/;nixos-modules/;tests/` → only dnsmasq.rs itself). The broker's
   live dnsmasq path is the `SeedDnsmasqLease` admission kernel
   (d2b-broker/src/kernel_ops.rs:1501), which renders no config; real
   dnsmasq conf generation is in d2b-provider-network-local (controller.rs
   + net-mdns.nix). Replacement: nothing. [packages/d2b-host/src/dnsmasq.rs]
   leaf. [family-family]

2. [delete] Delete whole `ssh_keygen.rs` module (350 LOC) +
   `pub mod ssh_keygen` re-export in lib.rs:42. `probe_fingerprint /
   probe_public_key / parse_ssh_keygen_lf / SshKeyFingerprint /
   SshKeygenError` have zero production callers (`grep ssh_keygen|
   probe_fingerprint|probe_public_key|SshKeygenError packages/;tests/;
   nixos-modules/` → only ssh_keygen.rs + lib.rs). Broker's rotate/trust/
   show hand-rolls `ssh-keygen -lf/-y -f` invocation at broker
   ops/exec_reconcile.rs:257 (its own `run_ssh_keygen` trait); nothing here
   feeds it. Replacement: nothing. [packages/d2b-host/src/ssh_keygen.rs] leaf.

3. [delete] Activation-helper bin: four dead verbs — `ensure-regular-file`,
   `setfacl-on-path`, `clear-acl-on-path`, `chown-if-orphan` (~472 of 1453
   LOC) + their six Args fields (size_mib/acl_spec/also_spec/require_kind/
   if_owner/setfacl_bin) + 5 print_help lines + the fs_posture tests that
   only pin them (tests/activation_helper_fs_posture.rs is 429 LOC, mostly
   dead-verb coverage). Workspace grep of each verb string: only helper
   self + tests; broker's only live verb surface is `enforce-dir-posture`
   (consumed by nixos-modules/host-activation.nix ADR 0046) and the
   private-store verbs (activation TV HOST_PREP_DAG). Includes dead
   `--if-owner` parse (args.rs:396,436) never read by any op. [packages/
   d2b-host/src/bin/d2b-activation-helper.rs:470-1055] family.

4. [delete] `routes.rs` preflight surface with zero production callers
   (~460 LOC): RouteTableSnapshot/RouteRow/AddrFamily/route_preflight ops
   + detect_host_lan_cidrs/HostLanCidrs/KNOWN_FOREIGN_PREFIXES +
   detect_host_lan_cidrs check. Broker does its own `ip`-based preflight in
   broker ops/routes.rs + ops/network.rs; ADR 0015 retired the net-route
   preflight singleton. Keep only `render_hosts_block` +
   `extract_managed_block` + HOSTS_MANAGED_BEGIN/END (live broker
   ops/hosts.rs:22). [packages/d2b-host/src/routes.rs] family.

5. [delete] `netlink.rs` fake-only backend trailer (~340 LOC + ~210 test):
   NetlinkBackend trait, ipv6_off_sequence/readback_sysctls/
   readback_bridge_port_flags, Ipv6OffSysctl family, fake::FakeBackend.
   The broker's real rtnetlink work goes through its own `ip` binary +
   d2b-broker sys.rs (quarantined); no `impl NetlinkBackend` production
   implementor exists (`grep 'impl NetlinkBackend|NetlinkBackend' → only
   fake + d2b-provider-network-local's own unrelated trait`). Keep only
   destroy_value_for_key (used by broker sysctl.rs:208). [packages/
   d2b-host/src/netlink.rs] family.

6. [delete] `ifname.rs` near-duplicate of d2b-contracts-resource v3 ifname
   surface (~400 LOC, non-test): derive_from_env_vm/Fnv1a/base32_crockford/
   detect_collisions/validate_prefix/looks_d2b_owned/DerivedRole. The
   contract crate already ships byte-identical vocabulary (identity.rs/v3
   ifname); broker's live caller at d2b-contracts/src/identity.rs:621
   (ResourceUid::from_bytes family) covers the render. Replacement: re-export
   from d2b-contracts-resource v3 (already a dep). [packages/d2b-host/src/
   ifname.rs] family.

7. [delete] `hardlink_farm.rs` legacy u32-token activation path (~310 LOC +
   ~150 test): swap_current_symlink/current_generation/sweep_live_pool/
   reconcile_stale_swap_tmp + build_farm verbs. Broker's live path is the
   split-layout twins swap_state_current/swap_meta_current + store_sync.rs
   (_sync_inner at :255, store_view_farm.rs). Zero production callers
   (`grep swap_current_symlink|sweep_live_pool|reconcile_stale_swap_tmp`
   → only hardlink_farm.rs self + tests). [packages/d2b-host/src/
   hardlink_farm.rs:1778-1952] family.

8. [delete] `replace-live-paths` surface in hardlink_farm.rs:958-1040 +
   broker wrapper replace_live_paths_cross_mount_safe_async +
   replace_live_paths_via_namespace (store_view_farm.rs:309-340) + helper
   verb cmd_replace_store_view_live (d2b-activation-helper.rs:1267-1290) +
   its test. Repair path goes through run_store_sync_inner(force_republish=
   true); never the exchange (`grep replace_live_paths|ReplaceLivePaths
   Request` → only defs + docs; broker store_view_farm.rs). [packages/
   d2b-host/src/hardlink_farm.rs] family.

9. [shrink] `hardlink_farm.rs` 8 near-identical tmp+fsync+rename+dir-fsync
   skeletons (~282 LOC): write_generation_marker/write_store_paths/
   write_guest_meta/write_host_meta/write_live_marker/write_system_symlink/
   plant_generation_gcroot/swap_current_pointer. Replace with two shared
   helpers (atomic_write_file(path,bytes) + atomic_symlink(target,path),
   ~70 LOC) → -~212. [packages/d2b-host/src/hardlink_farm.rs:706-1990] leaf.

10. [delete] `ownership_matrix.rs` recursive-walk machinery never enabled in
    production: `recursive` field + ChildDrift variant + walk_children +
    should_recurse. The only production caller (broker ownership_preflight.
    rs CANONICAL_MATRIX) sets recursive:false on all 17 rows; `grep
    should_recurse|walk_children|ChildDrift` → matrix.rs + tests only.
    [packages/d2b-host/src/ownership_matrix.rs:83-85,135-147,195-201,338-379]
    leaf.

11. [yagni] `host_prep_dag.rs` `build_host_prep_dag_for` (ranked ~154 LOC)
    + `pub` topo_sort + CycleError + `HostPrepStepId::new` pub — zero
    production callers (`grep build_host_prep_dag_for|topo_sort` → only
    host_prep_dag.rs + its tests; d2bd composes via build_host_prep_dag at
    d2bd/src/composition.rs). Make private or fold; sibling topo_sort exists
    in d2bd-runtime supervisor/dag.rs:222 (family dup). Plus dead
    HostPrepStepFailed struct (zero construction sites). [packages/
    d2b-host/src/host_prep_dag.rs:287-318,377-533] leaf.

12. [delete] `cgroup.rs` `create_d2b_slice` (~46 LOC) — zero production
    callers; broker's own wrapper at ops/cgroup.rs reimplements the
    sequence. `create_vm_role_leaf` (1-line wrapper, zero callers) + dead
    `EnabledControllers` family (only read in-crate tests).
    [packages/d2b-host/src/cgroup.rs:583-628,402-449] leaf.

13. [yagni] `host_generation.rs` / `HostGenerationMeta` write-only + 2
    sibling write-only marker paths — write_host_meta + HOST_META_SCHEMA
    _VERSION never read (`grep HostGenerationMeta` → only host_generation.
    rs + hardlink_farm tests). [packages/d2b-host/src/host_generation.rs]
    leaf.

14. [yagni] `ioctl_policy.rs` test-only surface: `is_allowed` pub (zero
    production callers; broker uses ioctl_allowlist via seccomp::compile_
    ioctl_policy_to_bpf) + dead constants TUNSETPERSIST/TUNSETOWNER/
    TUNATTACHFILTER (write-only; test-negative assertions). Make allowlist
    pub(crate). [packages/d2b-host/src/ioctl_policy.rs:135-144,32-39] leaf.

15. [sig] Duplicate `ioctl_allowlist` when seccomp.rs already exports
    `compile_ioctl_policy_to_bpf` — broker's only seccomp consumer is
    compiled-program → libc::sock_filter (broker sys.rs); the policy's own
    `ioctl_allowlist` pub fn is exercised only by tests. Fold.
    [packages/d2b-host/src/ioctl_policy.rs] leaf.

16. [sig] Activation-helper `canonical_json` + `framed_digest` hand-rolled
    duplicate `framed_canonical_digest` in d2b-contracts-resource
    resource_schema.rs (used by broker handoff + ResourceUid::from_bytes
    family). Replacement: call the shared helper. [packages/d2b-host/src/
    bin/d2b-activation-helper.rs:470-627] wide.

17. [shrink] Activation-helper 3 near-identical stdin-JSON verb bodies
    (run/copy/store verbs ~123 LOC) → one generic run_stdin_json_verb
    helper (~35 LOC) → -~85. [packages/d2b-host/src/bin/d2b-activation-
    helper.rs:1057-1295] family.

## Consistency notes
Duplicate hand-rolled UID→UUIDv4 rendering remains (ResourceUid::from_bytes
at d2b-contracts/src/identity.rs:621 is the shared seam; d2b-host
ifname.rs:24-336 still hand-rolls FNV-1a + base32 — reported as finding 6;
broker-side uuid rendering at find-7/blocks not in lane). Same-filesystem
carve-out for /nix/store is the crate's one correct divergence — keep.
Dossier/ADR 0018 + ADR 0012 document the netlink/rtnetlink migration smoke
dossier cites this crate as authority — refs stay for the live
enforce-dir-posture + ownership carve-out; the retired rtnetlink backend
copy is the cut (finding 5).

## Reopened refusals
None. No prior findings for this crate; nothing reopened.

## Checked
Read lib.rs (module map + disjointness contract), Cargo.toml (deps: no
unused d2b-contracts-resource; nix/sha2/tokio are production deps with
live users; only actual unused-flex is the fake-backends feature
line:75-82, retained per broker dev stanza), BUILD.bazel (no dead targets;
bin target + tests all named; d2b_host_test_support/fake_backends are
broker-required). Scouts read full cgroup.rs, netlink.rs, routes.rs,
ifname.rs, bridge_port.rs, nftables.rs, hardlink_farm.rs, host_prep_dag.rs,
ownership_matrix.rs, host_generation.rs, dnsmasq.rs, ssh_keygen.rs,
ioctl_policy.rs, seccomp.rs, media.rs, ioctl_policy.rs, modules.rs,
devices.rs, routes.rs and activation-helper bin + tests. Zero-caller
claims each name their grep (workspace-wide search across packages/,
nixos-modules/, tests/, docs/). LOC measured, not estimated (see findings
1-3: dnsmasq 213, ssh_keygen 350, helper verbs 472). Leave/family rank
and call-site evidence recorded per finding.

