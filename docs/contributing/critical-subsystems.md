# Critical subsystems

Full invariants for the subsystems where a careless change causes silent data
loss, a security regression, or an unrecoverable device-tampering signal to a
remote identity provider.

[`../../AGENTS.md`](../../AGENTS.md) carries the index: which subsystems are
critical, where each lives, and the one-line risk. **Read the row there
first, then the section here for the subsystem you are about to touch.**
Touch none of these without a clear plan and a corresponding test run.

## Net VM networking / firewall

**Where:** `nixos-modules/net.nix` (the `lib.mkForce` neutralization of `base.nix`'s `10-eth-dhcp`, plus the per-env MTU/MSS and east-west wiring)

Net VM dual-stacks DHCP on its uplink, breaks NAT, or weakens same-env isolation unexpectedly. Validate with `tests/unit/nix/cases/net-vm-network.nix`.

## Per-VM `/nix/store` hardlink farm

**Where:** `nixos-modules/store.nix`, `/var/lib/d2b/vms/<vm>/store{,-meta}/`, `nixos-modules/processes-json.nix` (`virtiofsdRunner` ro-store `--shared-dir`), daemon `StoreSync` op + broker `store_view_farm`

The guest's `/nix/store` MUST be the per-VM closure-only farm `/var/lib/d2b/vms/<vm>/store`, never the host's full `/nix/store`: virtiofsd-ro-store's `--shared-dir` points at that farm (the `share.source == "/nix/store"` string stays as the eval-time sentinel - do not "simplify" it back to serving `/nix/store`, that re-leaks the whole host store to every guest). Requires `/var/lib/d2b` and `/nix/store` on the **same filesystem** - hardlinks can't cross FS boundaries; if split, `d2b vm switch` refuses with a fatal error. The broker builds the farm inside a private mount namespace where `/nix/store` is lazily detached (NixOS bind-mounts `/nix/store` on itself, so a same-`st_dev` cross-vfsmount `link(2)` returns `EXDEV` - recoverable, distinct from a fatal different-filesystem `EXDEV`); a `link(2)` `EMLINK` on a `--optimise`d store's saturated empty-file inode falls back to a byte copy. The daemon owns the sync; there is no per-VM `store-sync` unit.

## TPM persistence (per-VM swtpm)

**Where:** `/var/lib/d2b/vms/<vm>/swtpm/`; spawned via broker `SpawnRunner` from `packages/d2b-host/src/swtpm_argv.rs` and supervised by `d2bd` as a child of the VM's DAG. The broker **provisions + hardens** this dir on first start (`packages/d2b-priv-broker/src/ops/swtpm_dir.rs`, gated on `seccomp_policy_ref == "w1-swtpm"`): fd-safe create (owner `d2b-<vm>-swtpm`, mode 0700, inherited ACLs cleared), reconcile-in-place on a correct-owner existing dir, fail-closed on owner/type/symlink mismatch, ancestor `--x` traverse ACL, stale `tpm.sock` unlink - emitting the path-free `PrepareSwtpmDir` audit op.

Holds the per-VM TPM 2.0 NVRAM + EK seed. **Wiping it looks like device tampering to any IdP** (Entra ID, Intune, Bitlocker-style policies) and forces re-enrollment. Never zero it casually. The per-VM state root is `3770` (setgid **+ sticky**) so a non-owner role UID cannot rename/replace the `swtpm/` entry; an identity-bound, root-owned marker at `/var/lib/d2b/swtpm-markers/<vm>` makes a *previously-provisioned-then-missing/replaced* dir **fail the VM start closed** (`previously-provisioned-swtpm-state-missing`) rather than silently re-creating an empty TPM. The state directory's ACLs are asserted by `tests/unit/smoke/smoke-eval-tpm.nix`; the broker hardening by `packages/d2b-priv-broker/src/ops/swtpm_dir.rs` tests.

## USBIP passthrough

**Where:** `nixos-modules/components/usbip.nix` (eval-time gating) + broker `UsbipBindFirewallRule` + `SpawnRunner` (per-busid attach process supervised by `d2bd`)

Eval-time gating still scopes attach to opted-in envs (validated by `tests/unit/nix/cases/usbip-gating.nix`). At runtime, attach/detach runs through the broker - there is no per-env `d2b-sys-<env>-usbipd-*` socket. Misrouted attaches expose a YubiKey to the wrong env.

## GPU sidecar (graphics VMs)

**Where:** `nixos-modules/components/graphics.nix` + broker `SpawnRunner` for cloud-hypervisor on graphics VMs; pidfd handed back via `OpenPidfd` and supervised by `d2bd`

Graphics VMs run cloud-hypervisor with the GPU device attached. Restarting `d2bd` no longer terminates CH - pidfd handoff means the child outlives a daemon reconnect - but the broker spawn path is the only audited place CH is launched. Bypassing it breaks the audit trail. Validate the evaluated graphics shape with `tests/unit/nix/cases/video-contract.nix`.

## Video sidecar (graphics VMs)

**Where:** `nixos-modules/components/video/guest.nix`, `nixos-modules/processes-json.nix`, `pkgs/vhost-user-video/`, `packages/d2b-host/src/video_argv.rs`, broker `SpawnRunner{role: Video}`

`graphics.videoSidecar = true` is an explicit opt-in H264 decode path: guest `virtio_media` + patched Cloud Hypervisor `--vhost-user-media` + patched crosvm `device video-decoder --backend vaapi`. There is no per-VM video systemd unit, no stock crosvm/CH fallback, and no free-form video extra args. The video runner MUST use the dedicated `d2b-<vm>-video` principal, not `d2b-<vm>-gpu`, so broker/activation ACLs can deny host Wayland/PipeWire/Pulse sockets to video without breaking GPU cross-domain. The broker masks `/dev` for the video runner and exposes only the declared device allowlist: default `/dev/dri/renderD128`, plus `/dev/nvidiactl`, `/dev/nvidia0`, and `/dev/nvidia-uvm` only when `graphics.videoNvidiaDecode = true`. `virtio_media` is a guest module, not a host `/proc/modules` preflight requirement. Firefox/VA-API uses the separate experimental `graphics.virglVideo` GPU path; it is default-off and must not be treated as stable video-sidecar coverage. Validate evaluated shape with `tests/unit/nix/cases/video-contract.nix`; rendered argv and sandbox coverage lives in `packages/d2b-contract-tests/tests/minijail_swtpm_video.rs` and runs in the enforcing fixture-contract lane.

## UI color contract / niri backend

**Where:** `nixos-modules/ui-colors.nix`, `nixos-modules/niri-vm-borders.nix`, `docs/reference/ui-colors.{md,json}`, `tests/unit/nix/cases/niri-vm-borders.nix`, and sibling consumers such as `vicondoa/d2b-wlcontrol`

The compositor-agnostic `d2b.site.ui` / `d2b.envs.<env>.ui` / `d2b.vms.<vm>.ui` color model is the source of truth for host/env/VM/state colors. Generated `/etc/d2b/ui-colors.json` and `/etc/d2b/ui-colors.css` are public presentation metadata, not authz or policy inputs. Niri-specific settings belong only under `d2b.site.ui.compositors.niri`; do not add compositor-specific color source options. Keep the JSON schema, reference docs, GTK CSS `@define-color` names, and nix-unit artifact-shape tests in sync. Downstream tools must fail visibly but remain usable when the artifact is missing or malformed, without reading root-owned d2b state directly.

## ComponentSession capability boundary

**Where:** `packages/d2b-contracts/src/v3/component_session.rs`, `packages/d2b-session/`, `packages/d2b-session-unix/`

Authenticated transport evidence and attachment credits are consumed into a private single session owner; do not add a clone/accessor that lets callers reuse admission evidence. **`SessionAuthority` is sealed** by a private supertrait in a private module (`admission.rs`), so no crate outside `d2b-session` can implement it - that seal is load-bearing, because a foreign authority implementation is a direct path to minting a genuine admission. Prove exact Zone equality before every capability mint, and never expose a store path, socket, or handle through the session. These crates are tested but deliberately unwired from production listeners until the full authenticated registration path lands.

## Zone message bus boundary

**Where:** `packages/d2b-bus/src/{router,registry,authorization,streams,operations}.rs`, `packages/d2b-resource-api/src/adapter.rs`

Registration consumes the single-owner capability admission; comparing a clonable token is insufficient. Every route is exact, subject-bound, revision-bound, and Zone-checked before minting authority. There is no wildcard pub/sub and no direct store handle. `UnregisteredBusAdapter` is a deliberate unreachable seam and must remain unregistered until authenticated ComponentSession, the Zone bus, and Zone registration land together.

## Resource mutation seal

**Where:** `packages/d2b-resource-store/src/mutation_seal.rs`, `packages/d2b-resource-store-redb/src/`, and `packages/d2b-resource-api/src/`

The resource write boundary is a concrete, process-local capability. Its
invariants are:

1. `SealedMutation` has one constructor, `MutationSealIssuer::seal`.
2. `mutation_seal_pair` and `MutationSealIssuer::seal` each have one
   non-test call site in the resource API.
3. The store and redb crates never depend on the resource API or import RBAC
   evaluator types.
4. `RedbResourceStore::commit_verified` is the only mutating backend method
   and consumes `SealedMutation` by value.
5. The seal module has no test-only or non-test-only configuration branch.
6. The external seal harness forces the real resource boundary through the
   compiled test configuration rather than opening a test escape hatch.
7. Seal types do not implement formatting, serialization, or comparison
   traits, and the module renders no identity.
8. A UUID, authority address, or database path never appears in diagnostics,
   telemetry, or audit data.
9. A store UUID is the canonical `ResourceUid`; it is compared only as a
   diagnosable identity component after the private authority check.
10. `StoreSlot` is a bounded, deterministic composition correlator. It is
    never persisted, serialized, placed on the wire, or compared as identity.
11. Store opening checks Zone, store UUID, and slot agreement for the acceptor,
    and every startup error carries the slot of the store producing it.

The issuer is retained by the native authorizer and the acceptor crosses into
the concrete backend by value. A downstream crate can construct an inert pair
for a store it owns, but it cannot open evidence against a store instance
whose acceptor it does not hold.

## Authoritative subject resolution

**Where:** `packages/d2b-bus/src/router.rs` (`ZoneRegistrar`), `packages/d2b-session-unix/src/subject.rs`

`ZoneRegistrar` **exclusively owns and consumes** subject resolution: a peer is mapped to a subject from registrar-private state using verified peer evidence. There is no public subject-configuration type and no raw-claim registration path, and there must not be one - caller-supplied `subject_ref`/`subject_uid` are exactly how a component would name itself something it is not. Production currently fails closed because no authoritative resolver is wired, which is the intended state until the Zone runtime supplies one; do not "fix" that by accepting claims from the caller. This boundary moved several times before it closed, each time by reappearing as a public constructor or registrar mutator somewhere the guard was not looking, so it is enforced by the type-based mint-surface inventory and a compile-fail fixture rather than by convention.

## Capability mint surface allowlist

**Where:** `packages/d2b-api-surface/`, `tests/tools/api-surface-json.sh`, `tests/golden/api-surface/`, the source/mutation checks in `packages/d2b-bus/tests/public_mint_surface.rs`, and the capability definitions in `packages/{d2b-bus,d2b-resource-store,d2b-session,d2b-session-unix}/src/`

The **enforcing compiler leg** uses stable trait-solver ambiguity assertions in the defining crates. It rejects the enumerated `Clone`, `Copy`, `Default`, and `From` implementations for `ComponentSessionAdmission`, `VerifiedUnixPeer`, `SessionAcceptor<C>`, and `AuthenticatedComponentSession<C>` in every compiled configuration. Generic assertions catch unconditional blanket implementations; separate assertions cover `C = ()` and the workspace's `C = ComponentSessionAdmission` uses. They do not enumerate every bounded or downstream `From<X>` implementation, so private construction fields, sealed traits, instance identity, and consumed authority remain the primary boundary. The external-seals tests require `error[E0283]` plus `CapabilityMustNotImplementCloneCopyDefaultOrFrom`; fabrication fixtures require the construction diagnostic that proves private fields remain closed. The compiler-derived API leg builds one public and one private-plus-hidden rustdoc JSON census for the whole workspace under the pinned nightly, then validates exact public, capability-bearing, hidden-public, and explicit-impl snapshots through `d2b-api-surface`; this replaces the serial package-by-package HTML rustdoc loop. Regenerate those snapshots explicitly with `make api-surface-pin`. The **best-effort source leg** inventories explicit workspace impl and derive forms and compares them with `approved-capability-trait-impls.txt`. Module aliases and module-level globs resolve monotonically over a finite universe: parsed alias names form the binding universe, declared local module paths form the only target universe, explicit bindings shadow glob imports, conflicting glob results are ambiguous, and separate target/visibility and taint budgets bound the two fixed points. Capability propagation resolves every glob target through the completed module-alias fixed point, including renamed targets; a multiple-target result is ambiguous and fails closed. A target can never acquire a path outside that finite module set, so glob cycles cannot grow indefinitely. Capability relevance propagates through resolved aliases to every descendant module containing a discovered capability binding. Unknown glob destinations taint their importing module; that taint propagates through later glob re-exports and makes otherwise unclassified impl self types fail closed. Roots matching Cargo-declared dependency names are classified as external and import no local capability binding, so ordinary dependency globs remain accepted. Unresolved alias bindings imported by a glob remain tainted bindings and fail closed when used as an impl prefix. Block-local globs and impls carry lexical scope identities. The scanner accepts a same-scope direct module alias only when its target is resolved and no capability or tainted descendant is reachable; capability-relevant, ambiguous, unresolved, or otherwise unmodelled block-local glob aliases fail closed. This is intentionally not a claim of complete Rust glob resolution. Regression fixtures pin the terminating `a`/`b` glob cycle with explicit shadowing, nested re-export through glob, rejecting direct and grouped renamed glob targets, unresolved and two-hop glob taint, rejecting direct and grouped block-local capability globs, and accepting non-capability block-local and renamed-target globs. Existing direct, renamed, chained, cfg, raw-identifier, path-loaded, symlink, attribute, and duplicate-logical-module fixtures remain covered. The source leg also fails closed on generic or cfg-gated declared type aliases, cfg-gated renamed imports, unsupported aliases, lexically scoped capability aliases, unresolvable external modules, missing selected module files, and unrecognised module attributes. It does not perform general Rust name resolution, macro expansion, or `include!` expansion, and implementations outside the scanned workspace remain outside its claim. Approved snapshots retain rendered signatures for exact comparison; failure output uses fixed operation or syntax labels, package or crate identity, exit status, and crate-relative logical locations. Raw Cargo or rustdoc stderr, signature tokens, source text, attribute tokens, absolute scratch paths, and attacker-authored path literals are not emitted. The separate capability API inventory still propagates from fixed capability and claim identities through private field types. Widening any compiler seal or approved snapshot is a deliberate trust-boundary change requiring a stated reason. One census change is proposed and not yet landed: [ADR 0051](../adr/0051-security-key-semantic-backing-set.md) adds `ProjectionFactory::admits_backing_ref` and a `projection_protocol_version` accessor, adds the public `SemanticProjectionProtocolVersion` type and `LEGACY_ABSENT_PROTOCOL_VERSION` constant, narrows `ProjectionFactory::admits_export_target` to take the stored resource envelope instead of a bare `ResourceRef`, adds two `ProviderContractError` variants, and removes `SemanticContractError::BackingRefTypesUndetermined`. Additions, the signature change, and the removal are all two-way census entries. That record carries the stated reason; regenerate with `make api-surface-pin` and prove it with `make test-rust-api-surface`, and do not pin any of those changes without citing it.

## Resource controller effects boundary

**Where:** `packages/d2b-controller-toolkit/src/{runner,queue,context,result,owner_hints}.rs`, `packages/d2b-core-controller/src/{hints,dependencies,owner_reconcile}.rs`

Controller and core-reconciliation engines are test-only and unwired from the absent production store/watch dispatcher. An EffectPort call is permitted only after durable resource commit and consumption of the matching `CommittedRevisionProof`; abort, conflict, stale proof, or restart ambiguity cannot release an effect. Preserve per-resource single flight, bounded fair admission, deterministic owner/dependency propagation, and restart-safe idempotency when wiring the production path.

## Unsafe-local provider, launcher, and persistent-shell helper

**Where:** `nixos-modules/options-realms-workloads.nix`, `nixos-modules/unsafe-local-workloads-json.nix`, `packages/d2b-core/src/unsafe_local_workloads.rs`, `packages/d2b-contracts/src/unsafe_local_wire.rs`, `packages/d2b-unsafe-local-helper/src/{shell_runtime,shell_supervisor,shell_socket,output_ring,tty_exec}.rs`, and `docs/reference/unsafe-local-provider.md`

`unsafe-local` is explicit and default-denied. It runs only as the exact authenticated requesting uid and provides no isolation boundary. Public metadata never carries configured argv or shell policy; those come only from the integrity-pinned private bundle. A persistent-shell supervisor in a verified transient USER scope - not the reconnectable helper or d2bd - owns the login-shell PTY, bounded merged-output ring, attachment, and private same-UID listener. Ledger adoption preserves ambiguous sessions as degraded; teardown closes the PTY and signals only the exact re-verified scope. The helper-wide ring reservation is bounded, terminal responses transfer exactly one CLOEXEC stream fd, and shell names, supervisor ids, paths, environment, process/unit identity, and bytes stay out of Debug/errors/audit. Do not add cross-uid execution, a direct compositor fallback, VM state/network/device semantics, a root service, per-VM unit, broker op, free-form shell command, or broad same-UID cleanup.

## Manifest contract

**Where:** `docs/reference/manifest-schema.{md,json}` + `nixos-modules/manifest.nix`

Version-pinned via `manifestVersion`. Adding, removing, or renaming a per-VM field requires bumping the version, updating the schema, and noting it in the CHANGELOG. The `static.sh` md↔json drift gate catches partial updates.

## Manifest bundle - private artifacts

**Where:** `docs/reference/manifest-bundle.md` + `docs/reference/schemas/v2/*.json` + `packages/d2b-core/src/{bundle,host,processes,privileges,closures,minijail_profile}.rs` + `nixos-modules/{bundle,bundle-artifacts,host-json,processes-json,privileges-json,closures-json,minijail-profiles}.nix` + `packages/xtask/src/main.rs` (`gen-schemas`)

Sensitive bundle artifacts install at `root:d2bd` 0640 and ground every broker/sandbox/runner behaviour. `d2b-core` DTOs are canonical; `d2b._bundle` is the typed internal artifact table that owns JSON data, install names, classifications, and `/etc/d2b` materialization for every bundle artifact. Add new bundle artifacts through `nixos-modules/bundle-artifacts.nix` instead of hand-writing parallel install logic in each emitter. Committed schemas under `docs/reference/schemas/v2/` ARE the contract and the `tests/unit/gates/drift-check.sh` gate enforces `xtask gen-schemas` + `git diff --exit-code` through `make test-drift`. Breaking the schema without an intentional `bundleVersion`/`schemaVersion` bump silently breaks every downstream consumer.

## Control plane - `d2bd` + `d2b-priv-broker`

**Where:** `packages/d2b-contracts/**` + `packages/d2b-core/**` + `packages/d2bd/**` + `packages/d2b-priv-broker/**` (sibling workspace; `unsafe_code = "deny"` with quarantined `src/sys.rs` for fd-passing FFI) + `packages/d2b/**` + `docs/reference/{cli-contract,daemon-api,error-codes,privileges}.md` + the daemon Layer-1 gate set in `tests/static.sh`

The **only** persistent root surfaces the framework declares. `d2b-priv-broker.socket` is socket-activated: systemd creates/binds/listens/sets-ACL before the broker starts; the broker adopts fd 3 via `SD_LISTEN_FDS` and MUST NOT self-bind, self-fchmod, or self-fchown when `SD_LISTEN_FDS=1`. `d2bd.service` carries `Wants=d2b-priv-broker.socket` (not `Requires=`) so the daemon keeps serving while the broker is idle. The broker reloads the current bundle resolver per accepted request so it does not dispatch stale runner intents after a switch. The broker drops to the `d2bd` group and uses `SO_PEERCRED` at accept time for authz (launcher / admin / deny). Every host mutation flows through a typed broker op (cgroup v2 delegation, TAP/bridge lifecycle, `ApplyNftables`, `ApplyNmUnmanaged`, `ApplySysctl`, `UpdateHostsFile`, `ModprobeIfAllowed`, `UsbipBindFirewallRule`, `SpawnRunner`, `OpenPidfd`) and is recorded as an `OpAuditRecord` in `/var/lib/d2b/audit/broker-<utc-date>.jsonl` (root-owned `0640 root:d2bd`, append-only `O_APPEND`, daily rotation, 14-day default retention overridable via `d2b.site.audit.retentionDays`). Relevant enforcing coverage includes `tests/unit/nix/cases/broker-socket-activation.nix`, `tests/unit/nix/cases/broker-caps.nix`, and daemon startup integration tests under `packages/d2bd/tests/`. The legacy-unit policy lives in `packages/d2b-contract-tests/tests/policy_units.rs` and runs in the enforcing fixture-contract lane. See [ADR 0015](../adr/0015-daemon-only-clean-break.md).

## Storage lifecycle / restart / synchronization

**Where:** Planned generated contracts in `d2b-core::{storage,process_restart,sync}` + Nix emitters, broker storage/sync ops, daemon lifecycle DAG integration, and docs [ADR 0034](../adr/0034-storage-lifecycle-restart-and-synchronization.md) / [`docs/explanation/storage-lifecycle.md`](../explanation/storage-lifecycle.md)

Managed paths, restart adoption, locks, leases, cleanup, and degraded-state reporting are control-plane contracts. Normal daemon restarts are continuation events: do not broad-sweep `/run/d2b`; first re-discover adoptable runners from declared cgroup leaves, open fresh pidfds, verify identity, and quarantine/degrade ambiguity. Pidfds are not persisted. New advisory locks use OFD locks with `O_CLOEXEC`, explicit fd transfer only, and total acquisition order. The broker resolves storage/lock mutations from opaque bundle ids through anchored `openat2`/fd-relative path walking; daemon-owned ledgers are diagnostics, never repair authority.

## Eval-time assertions

**Where:** `nixos-modules/assertions.nix`

These are the framework's contract with consumers. Loosening one silently turns a previously-rejected misconfig into runtime breakage. New assertions need a matching case in `tests/unit/nix/cases/assertions.nix`.

## Guest-control exec session table

**Where:** `packages/d2bd/src/{exec_session,exec_session_real}.rs`, `run_exec_owner` in `packages/d2bd/src/lib.rs`, `packages/d2b/src/exec_client.rs`, `packages/d2b-contracts/src/public_wire.rs` (`ExecOp`/`ExecOpResponse`)

Arbitrary `d2b vm exec` is **admin-only**; configured `d2b launch` local-VM items may use the same detached guest-control backend with launcher authority because argv is resolved exclusively from the hash-verified private bundle. Both run through `d2bd` plus authenticated guest-control vsock to `guestd`. Attached exec uses the daemon's in-process **session table**: per-session workers own one authenticated guest-control client and proxy typed exec ops. **guestd runs every exec as the VM's workload user (`ssh.user`) inside a real PAM login session (`systemd-run --property=PAMName=login --uid=<user>`) - never as root; the wire `user` field is ignored and the target user is host-fixed, bare `argv[0]` is resolved by the workload user's login `PATH`, and each attached exec runs in a process-unique named transient unit (`d2b-exec-<…>.service`) that teardown stops via `systemctl kill` so a quiet command cannot outlive owner-disconnect, cancel, or the runtime ceiling. Operators elevate with `sudo` inside the session.** Detached non-TTY exec is enabled with `d2b vm exec -d <vm> -- <cmd>` and managed through VM-first verbs (`d2b vm exec <vm> list`, `logs <id>`, `status <id>`, `kill <id>`); command forms always require `--`, so those verb words remain valid VM names. Detached jobs and configured local-VM launches also run as the workload user, never root: the root detached runner only owns trusted slot/log files, re-validates the non-root uid before spawning the workload unit, and fails terminally rather than falling back to direct root execution. Guestd reconciles detached runner/workload units on startup, cleans orphaned workloads, and runs a periodic reaper for terminal records and retained logs; `kill` maps to idempotent two-phase `ExecCancel` (SIGTERM/grace/SIGKILL). There is **no per-VM systemd unit, no new broker op, and no SSH** - the guest owns the PTY; the host only flips termios for attached TTY via an RAII raw-mode guard restored on every exit/error/panic. The admin `SO_PEERCRED` check runs before arbitrary exec session setup; configured launch instead requires local launcher/admin authority and a trusted configured item. Old/non-guest-control generations fail closed (exit `70`) with no proxy and no SSH fallback. Session-table caps (global/per-UID/per-VM), detached slot/log quotas, and rate limits are enforced before connect/auth or create. Attached audit emits one redacted kind=critical session-establishment event (vm/peer_uid/tty); detached create/kill daemon audit carries only vm/peer_uid/action/result/exec_id, while configured-launch audit adds target/item/operation correlation without execution details. Opaque session handles, argv, stdio, env, cwd, and paths never reach any Debug/trace/audit/metric surface. Validate with the `exec_session`/`exec_client` hermetic test matrices.

## Unsafe-local persistent shells

**Where:** `packages/d2bd/src/{workload_dispatch,unsafe_local_helper,unsafe_local_terminal,shell_backend}.rs`, shell owner dispatch in `packages/d2bd/src/lib.rs`, `packages/d2b-unsafe-local-helper/src/{shell_runtime,shell_supervisor}.rs`, and `tests/host-integration/unsafe-local-helper.nix`

`d2b shell` remains **admin-only** for every provider. The CLI creates and manages qualified `shell-terminal.d2bus.org.ShellSession` resources through the authenticated Zone request path and proxies terminal bytes through the ProcessAttachClient named stream. The retired public `ShellOp` socket family and `unsafe-local-shell-v1` feature negotiation do not exist. Unsafe-local target identity and `defaultName`/`maxSessions` come only from the hash-verified private bundle. The daemon dispatches helper protocol v2 to the exact `SO_PEERCRED` uid, validates exactly one connected CLOEXEC stream fd, and multiplexes terminal protocol v1 behind a fresh opaque attachment handle. Named-stream close or client hangup detaches but never kills; typed `Kill` targets only the helper-verified transient user scope. Shells survive CLI, daemon, and helper reconnects while that scope and the non-lingering user manager live. User logout ends them by design. User scopes provide lifecycle ownership, **not containment from other processes with the same host uid**. There is no root unit, broker op, per-VM service, SSH path, host-shell fallback, direct-compositor fallback, or automatic replay after an ambiguous daemon timeout. Never log/audit/label shell names, supervisor ids, attachment handles, terminal bytes, helper diagnostics, PIDs, unit names, argv, env, cwd, or paths; audit may use configured target/peer uid and fixed digests, while metrics use closed provider/component/operation/outcome/error labels.

## Lifecycle permission group

**Where:** `nixos-modules/host-users.nix`

Membership in `d2b` + `SO_PEERCRED` at `public.sock` accept time is the **only** lifecycle authorisation surface. There is no polkit allowlist; wiring anything else into the group inverts the threat model. **Exception:** the guarded `ExecStop` shutdown hook runs as uid 0 and receives the narrow `HostShutdown` role, which is permitted only for `vmStop` during host-shutdown teardown (see `packages/d2bd/src/admission.rs`). This exception is scoped strictly: all other admin-only operations (exec, USB attach, key rotation, host prepare, audit export) are denied for this role. The daemon-restart continuation guard is preserved: `Restart=on-failure` restarts never receive `HostShutdown` treatment because the restarting daemon re-adopts runners and the shutdown hook only runs under systemd stop with a live `stopping` system state check.

## SSH key generation / rotation

**Where:** `nixos-modules/host-keys.nix`, `host-activation.nix`

The framework owns `${cfg.site.keysDir}/<vm>_ed25519`. `d2b keys rotate` MUST NOT touch consumer-supplied keys.

## virtiofsd sandbox model

**Where:** `nixos-modules/minijail-profiles.nix` (virtiofsdProfiles), `packages/d2b-priv-broker/src/sys.rs` (`clone3_spawn_runner` user-NS path), `nixos-modules/processes-json.nix` (argv emit)

virtiofsd profiles MUST declare zero host capabilities (`capabilities = []`), `requiresStartRoot = false`, and a `userNamespace` block mapping in-NS UID/GID 0 to the per-share principal. Normal VM shares map to `d2b-<vm>-runner`; the guest-control token share (`d2b-gctl`) maps to the narrower `d2b-<vm>-gctlfs` principal. The broker pre-establishes the user namespace via `clone3(CLONE_NEWUSER)` + `pipe2` sync + `/proc/<pid>/uid_map` writes BEFORE virtiofsd's first instruction runs. virtiofsd argv MUST include `--sandbox=chroot --inode-file-handles=never` and `--readonly` for every `readOnly` share (`ro-store`, `d2b-gctl`). Reintroducing host caps, `requiresStartRoot=true`, or `--sandbox=namespace` violates [ADR 0021](../adr/0021-broker-user-namespace-for-virtiofsd.md). Rendered profile and argv coverage lives in `packages/d2b-contract-tests/tests/minijail_roles.rs` and runs in the enforcing fixture-contract lane.

## cgroup slice naming and ownership markers

The privileged broker's host-prepare dispatch (see the Control plane
row above) carries two operational conventions that ground every
broker op mutating host state.

### cgroup slice naming

- Single canonical slice: **`/sys/fs/cgroup/d2b.slice`** (no
  `system-` prefix, no `d2b-launcher.slice` parent). The broker
  creates it on `host prepare --apply` if absent.
- Per-VM directories live one level below the slice:
  `d2b.slice/<vm>/<role>/`. The VM layer is **process-free**; only
  the per-role leaves hold processes.
- Delegation: the broker `fchown`s the delegated subtree (the
  `d2b.slice` directory and every descendant) to the `d2bd`
  system user. The host cgroup root is never chowned.
- Forbidden surfaces: writing `cpuset.cpus.partition` on
  d2b-owned cgroups (the cgroup v2 root and other ancestors
  are out of scope; d2b never reads/writes them), threaded
  cgroups, `cgroup.kill` on `d2b.slice` or any ancestor of
  a daemon-owned leaf, and **Phase B (post-delegation) runtime
  mutation while running as uid 0** (Phase A privileged setup -
  `+controllers` cascade, slice/leaf `mkdir`, `fchown` to
  `d2bd`'s uid/gid - legitimately runs as root per ADR 0011
  Decision item 2; the uid != 0 invariant applies to the
  steady-state cgroup code path after privilege drop). See
  [`docs/reference/cgroup-delegation.md`](../reference/cgroup-delegation.md)
  and ADR 0011 for the algorithm + audit shape.

### Ownership-marker conventions

The broker writes its host mutations inside greppable ownership
markers so foreign-rule preservation can be enforced fail-closed:

| Surface | Marker shape |
| --- | --- |
| nftables (`inet d2b` table) | every rule + chain carries `comment "d2b managed: <ownership-id>"`; foreign tables are never flushed |
| `/etc/hosts` | block delimited by `# d2b-managed begin` and `# d2b-managed end`; foreign lines outside the block are byte-preserved |
| NetworkManager unmanaged config | `/etc/NetworkManager/conf.d/00-d2b-unmanaged.conf`, contents delimited by `# d2b-managed begin` / `# d2b-managed end` |
| systemd-networkd | detection-only; coexistence requires an operator-shipped configured-unmanaged file matching the `d2b-`/`d2bv-` prefix (no d2b write) |

Discovering a foreign ownership marker where d2b expects its own
is fail-closed (`path-safety-violation`,
`nm-managed-foreign-conflict`, `foreign-nft-rule-preserved`). See
[`docs/explanation/host-prepare.md`](../explanation/host-prepare.md)
§ "NetworkManager / systemd-networkd coexistence" and ADR 0013 for
the rationale.
