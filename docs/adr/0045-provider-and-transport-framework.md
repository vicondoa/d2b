# ADR 0045: d2b 2.0 provider and transport framework

- Status: Accepted
- Date: 2026-07-10
- Target release: 2.0.0
- Refines: [ADR 0035](0035-efficiency-and-simplification-roadmap.md)
  (provider naming and workspace simplification)
- Supersedes for d2b 2.0: [ADR 0010](0010-wire-protocol-and-typed-errors.md),
  [ADR 0015](0015-daemon-only-clean-break.md),
  [ADR 0028](0028-guest-control-plane-over-vsock.md),
  [ADR 0032](0032-d2b-v2-constellation-control-plane.md),
  [ADR 0034](0034-storage-lifecycle-restart-and-synchronization.md),
  [ADR 0042](0042-d2b-clipboard-authority-and-picker-split.md),
  [ADR 0043](0043-realm-native-control-plane.md), and
  [ADR 0044](0044-unsafe-local-runtime-provider.md), as specified in
  [Normative precedence](#normative-precedence)
- Related: [ADR 0037](0037-local-hypervisor-runtime-seam.md)
  (local hypervisor runtime seam)

## Context

D2b 1.x has several individually reasonable control-plane designs that no
longer compose into one security or ownership model:

- the public daemon, broker, realm peers, guest control, unsafe-local helper,
  shell supervisor, clipboard components, desktop helpers, and security-key
  frontend use different framing, authentication, version, limit, error, and
  file-descriptor rules;
- the nominal provider traits are separate from the production lifecycle paths
  in `d2bd`, the broker, gateway crates, user helpers, and component-specific
  controllers;
- the live filesystem is largely host-global and VM-name keyed, with repair
  authority split among activation, tmpfiles, daemons, and the broker;
- ADR 0043 requires separate realm controller and broker boundaries, while the
  shipped three-unit model still assumes one host-global daemon and broker;
- guest control uses a long-lived shared token and HMAC instead of an enrolled
  workload identity;
- the client toolkit duplicates public wire contracts, and there is no
  canonical provider SDK distribution;
- interactive user secrets, unattended service credentials, and host-local
  realm identities do not have one explicit lifecycle.

An incremental transition would require every new authority boundary to retain
old parsers, state readers, aliases, protocol negotiation, and migration
branches. That would make the new design depend permanently on the behavior it
is intended to remove.

This ADR therefore defines d2b 2.0 as a destructive architectural replacement,
not as an upgrade layer over d2b 1.x.

## Decision summary

D2b 2.0 has the following fixed decisions:

1. The host is factory-reset for all d2b state from a persistently selected,
   dedicated v2 reset boot generation. Before selecting that generation, every
   configured user completes a mandatory live-session preparation in which
   `d2b-userd` deletes d2b-owned Secret Service items, revokes scoped TPM
   exports, and emits a root-verifiable receipt bound to the reset intent.
   Reset mode never accesses or unlocks Secret Service and refuses without the
   exact receipt set. No d2b 1.x backup is retained, and a reset interruption
   boots back into reset mode rather than v1.
2. There is no d2b 1.x parser, importer, alias, tombstone, re-export, protocol
   negotiation path, compatibility feature, or fallback in production code.
3. Every d2b-owned live IPC boundary uses one authenticated
   `ComponentSession` contract, including local Unix seqpacket boundaries and
   `SCM_RIGHTS` attachment transfer. Local NN identity evidence is directional;
   absolute expiry is distinct from relative ttrpc timeout; request-ID
   cancellation, readiness-correct nonblocking I/O, and aggregate FD credits
   with close-once truncation cleanup are mandatory.
4. `snow` supplies fixed Noise profiles. `ttrpc-rust` with `rust-protobuf`
   supplies typed control RPC inside the session. The d2b named-stream mux
   supplies bounded data channels.
5. Guest bootstrap uses a one-time operation-bound PSK only. Normal guest
   sessions use a guest-generated static Noise key sealed to that workload's
   vTPM. File-backed swtpm state is explicitly cloneable by host root and is not
   accepted as anti-cloning evidence or as a realm-controller host posture.
6. Eleven typed provider authorities replace catch-all and synthetic provider
   facades: runtime, infrastructure, transport, substrate, credential, display,
   network, storage, device, audio, and observability. Provider IDs are globally
   unique, factories are unique by type/implementation pair, and Azure VM
   infrastructure and runtime authority do not overlap.
7. ADR 0043's per-realm `d2bd`, broker, identity, state, audit, cgroup, and
   resource partition is implemented literally. The local-root broker retains
   host-global authority and is the only broker socket-activated by PID1. Its
   allocator pre-binds child listeners and parent-spawns each child controller
   and broker as separate pidfd-supervised processes through typed operations,
   passing only their listener, namespace, cgroup, resource, and lease FDs.
   Child processes are not PID1 units.
8. Dynamic paths use deterministic 96-bit, domain-separated SHA-256 IDs over a
   canonical printable-ASCII length-prefixed grammar that Nix passes directly
   to `builtins.hashString "sha256"`. IDs are rendered as 20 lowercase unpadded
   RFC 4648 base32 characters. Human names never become runtime path components.
9. Brokers are the only creators and repair owners of dynamic paths below fixed
   filesystem anchors. PID1 creates only the closed local-root endpoint set;
   the owning user manager creates the closed per-user set, and the local-root
   allocator creates child-realm listeners. Authoritative state remains
   bounded, versioned, atomic JSON.
10. GNOME Keyring is the interactive Secret Service. `d2b-userd` uses `oo7`.
    TTY unlock directly invokes the stdin-only keyring backend without a
    shell. Unattended services receive only explicitly exported TPM2-sealed
    credentials materialized by systemd under `$CREDENTIALS_DIRECTORY`; TPM
    sealing does not cryptographically identify the receiving unit.
11. All host and guest Rust crates use one Cargo workspace, one lockfile, and
    workspace version `2.0.0`. Canonical client and provider toolkit crates live
    in this repository.
12. Delivery uses Git Town ordinary PR stacks, Rust `xtask`, immutable tree snapshots, concurrent
    validation and panel lanes, and a tree-bound mechanical seal. W0 bootstraps
    equivalent evidence before `xtask` exists and requires one panel on the
    Proposed tree plus a second full panel on the Accepted/index candidate.
    Delivery ends with a streamline wave that converts friction recorded during
    the preceding waves into tested tooling and process improvements.

These are requirements, not options deferred to implementation. In particular,
retaining d2b 1.x compatibility is not a reviewable alternative. Review may and
must find destructive-safety, completeness, authorization, cryptographic,
resource-bound, and operational defects. A recommendation whose requested
remedy is to restore a d2b 1.x compatibility surface is out of scope.

## Normative precedence

Historical ADRs remain useful records. For d2b 2.0, this ADR controls whenever
one of the following decisions conflicts with it:

| ADR | d2b 2.0 precedence |
| --- | --- |
| [ADR 0010](0010-wire-protocol-and-typed-errors.md) | Supersedes the 4-byte-length JSON public/broker wire, semver range hello, feature intersection, and permissive unknown-feature rule. Retains strict bounded decoding, stable typed errors, redaction, and mediated audit access, now under `ComponentSession` and `.v2` services. |
| [ADR 0015](0015-daemon-only-clean-break.md) | Supersedes the one-host-daemon, one-host-broker, and exactly-three-root-visible-units invariant. Retains daemon-owned lifecycle DAGs, no per-workload systemd templates, broker-mediated privileged mutation, no Bash fallback, and acceptor-side `SO_PEERCRED` local admission. Initiators authenticate responders through trusted endpoint provenance as defined here. Those invariants now apply independently at each host-local realm boundary. |
| [ADR 0028](0028-guest-control-plane-over-vsock.md) | Supersedes the guest HMAC challenge, long-lived guest-control token, protocol-6/schema-v2 wire, chunked unary compatibility assumptions, SSH mixed-generation window, and old-generation state preservation. Retains guestd, workload-user execution, bounded RPC, typed errors, and Cloud Hypervisor vsock as a transport beneath `d2b.guest.v2`. |
| [ADR 0032](0032-d2b-v2-constellation-control-plane.md) | ADR 0043 already superseded its host-centric entrypoint. This ADR also supersedes its old target, protocol, provider, state, and migration generations. It retains strict realm-tree policy, semantic operations and streams, relay-as-reachability-only, capability denial, idempotency, and the prohibition on host-held realm provider credentials. |
| [ADR 0034](0034-storage-lifecycle-restart-and-synchronization.md) | Supersedes flat VM-name paths, preservation of d2b 1.x persistent state, rollback migration, and old storage IDs. Retains broker-resolved opaque storage/sync IDs, anchored path resolution, OFD locks, explicit FD transfer, restart adoption before cleanup, quarantine, and durable atomic JSON after the reset. |
| [ADR 0042](0042-d2b-clipboard-authority-and-picker-split.md) | Supersedes newline-delimited JSON picker/bridge wires, protocol-v1 hello, raw VM path components, and any specialized IPC exception. Retains d2b-owned clipboard authority, an untrusted UI-only picker, metadata-only audit, policy recheck, exact FD validation, and no direct guest clipboard authority. Clipboard control and transfer now use `d2b.clipboard.v2` and `d2b.clipboard.picker.v2` sessions. |
| [ADR 0043](0043-realm-native-control-plane.md) | Retains and makes literal one controller/broker boundary per host-local realm, strict parent/child policy, provider-neutral operations, and relay non-authority. Supersedes bare aliases, node-qualified compatibility targets, migration/import parsers, option tombstones, additive old-schema transition, and preservation of old local VM/controller state. The narrow independently-running-controller exception is defined below. |
| [ADR 0044](0044-unsafe-local-runtime-provider.md) | Retains exact-user execution, verified user scopes, no root launch, no isolation claim, no proxy bypass, and no ambient fallback. Supersedes `unsafe-local` as a runtime implementation ID, its specialized helper protocols, additive host-launcher migration, and compatibility fallback. The implementation ID is `systemd-user`; `unsafe-local` remains only the required no-isolation posture and warning. |

This table is normative. Text in an older ADR that promises preservation,
import, compatibility diagnostics, side-by-side mixed generations, aliases, or
specialized local wires does not apply to d2b 2.0.

## Destructive cutover

### No compatibility surface

D2b 2.0 removes, rather than deprecates:

- d2b 1.x configuration readers and removed-option modules;
- d2b 1.x state, bundle, manifest, ledger, and audit readers;
- d2b 1.x public, broker, guest, realm, provider, helper, clipboard, picker,
  notify, and security-key protocol decoders;
- old crate re-exports, package aliases, Nix aliases, share-path aliases, and
  compatibility wrapper binaries;
- old CLI verbs, target resolvers, bare-name aliases, feature strings, and
  fallback behavior.

Unknown old configuration reaches the normal unknown-option path. Unknown old
CLI syntax reaches the normal unknown-command or invalid-target path. Old bytes
fail the fixed v2 preface before semantic dispatch. Production code does not
recognize an input as "v1", recommend an old-to-new translation, or import any
old state. Release and migration documentation may explain that the cutover is
destructive; the executable does not carry a legacy diagnosis engine.

There are no compatibility symlinks, tombstones, state markers, parsers, aliases,
or protocol feature flags. There is no mixed authenticated d2b 1.x/d2b 2.0
topology and no side-by-side drain mode inside d2b.

`lib.mkRemovedOptionModule` is explicitly forbidden. It registers an old option
path and a tailored compatibility diagnostic, so it is an option tombstone even
when it returns only an error. Old option paths remain undeclared and therefore
produce Nix's generic unknown-option error. The release notes and migration guide,
not an executable parser or module tombstone, explain the replacement surface.

### Mandatory destructive-cutover acknowledgement

Every d2b 2.0 host configuration must set:

```nix
d2b.acceptDestructiveV2Cutover = true;
```

This is a new v2-only Boolean option with a default of `false`. Importing or
enabling the v2 framework while it is omitted or false fails module evaluation
before a bundle, operational unit, or reset closure is emitted. Reset-generation
construction also requires the evaluated true value and records it in the
generated v2 reset policy consumed by the reset binary. The acknowledgement
means that the operator accepts destruction of all d2b 1.x state, workload disks,
TPM state, keys, credentials, audits, and sessions with no rollback.

The acknowledgement is necessary but cannot cause deletion. The persistent reset
boot, dry-run, digest confirmation, lock, inhibitor, quiescence, and final-boot
rules below remain mandatory. It does not declare an old option, parse old
configuration, select a compatibility mode, or recognize old state. A
configuration that also contains an old option still fails through the generic
unknown-option path.

### Factory reset trigger

The host-tree reset does not run as a live switch from a v1 generation. W11
builds two closures before touching state:

1. the complete final v2 boot generation; and
2. a dedicated v2 reset boot generation whose default target is
   `d2b-reset.target`.

Before that reset generation is selected, W11 performs a mandatory pre-reset
user phase in the ordinary multi-user generation while every configured
owning user has an authenticated login session and an already-unlocked
keyring. The phase uses the final v2 `d2b-userd` reset-preparation path; it does
not require a v1 parser or preserve any v1 state. The root coordinator first
constructs a canonical reset intent containing the reset and final closure
fingerprints, a fresh 256-bit nonce, the exact UID set from final-v2
configuration, fixed d2b Secret Service ownership selectors, scoped-export
inventory digests, the generated deletion-anchor classes, and the private
outlier-manifest digest when one exists. The non-mutating command:

```text
d2b host reset --factory --prepare-users
```

emits the ordered intent and its digest. The only mutating pre-reset command
is:

```text
d2b host reset --factory --prepare-users --apply --confirm <reset-intent-digest>
```

After confirmation and before deleting an item, the coordinator quiesces every
operational d2b writer that can create a Secret Service item or scoped export
while leaving the owning desktop/login sessions and keyrings available. It
admits only itself and one bounded final-v2 user preparation at a time. This is
not a mixed v1/v2 control topology: no v1 session or parser participates.

For each configured UID, the coordinator parent-starts exactly that user's
final-v2 `d2b-userd` reset-preparation invocation through the owning user
manager with a fresh inherited `d2b.user.v2` endpoint. `d2b-userd`:

1. requires the owning login session and unlocked Secret Service to be
   available; it never prompts for or attempts a keyring unlock;
2. selects only items carrying the fixed d2b ownership attributes, deletes
   them through `oo7` without reading secret values, and re-queries the fixed
   selector to prove absence;
3. revokes every configured scoped TPM export for that UID through the typed
   reset-preparation operation, waits for removal of its root-owned
   materialization and authorization record, and proves the expected export
   inventory absent; and
4. emits a bounded completion-receipt payload over the reset-intent digest and
   nonce, UID, selector version and result digest, expected export-inventory
   digest and revocation-result digest, and its executable/configuration
   generation.

The root coordinator verifies the `d2b-userd` peer UID, launch binding,
ComponentSession transcript, result digests, and export-removal evidence. It
then MACs the payload with a one-use reset-receipt key and stores the resulting
root-verifiable `d2b-userd` completion receipt as a root-owned `0600` file
outside every deletion anchor. The key and receipt set are carried into reset
mode as root-only reset evidence; they authorize no operation other than
receipt verification. A receipt is valid only for its exact reset-intent
digest, nonce, UID, inventories, and closure fingerprints.

After the exact configured-UID receipt set exists, the coordinator closes a
root-owned pre-reset barrier: new d2b Secret Service writes and scoped exports
are denied, all prepared login sessions/user managers/keyring daemons are
terminated, no process or user bus remains for a configured UID, every d2b user
process is stopped, and all operational d2b user units are masked for the reset
boot. The root coordinator survives operator-session loss and owns the
remaining transition. If any user/session/keyring is absent during its phase,
an item or export cannot be removed, a receipt is missing/duplicate/stale, or
the barrier cannot close, the reset generation is not selected. Aborting the
phase invalidates all receipts and requires a new nonce and complete rerun;
there is no partial-receipt resume.

The reset generation contains the v2 reset binary and its audited data, but no
startable v1 or v2 d2b daemon, broker, socket, provider, helper, user-agent, or
runner service. Every operational d2b system/user service and socket unit known
to that private closure is absent or masked; the reset target itself is the only
d2b-named unit permitted to be active, and it starts no d2b service. The reset
entry is installed, marked bootable, and selected as the persistent bootloader
default before reboot. It is not a one-shot entry and boot counting may not fall
back to an older generation. The prebuilt final v2 generation is present but is
not selected. A power loss, kernel panic, reset-tool crash, failed apply, or
ordinary reboot therefore returns to reset mode, never to v1.

The two host-tree reset commands run only after that reboot. The dry-run
command is:

```text
d2b host reset --factory
```

It emits a bounded deletion plan and a digest over the ordered, canonical plan.
That plan includes the reset-intent digest and the ordered digest of the
verified per-user receipt set. The only reset-mode command shape that may
mutate the remaining host trees is exactly:

```text
d2b host reset --factory --apply --confirm <plan-digest>
```

The apply command:

1. verifies the running generation and target fingerprint for the preselected
   reset closure, effective uid 0, and the absence or mask state of every d2b
   operational system/user service and socket unit; it never mutates through a
   broker;
2. validates the root ownership and mode of the reset evidence, authenticates
   every receipt, and requires exactly one receipt for every configured UID
   with the current intent digest, nonce, inventories, and closure
   fingerprints; it does not start a user process, connect to D-Bus, access
   Secret Service, or attempt a keyring unlock;
3. acquires the exclusive host reset OFD lock at
   `/run/d2b-reset/host.lock`, then acquires a systemd-logind block inhibitor
   for shutdown, reboot, sleep, and idle for the complete mutation and
   verification interval;
4. under that lock, proves that every declared d2b cgroup is unpopulated and the
   exact fingerprinted reset process is the only d2b process; no
   ComponentSession, provider/helper/user session, runner, lease, or delegated
   namespace remains; no process has an fd, cwd, root, executable, map, or
   mount-namespace reference into a deletion root; and no mount exists at or
   below a deletion root;
5. reopens and holds every generated parent/root from its immutable anchor with
   `openat2`, `O_DIRECTORY | O_CLOEXEC`, `RESOLVE_BENEATH`,
   `RESOLVE_NO_SYMLINKS`, `RESOLVE_NO_MAGICLINKS`, and `RESOLVE_NO_XDEV`;
   records and compares inode, device, and `statx` mount ID, canonicalizes the
   complete ordered plan, recomputes its digest while the reset lock and
   inhibitor are held, and requires exact equality with `--confirm`;
6. accepts the authenticated receipts as the proof that Secret Service items
   were deleted, directly proves root-owned scoped-export material absent, then
   deletes all remaining declared d2b runtime, persistent, cache, generated
   configuration, user-helper, key, token, disk, TPM, store-view, audit,
   ledger, socket, and lock state as opaque trees;
7. recursively walks only through those held dirfds: it opens directory entries
   with the same `openat2` resolve flags, enumerates only through the opened
   directory FD with `getdents64`, removes non-directory leaves (including
   symlinks themselves) with `unlinkat(parent_fd, name, 0)`, and removes an
   emptied directory with `unlinkat(parent_fd, name, AT_REMOVEDIR)` after closing
   its child dirfd. `unlinkat` never follows a final symlink. Inode, device, and
   mount-ID observations are revalidated as alarms, not treated as atomic unlink
   identity authority. A mount point or mount crossing, including `EXDEV` from
   `RESOLVE_NO_XDEV`, is reported as a fatal `EBUSY` and is never traversed.
   `ENOTEMPTY` permits at most two complete re-enumerations of that directory; a
   third `ENOTEMPTY` fails the reset;
8. never decodes a v1 record and never touches `/etc/nixos`, unrelated Secret
   Service items, unrelated user files, unrelated systemd credentials, or a
   non-d2b mount;
9. creates no archive, snapshot, copied tree, rollback generation, or recovery
   token;
10. fsyncs every affected directory, re-runs the receipt, quiescence, and
    absence proofs, and writes bounded reset-complete evidence outside the
    deleted anchors;
11. only after all prior steps succeed, atomically selects the already-built
    final v2 generation as the persistent boot default and reboots into it. It
    does not live-switch services from reset mode.

The exclusive reset lock, the quiescence proof, non-writable anchors, and held
directory FDs are the deletion synchronization contract and remain in force for
the complete traversal. No `fstatat`-then-`unlinkat` sequence is treated as an
atomic identity check. A `/proc/self/mountinfo` snapshot and
`STATX_MNT_ID_UNIQUE`, when the running kernel supplies it, are additional
pre/post traversal alarms for an unexpected topology change only; neither
authorizes deletion nor proves the identity of the object removed by a later
`unlinkat`.

The shutdown inhibitor is released only after the final boot selection is
durable or after a failure has left reset mode selected. If clean shutdown
cannot be inhibited, reset refuses before deletion. SIGTERM, cancellation, or
an operator disconnect before final selection leaves the reset entry selected.

The plan includes d2b-owned user state, but Secret Service deletion and scoped
export revocation are completed only in the mandatory pre-reset user phase.
Reset mode treats the authenticated receipts as required evidence and the
corresponding item/export absence as a precondition; it never opens a user bus,
forks a user child, invokes `oo7`, accesses a keyring, or attempts an unlock.
No d2b process other than the exact reset process may remain live.

The released reset implementation knows only generated current-v2 d2b anchor
classes. Literal names for legacy outlier roots are not permitted in final v2
production code, configuration, schemas, tests, or binaries; historical ADR
prose may record them as history. If the physical host has an outlier outside
those anchors, W11 may place its literal path in exactly one audited, data-only
reset manifest embedded only in the private reset closure. The manifest is a
closed list of anchored deletion roots; it contains no record shape, migration
rule, decoder choice, rename, or import action and is never a v1 parser or
importer. The private manifest, its closure GC roots, and every outlier literal
are deleted before the W11 final seal and may not enter W12 or a release
artifact.

Workload disks, swtpm/vTPM state, SSH and Noise keys, store views, audit history,
provider caches, and detached or persistent shell state are intentionally lost.
Realms and workloads are recreated from v2-only configuration with new
identities.

### Repair forward

There is no rollback archive and no d2b 1.x repair path. Before reset, W11 builds
the reset and final v2 closures, verifies the reset plan in isolated
validation, and completes the mandatory user receipt phase. A failed user phase
invalidates its partial receipts and is rerun with a fresh nonce; deleted d2b
items or revoked exports are not restored from v1. During reset, failure returns
to the persistently selected reset generation. After final-v2 selection, any
physical-host failure is repaired forward on the private integrated v2 branch.
Neither path boots a d2b 1.x generation.

## Realm process and authority model

### Per-realm control planes

Every host-local realm has all of the following:

- one `d2bd` controller process and public socket;
- one separate broker process and broker socket, with privilege confined
  according to the local-root/child split below;
- a distinct system user/group and local access policy;
- a distinct Noise identity;
- a distinct persistent state root and runtime root;
- a distinct append-only audit domain;
- a distinct delegated cgroup subtree;
- a distinct network namespace and resource partition when it claims network
  isolation;
- typed provider registries owned by that controller.

The physical-host topology begins as:

```text
local-root d2bd (PID1 service)
  local-root broker (the only PID1 socket-activated broker)
    host-global allocator
    parent-spawn of child processes
  resolver and realm-tree policy
  pidfd supervision/adoption of child controllers and brokers

home d2bd process + separate home broker process
dev d2bd process + separate dev broker process
work d2bd process + separate work broker process

per-user d2b-userd
per-user d2b-provider-runtime-systemd-user-agent
```

The migrated realm tree is:

```text
local-root
  home
  dev
    personal-dev
  work
    interactive work desktop and provider executor
```

The local-root instance keeps the unsuffixed units:

```text
d2bd.socket
d2bd.service
d2b-priv-broker.socket
d2b-priv-broker.service
```

`d2b-priv-broker.socket` is the only PID1 broker socket activation.
`d2bd.socket` remains the fixed local-root public endpoint; it does not activate
or supervise a child realm.

A child host-local realm has no `.socket` or `.service` unit. It uses its
derived ID in process identities and broker-owned listeners:

```text
controller uid: d2bd-r-<realm-id>
broker uid:     d2bbr-r-<realm-id>
public socket:  /run/d2b/r/<realm-id>/public.sock
broker socket:  /run/d2b/r/<realm-id>/broker.sock
```

The child controller system user is `d2bd-r-<realm-id>`, its child-broker system
user is `d2bbr-r-<realm-id>`, and its local public access group is
`d2b-r-<realm-id>`. The controller and broker UIDs are distinct. A separate
internal `d2bcg-r-<realm-id>` group contains only those two identities and
owns the narrow cgroup delegation described below; the public access group is
never a cgroup owner. The local-root identities remain `d2bd` and `d2b`.

The local-root allocator creates this cgroup layout before either child
process executes:

```text
/sys/fs/cgroup/d2b.slice/r-<realm-id>/
  controller/
  broker/
  workloads/
    w-<workload-id>/
      <role-id>/
```

The realm root, `workloads/`, and every `w-<workload-id>/` are process-free.
The controller is born directly in `controller/`, the broker is born directly
in `broker/`, and workload processes appear only in role leaves. There is no
controller or broker process in a sibling cgroup outside the delegated realm
root.

The allocator grants `d2bcg-r-<realm-id>` write access to the cgroup-v2
delegation files at the realm common ancestor and throughout
`workloads/` only. Ancestors grant only the execute/search permission needed
to reach that root, never write permission on `/sys/fs/cgroup/d2b.slice`, the
cgroup root, or another realm.
Controller and broker user-namespace maps include that one internal GID. New
runners use `clone3(CLONE_INTO_CGROUP)` with an already-validated role-leaf FD
so their first instruction executes in the destination leaf. An equivalent
privileged spawn may be used only if the child is blocked before its first
instruction and the local-root broker places it directly in the destination.
After initial controller/broker placement, any `cgroup.procs` move is confined
to source and destination leaves beneath the same
`r-<realm-id>/` root; moving a process through `d2b.slice`, the cgroup root, or
a peer realm is forbidden.

A child controller, public group, broker socket, state root, delegated cgroup,
and internal cgroup group may not be shared with another realm even when both
realms have the same allowed users.

The user manager similarly owns fixed `d2b-userd.socket`,
`d2b-runtime-systemd-user.socket`, `d2b-clipd-control.socket`,
`d2b-clipd-picker.socket`, and `d2b-clipd-bridge.socket`. Unit provenance is
part of the generated endpoint row; a service cannot substitute a self-bound
listener.

No per-workload systemd service is introduced. A realm controller supervises
its workload DAGs. Its broker performs that realm's privileged effects. Unit
count does not scale by host-local realm or workload; child process count and
pidfd state scale by realm, and workload runner count scales by workload.

### Local-root allocator

The host-global allocator is a subsystem of the local-root privileged broker,
not a separate daemon and not a shared child-realm broker. The local-root broker
is the only broker that retains all d2b-required initial-namespace/global
capabilities and global host path/device access. It owns global claims such as:

- cgroup subtree delegation;
- bridge, veth, TAP, interface, subnet, and namespace allocation;
- shared nftables and host-file serialization;
- global device and scarce-resource partitions;
- cross-realm collision checks.

For each child realm generation, the local-root controller requests closed
typed allocator operations. The local-root broker:

1. creates the realm roots and cgroup layout, opens the exact namespace,
   cgroup, storage, device, and lease FDs, and pre-binds both the child public
   listener and child broker listener from generated endpoint rows;
2. creates dedicated user, mount, network, IPC, PID, and cgroup namespaces with
   only that realm's generated UID/GID maps and resource views;
3. invokes the typed child-controller and child-broker spawn operations as
   separate `clone3` children, places them directly in `controller/` and
   `broker/`, and passes each process only its declared listener, namespace,
   cgroup, resource, and bootstrap-session FDs;
4. returns distinct pidfds and launch records to the local-root `d2bd`, which
   supervises both processes, adopts them after its own restart only after
   cgroup/executable/generation verification, and requests a typed respawn on
   failure.

The controller and broker are separate children with separate UIDs, FD tables,
listeners, ComponentSessions, state roots, and audit roots. Neither is a PID1
unit, neither receives `SD_LISTEN_FDS`, and neither self-binds or repairs its
listener. PID1 cannot independently restart one behind the allocator's
namespace or cgroup setup.

Child brokers run under distinct dedicated unprivileged host UIDs. Before any
child-broker code executes, the local-root launch path has installed the
dedicated namespaces and mapped only that realm's generated UID/GID ranges.
The child starts with an empty initial-namespace permitted, effective,
inheritable, ambient, and bounding capability set. Any capabilities granted
inside its user namespace are a closed generated namespace-scoped set and
confer no authority in the initial user, mount, network, IPC, PID, or cgroup
namespace.

A child mount namespace exposes its immutable executable closure and minimal
pseudo-filesystems, but no traversable global host root or ambient `/dev`.
Allocator-approved namespace, storage, cgroup-leaf, and device authority arrives
only as validated `O_CLOEXEC` dirfds/FDs plus typed, revocable lease IDs over
`d2b.broker.v2`. The child may perform only the namespace-local operation named
by a live lease. It cannot reopen a delegated object by global path, call
`setns` into an initial namespace, create a broader ID mapping, retain an
initial-namespace capability, or discover another realm's resources.

Host-global mutation remains a local-root broker operation. This includes
creating/delegating cgroup subtrees and namespaces, opening global devices,
loading modules, changing host sysctls or host files, allocating bridge/veth/TAP
and nftables state, and serializing a shared host surface. A child submits a
typed bounded request; local-root validates policy and either returns a narrow
lease/FD or performs the global mutation and returns a non-authoritative
observation. It never gives a child a global path or an initial-namespace
capability.

A lease identifies a generated resource ID, owning realm generation,
capability, limits, and expiry/revocation policy. It does not contain a raw path,
free-form command, credential, or unbounded kernel object name. Child brokers
may mutate only resources in live leases delegated to their realm. Two claims
for one exclusive resource fail before side effects.

This is a distinct child-broker security boundary rather than a realm tag in a
root process: each child has a separate uid, user/mount/network namespace,
socket, state and audit root, capability set, FD table, and allocator lease set,
and the kernel denies access outside those objects. Local-root allocation does
not make local-root the peer realm's semantic authorization proxy. Clients
connect directly to a host-local realm public socket after resolution,
preserving that realm's local admission, OS DAC, attachment, state, and audit
boundary.

### Controller workload ownership and the narrow exception

The default acyclic rule remains:

- a controller workload is provisioned and recovered by its direct parent;
- a controller cannot provision, replace, or recover its own substrate;
- one authenticated controller generation is authoritative for a realm;
- ambiguity publishes no route and reports a typed degraded state.

A realm-owned provider executor is permitted only when that realm controller
starts independently on already-provisioned substrate. This permits the `work`
realm's interactive desktop to own Entra, Azure Container Apps, Azure Relay, and
future Azure VM provider execution after the host-local `work` controller and
broker are already running.

The exception does not permit that executor to create, adopt as authority,
replace, repair, stop, or recover:

- the controller that authorizes the provider operation;
- the controller's parent-spawn launch record (or local-root host service),
  identity credential, state root, broker, or delegated resource partition;
- any parent substrate required to start that controller.

Those responsibilities remain with local-root or the direct parent. Registry
construction rejects a dependency cycle before startup.

### Realm transport shortcuts

Strict parent/child policy remains the authorization path. A shared transport
may optimize only an already-authorized data path:

```text
control:
  source -> source controller -> applicable ancestors -> target controller

data after authorization:
  source peer -> shared transport fabric -> target peer
```

The nearest common ancestor issues a short-lived, operation-bound grant that
binds source and target identities, operation/capability, policy epochs,
controller generations, route digest, expiry, and replay nonce. Endpoints bind
the grant digest into their end-to-end ComponentSession. Relay, TLS, managed
identity, and rendezvous evidence establish reachability only.

A grant contains no credential, token cache, raw endpoint, provider resource
ID, command payload, stream data, or user label. Each endpoint returns a signed
close report binding shortcut ID, endpoint role, generation, bounded terminal
reason, byte-count class, and local close time. Byte-count class is untrusted
diagnostic data and drives no policy, billing, or compliance decision. Missing
reports close as `endpoint-unconfirmed` at expiry or revocation rather than
being recorded as success.

A shortcut creates no realm edge, generic TCP tunnel, VPN, port forward, or
provider-native authorization. Revocation or expiry closes the named stream.
An implementation that advertises `active-shortcut-revoke-v2` must close its
transport binding on revocation. Without active revocation, a shared-fabric
grant has a hard maximum lifetime of 60 seconds and requires a new
policy-authorized grant to continue.
Failure uses an already-authorized parent path or returns a typed transport
error; it never probes SSH, a direct network, ambient credentials, or an
unconfigured provider path.

## Provider framework

### Eleven primary provider types

Each configured provider instance has exactly one closed primary type:

```rust
pub enum ProviderType {
    Runtime,
    Infrastructure,
    Transport,
    Substrate,
    Credential,
    Display,
    Network,
    Storage,
    Device,
    Audio,
    Observability,
}
```

The authority boundaries are:

| Type | Exclusive authority |
| --- | --- |
| `Runtime` | Plan, ensure, start, stop, execute within, inspect, adopt, and destroy a workload execution instance. It does not create, delete, or power the infrastructure beneath that instance. |
| `Infrastructure` | Plan, create/apply, power, inspect, adopt, bootstrap-bind, and destroy external infrastructure that may host workloads or controllers. It does not deploy or execute the workload inside an already-bound resource. |
| `Transport` | Connect, listen, rendezvous, inspect, and revoke bounded carriage sessions. It does not authenticate a d2b principal or authorize a semantic operation. |
| `Substrate` | Check and prepare an authorized full-host OS substrate. It cannot widen the owning realm or bypass the substrate repair owner. |
| `Credential` | Interact with a credential source and issue only co-located opaque leases. Co-located in-process credential and consumer modules may exchange secret material only through the private non-serializable interface below; secret material never crosses a process, session, persistence, or telemetry boundary. |
| `Display` | Open, inspect, adopt, and close an already-authorized display session. It does not infer authorization from focus, app ID, title, or compositor identity. |
| `Network` | Plan, ensure, inspect, adopt, and release realm-scoped network resources and policy. Global claims require local-root allocator leases. |
| `Storage` | Plan, ensure, inspect, adopt, snapshot when advertised, and destroy generated storage resources. It cannot treat a diagnostic ledger as repair authority. |
| `Device` | Plan, attach, inspect, adopt, and detach typed mediated devices. It cannot grant a broader device or operation than the request. |
| `Audio` | Open, route, mutate bounded state, inspect, adopt, and close an authorized audio session. It does not expose ambient host audio endpoints. |
| `Observability` | Report bounded health/status, query or stream allowed observations, and perform policy-authorized export. It never becomes audit ownership, repair authority, or a fallback path to raw state. |

Workload roles, runtime locality, execution posture, and optional capabilities
are not provider types. `realmController`, provider executor, transport
connector, persistent shell, durable exec, console, and guest-control endpoint
are roles or capabilities bound to one primary provider.

For the `azure-vm` implementation pair, the authority split is exact even while
the initial implementations remain unadvertised scaffolds. Infrastructure
`azure-vm` exclusively owns Azure VM resource create, power-state change, adopt,
bootstrap binding, inspect, and delete. Runtime `azure-vm` accepts only an
already-bound opaque infrastructure handle and owns workload deployment, exec,
workload start/stop, and workload inspection inside that VM. Runtime
`azure-vm` cannot create, delete, adopt as infrastructure authority, or power
the VM; its `destroy` removes only its workload deployment.

### Contract and trait ownership

`d2b-contracts` is the only owner of serialized provider DTOs:

- `ProviderDescriptor`, `ProviderType`, `ProviderHealth`, and capability claims;
- `ProviderOperationContext`;
- typed requests, plans, opaque handles, observations, and results;
- stable error kinds, retry classes, and redacted error envelopes;
- registry generation and configuration fingerprint;
- JSON and protobuf schemas.

`ProviderDescriptor` contains exactly the globally unique `ProviderId`, one
primary type, canonical implementation ID, provider API version, positive capability
claims, configuration-schema fingerprint, configured scope digest, and registry
generation. It contains no credential, token subject, endpoint, cloud resource
ID, command argument, host path, provider response, or user-provided label.

Provider implementation crates must not copy these types. `d2b-provider` owns
only object-safe in-process traits, typed registries, runtime-only contexts, RPC
proxies, and runtime error wrappers. It has no cloud SDK, broker, daemon,
concrete transport, implementation, or test-mock dependency.

Every primary provider implements:

```rust
#[async_trait]
pub trait Provider: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;

    async fn health(
        &self,
        ctx: &ProviderCallContext,
    ) -> ProviderResult<ProviderHealth>;
}
```

The traits remain object-safe for `Arc<dyn ...>`. They have no generic methods,
`Self` return values, or implementation-specific associated types at the
registry boundary.

`ProviderHealth` is a typed, bounded observation rather than a free-form status
string. Its state is exactly `healthy`, `degraded`, `unavailable`, or `failed`;
it carries the observed registry generation, observation time, one closed reason,
and one closed remediation. Initial reasons are `none`, `provider-degraded`,
`health-timeout`, `health-stale`, `session-disconnected`, `queue-pressure`,
`handshake-timeout`, `authentication-failed`, `identity-mismatch`,
`configuration-mismatch`, `generation-mismatch`, and `capability-mismatch`.
Initial remediations are `none`, `retry-bounded`, `inspect-provider`,
`restart-agent`, `re-enroll-peer`, `repair-configuration`,
`replace-generation`, and `operator-interaction`. The DTO carries no provider
response, endpoint, path, credential subject, user label, or unbounded
diagnostic. The exact transition thresholds are defined in
[Operational health objectives](#operational-health-objectives).

### Exact specialized method families

Generated provider-agent RPC methods use the same semantic names and canonical
request/result DTOs as the in-process traits:

| Trait | Required methods |
| --- | --- |
| `RuntimeProvider` | `capabilities`, `plan`, `ensure`, `start`, `stop`, `inspect`, `adopt`, `destroy` |
| `InfrastructureProvider` | `capabilities`, `plan`, `apply`, `set_power_state`, `inspect`, `adopt`, `bootstrap_binding`, `destroy` |
| `TransportProvider` | `capabilities`, `connect`, `listen`, `issue_binding`, `revoke_binding`, `inspect` |
| `SubstrateProvider` | `capabilities`, `check`, `plan_remediation`, `apply` |
| `CredentialProvider` | `status`, `acquire_lease`, `refresh_lease`, `revoke_lease` |
| `DisplayProvider` | `capabilities`, `open`, `inspect`, `adopt`, `close` |
| `NetworkProvider` | `capabilities`, `plan`, `ensure`, `inspect`, `adopt`, `release` |
| `StorageProvider` | `capabilities`, `plan`, `ensure`, `inspect`, `adopt`, `snapshot`, `destroy` |
| `DeviceProvider` | `capabilities`, `plan_attach`, `attach`, `inspect`, `adopt`, `detach` |
| `AudioProvider` | `capabilities`, `open`, `set_state`, `inspect`, `adopt`, `close` |
| `ObservabilityProvider` | `capabilities`, `status`, `query`, `subscribe`, `export` |

An implementation may return a typed capability denial for an optional method
such as `snapshot`, `listen`, `issue_binding`, or `subscribe`, but it may not
advertise that capability. Required lifecycle methods may return a typed
not-applicable result only where the contract explicitly defines a
non-owning/no-op state; they never silently invoke another provider.

Optional interfaces such as durable execution, persistent shell, console,
file transfer, guest endpoint resolution, or transport active revocation use
parallel capability registries keyed by the same `ProviderId`. Trait-object
downcasting is forbidden.

### Runtime and controller-host posture

Every runtime descriptor uses closed fields for:

- process and restart-adoption authority;
- cgroup ownership;
- network namespace ownership;
- user namespace construction;
- persistent identity protection;
- device mediation.

The network posture is exactly one of:

| Posture | Meaning |
| --- | --- |
| `host-shared` | Shares the host network namespace, is reported as non-isolated, and cannot satisfy realm network isolation. |
| `none` | Has no network interfaces beyond loopback. |
| `isolated-namespace` | Uses a dedicated namespace and only allocator/broker-delegated attachment under realm firewall policy. |

The user-namespace posture is exactly one of:

| Posture | Meaning |
| --- | --- |
| `broker-preestablished` | A typed broker operation creates mappings and privileged mount setup before provider code executes. |
| `unprivileged-self-managed` | Permitted only when the host allows it, no privileged ID/capability is mapped, and conformance proves no broker surface is bypassed. |
| `none` | Creates no user namespace. |

`systemd-user` processes and shells are owned by the exact user's systemd
manager. They live in verified transient user scopes, not in the realm's
`d2b.slice` subtree. Adoption verifies uid, user-manager identity,
`InvocationID`, exact scope control group, executable/configuration
fingerprint, and provider generation. Teardown targets only that verified
scope. It never sweeps arbitrary same-uid processes.

An implementation may advertise `realm-controller-host-v2` only when it
provides persistent identity storage with an accepted non-copyable hardware or
cloud-attested posture. A VMM workload using file-backed swtpm is not eligible:
host root or a whole-state copy can clone its disk and TPM state. A future
hardware-backed vTPM, confidential-computing, or cloud-attestation design must
be separately accepted before it can advertise this capability.
`systemd-user`, Bubblewrap, Minijail, file-backed swtpm, and a name alone do not
qualify. `unsafe-local` is the closed no-isolation posture and warning, never a
provider implementation ID or a controller-host capability.

### Operation context and method semantics

The serializable `ProviderOperationContext` contains only bounded, canonical
fields:

- operation ID and idempotency key;
- already-authorized realm, workload or controller, and opaque principal
  reference;
- selected provider ID, provider type, provider generation, capability, and
  method;
- policy epoch and authorization decision digest;
- authenticated issue time and absolute wall-clock expiry;
- bounded correlation and W3C-compatible trace identifiers.

The non-serializable `ProviderCallContext` adds:

- a local monotonic deadline derived from the authenticated request;
- cancellation;
- the authenticated ComponentSession peer and endpoint role;
- local tracing state.

Tokio channels, `Instant`, file descriptors, paths, sockets, credentials, and
provider SDK objects never enter `d2b-contracts`.

A provider:

1. cannot choose or widen the realm, workload, controller, principal,
   capability, method, or provider instance;
2. cannot authorize the caller;
3. cannot call a privileged broker directly;
4. cannot return secret bytes, raw provider responses, or unbounded endpoint
   data;
5. makes every mutation idempotent by operation ID;
6. returns bounded observed state and one closed retry class;
7. verifies provider ID, configuration fingerprint, resource identity,
   operation binding, and generation during adoption;
8. fails closed on cancellation or deadline expiry;
9. advertises only methods present in the matching typed registry.

The closed retry classes are:

```text
never
same-operation
after-observation
after-interaction
```

`same-operation` requires the identical operation ID and request digest.
`after-observation` requires a fresh `inspect` before retry.
`after-interaction` requires a new explicit operator interaction and never
opens UI merely because status changed. A caller does not invent a new
operation ID to bypass an ambiguous mutation.

Host mutations returned by a provider plan are re-originated by the owning
realm daemon as typed broker operations. An agent never receives the broker
wire protocol. A method-specific FD may reach an agent only as a declared,
validated ComponentSession attachment after broker and daemon authorization.

### Registry construction, duplicates, and shutdown

The complete host bundle validates provider identity globally, then each realm
constructs eleven typed registries as one transaction:

- every configured `ProviderId` is globally unique across the complete accepted
  constellation configuration, every realm, and every primary registry;
- each instance appears in exactly one primary registry;
- descriptor type, implementation ID, API version, configuration fingerprint,
  and configured scope must match the bundle;
- an implementation factory is unique only by the pair
  (`ProviderType`, `ImplementationId`); registering two factories for that pair
  is fatal;
- any number of configured instances may use the same factory pair when each
  has a distinct globally unique `ProviderId`, scope, configuration
  fingerprint, and generation;
- every advertised optional capability has exactly one capability-registry
  implementation;
- every capability-registry implementation is advertised;
- a duplicate provider ID, factory pair, per-instance capability claim, or
  generation aborts the entire host registry generation before any provider
  receives a call;
- there is no "last registration wins" behavior.

Every generated factory-registration, capability-registration,
configured-instance construction, and generation-finalization API is fallible
and returns `Result<_, RegistryBuildError>`. Duplicate keys, malformed
configuration, descriptor mismatch, missing capabilities, and factory
construction failures return typed, redacted errors and abort the transaction;
they never use `panic!`, `assert!`, `unwrap`, or `expect` for configuration
handling. The factory key is exactly
(`ProviderType`, `ImplementationId`). Registration creates one factory for that
key; constructing multiple configured instances from it is a separate fallible
operation and is required to work for distinct globally unique `ProviderId`
values.

A registry generation is immutable. Reconfiguration builds and validates a new
generation, stops admission to the old generation, drains bounded in-flight
operations, and atomically publishes the new generation. Calls retain the
generation they entered. Mutations that cannot be proven complete are observed
or quarantined; they are not replayed under a new generation by guesswork.

Shutdown is registry-owned:

1. reject new calls;
2. signal cancellation;
3. wait the configured bounded drain deadline;
4. revoke registry-owned transport bindings and credential lease handles;
5. close provider-owned sessions;
6. stop an agent only through its declared restart/ownership contract;
7. record typed unresolved observations for anything that cannot be proven
   closed.

Agent disconnect marks that provider generation unavailable. It never selects
another implementation, ambient SDK credential, shell command, SSH path, or
direct broker call.

### In-process and provider-agent implementations

The registry is hybrid:

- audited, trusted, first-party host implementations may run in-process in the
  owning realm daemon;
- same-user, workload-resident, cloud-credential, and third-party
  implementations run as provider agents over `d2b.provider.v2`;
- no dynamic library loading or untrusted in-process plug-in mechanism exists;
- an RPC proxy implements the same Rust trait as an in-process provider;
- conformance is identical on both sides of the proxy.

Initial placement is:

| Type | Implementation ID | Placement |
| --- | --- | --- |
| Runtime | `cloud-hypervisor` | Trusted in-process adapter in the owning realm daemon; all host effects use its confined broker. |
| Runtime | `qemu-media` | Trusted in-process adapter in the owning realm daemon; QMP remains an external protocol. |
| Runtime | `systemd-user` | Per-user provider agent under the exact user's systemd manager. |
| Runtime | `azure-container-apps` | Provider agent in the configured credential-owning workload, co-located with its credential module. |
| Runtime | `azure-vm` | Compile-tested fake-SDK agent/test scaffold only; never in a production registry. |
| Infrastructure | `azure-vm` | Compile-tested fake-SDK agent/test scaffold only; never in a production registry. |
| Transport | `unix-stream` | Trusted in-process adapter in the connecting/listening realm component. |
| Transport | `unix-seqpacket` | Trusted in-process adapter in the connecting/listening realm component, using `d2b-session-unix`. |
| Transport | `native-vsock` | Trusted in-process adapter in the owning realm daemon or guest component. |
| Transport | `cloud-hypervisor-vsock` | Trusted in-process adapter in the owning realm daemon; the CH `CONNECT` prelude remains external. |
| Transport | `azure-relay` | Provider agent in the configured credential-owning workload, co-located with its credential module. |
| Transport | `loopback` | In-process testkit implementation only. |
| Transport | `direct-tls` | Unadvertised compile/conformance scaffold in test code only. |
| Transport | `quic` | Unadvertised compile/conformance scaffold in test code only. |
| Substrate | `nixos` | Trusted in-process adapter in local-root d2bd; authorized host mutation remains a local-root broker operation. |
| Substrate | `linux` | Trusted in-process adapter in local-root d2bd; authorized host mutation remains a local-root broker operation. |
| Credential | `secret-service` | `d2b-userd` credential agent using `oo7` after keyring unlock. |
| Credential | `entra` | Private module in the configured credential-owning provider agent/workload. |
| Credential | `managed-identity` | Private module in the configured credential-owning provider agent/workload. |
| Display | `wayland` | Trusted in-process adapter in the owning realm daemon; the separately sandboxed proxy speaks external Wayland. |
| Network | `local-realm` | Trusted in-process adapter in the owning realm daemon; namespace-local effects use the child broker and global effects use local-root. |
| Storage | `local` | Trusted in-process adapter in the owning realm daemon; mutation uses generated broker storage IDs. |
| Device | `host-mediated` | Trusted in-process adapter in the owning realm daemon; only allocator/broker-issued device FDs cross the boundary. |
| Audio | `pipewire-vhost-user` | Trusted in-process adapter in the owning realm daemon; PipeWire and vhost-user remain external protocols. |
| Observability | `local` | Trusted in-process bounded adapter in the owning realm daemon. |

No initial implementation is implicitly placed. Adding an implementation ID
requires adding its exact placement, executable owner, broker relationship, and
credential posture to the generated provider contract before registration.

### Credential-opaque lease rule

`CredentialProvider` has no `get_secret`, byte-stream, file, environment, or FD
return path. `CredentialLease` contains only:

- an opaque random lease ID meaningful inside the owning agent;
- provider and consumer adapter IDs;
- allowed SDK operation classes;
- expiry, generation, source version, and rotation metadata;
- revocation state.

The credential provider and the SDK-consuming provider adapter must be
co-located in the same process and agent/workload. Their Rust modules may live
in separate crates and may exchange secret material through one sealed private
interface whose values are non-serializable, non-`Debug`, non-`Clone`, and
zeroized on release. This narrow in-process crate boundary is permitted; a
claim that secret bytes can never cross any crate boundary would be
unenforceable. The interface cannot be implemented by an RPC proxy, persisted,
attached as an FD, or exposed to an unrelated module.

A controller may coordinate the opaque lease handle but cannot redeem it,
serialize its secret, or forward it to another agent. Secret material never
crosses a process, ComponentSession, persistence, crash-report, log, trace,
metric, or audit boundary.

An Entra token, Relay credential, managed identity token, Secret Service value,
or cloud SDK credential therefore remains in its credential-owning boundary.
`d2bd`, all brokers, and unrelated providers never read those bytes.

The credential posture by primary provider type is:

| Provider type | Credential posture |
| --- | --- |
| Runtime | Local runtimes hold no provider credential. A cloud runtime may consume only a co-located opaque lease inside its provider agent. |
| Infrastructure | A cloud infrastructure adapter may consume only a co-located opaque lease inside its provider agent. Plans, handles, and observations remain non-secret. |
| Transport | Unix and vsock transports hold none. A Relay/direct transport connector may consume only its own co-located lease; transport authentication is not d2b authorization. |
| Substrate | Holds no user or cloud credential. Privileged apply is re-originated through the owning daemon and broker. |
| Credential | Owns interaction with the source and the local lease table. Only its co-located private consumer interface may expose material to the exact SDK adapter authorized by a lease. |
| Display | Holds no user or provider credential. Compositor/display handles are typed capabilities, not credentials. |
| Network | Local network providers hold none. A future cloud network adapter follows the same co-located lease rule as infrastructure. |
| Storage | Local storage providers hold none. A future cloud storage adapter follows the same co-located lease rule and returns opaque resource handles. |
| Device | Holds device/session authority only. CTAPHID traffic, touch, or a device FD never authorizes extraction of authenticator credentials. |
| Audio | Holds no user or provider credential. PipeWire handles remain scoped session capabilities. |
| Observability | Local observation holds none. An external exporter may consume only an explicitly scoped co-located lease and cannot read unrelated audit/state. |

### Type-first provider names

Implementation crates use:

```text
d2b-provider-<provider-type>-<implementation>
```

Examples include:

```text
d2b-provider-runtime-cloud-hypervisor
d2b-provider-runtime-qemu-media
d2b-provider-runtime-systemd-user
d2b-provider-runtime-azure-container-apps
d2b-provider-infrastructure-azure-vm
d2b-provider-transport-unix-stream
d2b-provider-transport-unix-seqpacket
d2b-provider-transport-cloud-hypervisor-vsock
d2b-provider-transport-azure-relay
d2b-provider-substrate-nixos
d2b-provider-credential-entra
d2b-provider-display-wayland
d2b-provider-network-local-realm
d2b-provider-storage-local
d2b-provider-device-host-mediated
d2b-provider-audio-pipewire-vhost-user
d2b-provider-observability-local
```

Axis-free names such as `d2b-provider-aca`, `d2b-provider-relay`,
`d2b-provider-azure`, and `d2b-provider-host` are deleted.

### Required provider parity

The following IDs and production postures are frozen for the initial v2
cutover:

| Type | Implementation IDs | Required v2 parity and posture |
| --- | --- | --- |
| Runtime | `cloud-hypervisor`, `qemu-media`, `systemd-user`, `azure-container-apps`; scaffold `azure-vm` | Wrap the real VM DAG, graceful stop, pidfd/cgroup adoption, qemu-media QMP path, same-user scopes and persistent shells, and live ACA lifecycle. Runtime `azure-vm` is an unadvertised workload-deploy/exec/inspect scaffold over an already-bound infrastructure handle and has no VM create/delete/power authority. |
| Infrastructure | scaffold `azure-vm` | Typed VM create/power/adopt/bootstrap/delete requests, plans, observations, fake SDK, and conformance only. Infrastructure exclusively owns the Azure VM resource, but has no workload deployment/exec authority. No production registration or provisioning claim. |
| Transport | `unix-stream`, `unix-seqpacket`, `native-vsock`, `cloud-hypervisor-vsock`, `azure-relay` | Preserve local stream/packet/FD, native and CH-vsock, and live Relay behavior. `loopback` exists only in testkit. `direct-tls` and `quic` remain unadvertised scaffolds unless required by existing live behavior. |
| Substrate | `nixos`, `linux` | Real checks, plans, and authorized apply for the currently supported full-host substrate paths. |
| Credential | `secret-service`, `entra`, `managed-identity` | Secret Service through `d2b-userd`; Entra and managed identity only inside the owning workload/agent; opaque leases only. |
| Display | `wayland` | Real Wayland, Waypipe/cross-domain, proxy, authorization, readiness, and lifecycle behavior; no synthetic facade. |
| Network | `local-realm` | Existing bridge, TAP, net VM, NAT, DHCP, nftables, netlink, external attachment, and realm-isolation behavior behind realm-scoped contracts. |
| Storage | `local` | Existing local state, disk image, Nix store-view, closure sync, media, and persistent-state operations behind generated storage contracts. |
| Device | `host-mediated` | Existing TPM, USBIP, CTAPHID/UHID security key, GPU, video, and mediated-device behavior with typed capabilities. |
| Audio | `pipewire-vhost-user` | Existing PipeWire and vhost-user-sound lifecycle, routing, status, mute, volume, and policy. |
| Observability | `local` | Existing bounded metrics, tracing, audit export, status, and projections without taking audit ownership or reading raw repair state. |

The working CTAPHID/UHID implementation is code canon. It is refactored behind
the device and ComponentSession contracts, not replaced with a new FIDO stack.
Each ceremony requires trusted, source-specific intent. A small bounded CBOR
parser may classify command and RP. Reset, credential deletion/management,
biometric enrollment, authenticator configuration, vendor, and unknown
destructive commands remain denied by a closed policy.

The physical authenticator is shared only at the bounded ceremony layer.
Controller/provider-executor, browser, and developer workloads acquire and keep
their own independent Entra sessions and token caches. The host serializes
ceremonies and releases the lease on completion, denial, disconnect, or
timeout. PIV, CCID, OTP, and OpenPGP continue to require exclusive USB
ownership; they do not run concurrently with CTAPHID proxying through an
implicit fallback.

Every production lifecycle and component call graph must enter the matching
typed registry. A provider-shaped facade that production bypasses does not
satisfy this ADR.

### Azure VM is scaffold only

`azure-vm` runtime and infrastructure contracts include typed configuration,
requests, plans, observed state, fake SDK clients, and conformance fixtures.
The infrastructure contract exclusively models VM resource create, power,
adopt, bootstrap, inspect, and delete. The runtime contract requires an opaque
bound infrastructure handle and models only workload deploy, exec,
workload-local start/stop, and inspect. Tests must prove that neither contract
can deserialize or dispatch the other contract's authority.

They do not:

- register in a production provider registry;
- advertise any live Azure capability or `realm-controller-host-v2`;
- make CLI or status claim that Azure VM support is available;
- execute a live Azure SDK call.

Production Azure VM provisioning and remote-controller execution are post-v2
work requiring a later accepted decision. Capability checks must return a typed
unavailable result before any credential acquisition or network call.

## Universal ComponentSession

### Scope

Every live IPC protocol owned by d2b uses `ComponentSession`, whether it runs
over:

- Unix stream;
- Unix seqpacket;
- an inherited Unix socketpair;
- native AF_VSOCK;
- the Cloud Hypervisor vsock adapter;
- a provider transport such as Azure Relay;
- a future explicitly configured direct transport.

There is no specialized broker, helper, public-daemon, picker, clipboard,
activation, TTY, or FD-bearing protocol exception. Durable JSON records,
generated bundle files, audit JSONL, and presentation projections are state,
not live IPC, and do not become sessions.

External protocols remain external:

- Wayland;
- PipeWire;
- vhost-user;
- QMP;
- Cloud Hypervisor HTTP and its vsock `CONNECT` prelude;
- CTAPHID;
- D-Bus and Secret Service;
- Niri IPC;
- Azure and other cloud APIs;
- Relay WebSocket rendezvous;
- shpool internals;
- ttrpc framing itself.

Ttrpc frames are carried as authenticated ComponentSession control payloads;
d2b does not redefine ttrpc. A d2b-owned adapter that controls any external
protocol still uses ComponentSession on its d2b side.

### Selected dependencies and layers

The fixed layering is:

```text
TransportProvider or direct local endpoint
  -> TransportSession
       reliable byte stream, or
       Unix packet transport with attachment capability
  -> ComponentSession v2
       fixed preface
       endpoint purpose and role validation
       Noise handshake and channel binding
       bounded encrypted records
       replay and sequence protection
       keepalive, close, and reconnect generation
       packet-atomic attachment descriptors when negotiated
       reserved ttrpc control channel
       bounded named-stream channels
  -> generated ttrpc/protobuf .v2 service
```

Dependencies are:

- Noise: `snow`;
- control RPC: the exact runtime pins `ttrpc = "=0.9.0"` from
  `ttrpc-rust` and `protobuf = "=3.7.2"` from `rust-protobuf`, with
  `ttrpc-codegen = "=0.6.0"` and `protobuf-codegen = "=3.7.2"` for the
  generated bindings;
- data: the d2b named-stream mux with independent bounded credit;
- Unix packet and FD substrate: established `rustix` or `nix` calls, wrapped by
  Tokio `AsyncFd` in one small audited `d2b-session-unix` abstraction.

There is no hand-authored protobuf codec, alternate realm record layer, local
plaintext mechanism, or second mux.

W2 begins with a dependency/API-fit spike against those exact versions. The
spike generates a minimal asynchronous server and client, carries unary calls
through the ComponentSession transport adapter, and proves on both Tokio
current-thread and multi-thread runtimes that client, accept, read, write, and
handler waits yield to an independent progress task. It also compile-proves the
generated service traits, cancellation path, and owned/borrowed handler
lifetimes used below. This gate runs before any `.v2` service schema is frozen.
An incompatible API, blocking generated path, or required sync adapter fails W2
and requires a reviewed dependency decision; it is not worked around after
schemas are generated.

### Fixed preface and service selection

Every connection starts with this 16-byte network-order preface:

```text
offset  size  value
0       8     44 32 42 43 53 32 0d 0a (ASCII "D2BCS2" then CR LF)
8       2     component-session major, exactly 2
10      2     component-session minor, exactly 0
12      4     canonical handshake-offer byte length
```

The offer is a bounded canonical binary contract, not JSON and not the codec
being selected. Its maximum is 16 KiB. It carries exactly one endpoint-policy
allowed purpose, initiator role, responder role, service package, schema
fingerprint, Noise profile, limit profile, and attachment policy. An endpoint
may support several services on separate purposes, but one connection does not
negotiate down a preference list. Any mismatch fails before semantic dispatch.

There is no semver range, v1/v2 choice, ignored feature flag, codec fallback,
authentication fallback, or lower limit selected after failure.

The service package names are:

```text
d2b.daemon.v2
d2b.realm.v2
d2b.guest.v2
d2b.provider.v2
d2b.broker.v2
d2b.user.v2
d2b.runtime.systemd-user.v2
d2b.shell.v2
d2b.clipboard.v2
d2b.clipboard.picker.v2
d2b.notify.v2
d2b.security-key.v2
d2b.wayland.v2
d2b.activation.v2
d2b.tty.v2
```

Any additional internal service uses a `.v2` package and requires an updated
closed inventory in this ADR's generated successor contract. No `.v1` service
package is registered in v2.

Every ttrpc request carries a bounded request ID, correlation/trace context,
authenticated wall-clock issue and absolute-expiry fields, and an idempotency
key for a mutating method. Service and method IDs come from generated metadata.
The authenticated principal comes only from session state, and the required
capability comes only from trusted service/method metadata. A request payload
cannot provide either value.

### Deadline and request cancellation

The ComponentSession request envelope carries `issued_at_unix_ms` and
`expires_at_unix_ms` as authenticated fields separate from the ttrpc context.
The fixed global maximum request lifetime is 15 minutes and the maximum
permitted wall-clock skew is 30 seconds; a generated service policy may lower
either value. Admission rejects an expiry before issue time, a declared
lifetime above the cap, an issue time more than the allowed skew in the future,
or a request already expired by the receiver's wall clock. Long-running
provider work returns an operation handle and is observed by later bounded
calls rather than extending one RPC beyond the cap.

Each remote/session endpoint must have clock-health evidence within that
30-second bound; loss of clock health rejects new cross-host calls until
restored. Skew is an acceptance bound, not extra lifetime: it is never added to
the remaining duration.

At the first authenticated ingress, the receiver computes:

```text
wall_remaining = expires_at_unix_ms - local_wall_clock
monotonic_deadline = Instant::now() + min(wall_remaining, service_max_lifetime)
```

It preserves the authenticated absolute expiry unchanged. At every ingress,
forwarding hop, scheduler enqueue/dequeue, provider queue, and final dispatch,
the component recomputes wall-clock remaining time and intersects it with the
remaining local monotonic budget and the service cap. Non-positive remaining
time rejects or cancels the call before semantic dispatch. A backward wall-clock
jump cannot lengthen the monotonic budget; a forward jump expires the call.

Only the resulting capped relative duration is mapped to ttrpc
`timeout_nano`, after clamping to the field's representable range. Epoch time is
never encoded in `timeout_nano`, and a peer-supplied ttrpc timeout can only
shorten, never replace or extend, the authenticated absolute expiry. A
forwarder generates a fresh relative ttrpc timeout from the current remaining
budget at that hop.

Request IDs are unique within one ComponentSession generation. The reserved
session-control channel carries a transcript-bound `CancelRequest` naming the
session generation and request ID. Acceptance creates one cancellation token
shared by queues, ttrpc dispatch, provider proxy, attachment owner, and named
streams. Cancellation, deadline expiry, or session loss removes queued work
before dispatch or signals the running handler, closes and accounts every
unconsumed attachment/stream, revokes partial ephemeral leases, and runs the
method's bounded cleanup. An ambiguous durable mutation is observed or
quarantined under its original operation ID, never replayed as cancellation
cleanup. The endpoint returns one bounded terminal cancellation acknowledgement
when the control channel remains available.

### Generated dispatch ownership

Generated ttrpc handlers dispatch in the session-owned request task. That task
may borrow the registered trait object and `ProviderCallContext`, call an
`async_trait` method through `&self`, and await the returned borrowed future to
completion because the request task owns the borrow lifetime.

No code may pass or spawn a future borrowing `&self`, a registry entry, request
stack data, or a session-local context into `tokio::spawn` or any other
`'static` executor. Work that intentionally outlives the request task is
converted before detachment into either:

- an owned `Arc<dyn Provider...>` plus an owned or `Arc`-backed call context; or
- an explicit owned operation object containing only the bounded authority,
  cancellation, deadline, generation, and resource handles it needs.

The detached task owns every captured value. It does not retain a reference to
the generated handler, registry generation stack frame, ttrpc request, or
session task. Compile tests cover both the borrowed in-task path and every
allowed owned detached path.

### Authentication profiles

The fixed profiles are:

| Session | Noise profile and identity |
| --- | --- |
| Local Unix client/controller/broker/helper/userd/component | Ephemeral `Noise_NN_25519_ChaChaPoly_SHA256`. Identity evidence is directional: an acceptor authenticates the connecting initiator from kernel-observed `SO_PEERCRED`; an initiator authenticates the responder from trusted fixed-socket or allocator-issued listener provenance, inode, launch/unit owner, and endpoint policy. Inherited/socketpair endpoints add first-packet `SCM_CREDENTIALS` and a parent launch binding. Noise supplies confidentiality and transcript integrity but no static peer identity. |
| Enrolled realm/controller/provider/workload peers | `Noise_KK_25519_ChaChaPoly_SHA256` using enrolled static public keys and fresh ephemeral session state. |
| One-time realm or guest bootstrap | `Noise_IKpsk2_25519_ChaChaPoly_SHA256` using the expected parent static key and a single-use, operation-bound 256-bit-or-stronger PSK. |
| Normal enrolled guest control | `Noise_KK_25519_ChaChaPoly_SHA256` using the parent realm key and guest vTPM-backed static key. |
| Explicit external interoperability | Mutual TLS may be transport evidence only when an explicit identity mapper is configured. It is never a fallback and does not replace the ComponentSession Noise profile. |

`snow` is the only production Noise implementation. W2 commits deterministic
vector fixtures for all three profiles and every purpose class. Each fixture
fixes protocol name, prologue bytes, role, static and ephemeral private-key test
inputs, PSK where applicable, handshake payloads, transcript hash, transport
keys, first protected records, and rejection mutations. W3 verifies the same
fixtures through `snow`; generated bindings and implementations may not update
expected vectors implicitly.

The canonical fixture IDs and required coverage are:

| Fixture ID | Profile | Required purposes |
| --- | --- | --- |
| `component-session-v2-local-nn` | `Noise_NN_25519_ChaChaPoly_SHA256` | Every local Unix purpose, including an attachment-enabled seqpacket case |
| `component-session-v2-enrolled-kk` | `Noise_KK_25519_ChaChaPoly_SHA256` | Realm peer, normal guest, and enrolled provider/workload peer |
| `component-session-v2-bootstrap-ikpsk2` | `Noise_IKpsk2_25519_ChaChaPoly_SHA256` | Realm bootstrap and guest bootstrap, including wrong operation, expired PSK, and replay rejection |

The vectors live in one canonical committed artifact owned by
`d2b-contracts::session`; copies in another crate or sibling repository are
forbidden.

An endpoint accepts no `none`, local plaintext, old HMAC, long-lived guest PSK,
or weaker retry. Noise ephemeral keys, cipher state, nonces, sockets, and
session keys are never persisted or adopted across reconnect.

### Local peer credentials

Local identity evidence is explicitly directional. D2b does not claim that both
sides of every Unix connection observe a meaningful service identity through
`SO_PEERCRED`.

For a pathname listener, the accepting responder reads kernel-observed
`SO_PEERCRED` from the accepted socket before the preface and maps the
initiator's pid/uid/gid to one closed endpoint role. The connecting initiator
does not treat the listener-side `SO_PEERCRED` value as responder identity,
because a socket-activated listener may have been created before the service
process. Instead it authenticates the responder endpoint by all of:

1. an expected fixed or broker-generated path from the integrity-checked
   endpoint contract;
2. anchored parent ownership and mode;
3. matching pre-connect and post-connect path device/inode/type observations
   under the non-writable anchored parent;
4. the expected system or user socket unit and activation owner for a fixed
   local-root/user endpoint, or the allocator/broker-issued listener identity
   and launch record for a child-realm or other dynamic endpoint;
5. the endpoint policy's exact purpose, responder role, service, schema, limits,
   and attachment policy.

A local-root or user service receives a socket-activated listener only from the
declared system/user manager unit. A child controller or broker receives its
already-bound listener only from the local-root allocator as part of its typed
parent spawn. Another dynamic owner receives an already-bound listener FD only
from the broker named in the generated row. No receiver unlinks and recreates
the path.

For an inherited or socketpair endpoint, the creating parent enables
`SO_PASSCRED` on both socket ends immediately after creation and verifies both
ends with `getsockopt` before any fork, clone, exec, endpoint handoff, or packet
send. This is a launch precondition, not receiver initialization. Each receiver
verifies that the inherited option is still enabled before its first read and
must not attempt to repair a disabled option after handoff. The peer's first
preface packet must carry exactly one `SCM_CREDENTIALS`, even when it sends at
its first schedulable instruction. The creating parent also mints a single-use
launch binding over purpose, endpoint roles, executable/configuration digest,
expected cgroup, pidfd identity, and expiry. The receiver verifies that the
kernel credential PID is the live process referenced by the pidfd and that its
executable and cgroup still match the launch binding before accepting Noise
bytes. Missing, extra, stale, or inconsistent evidence closes the endpoint.

The normalized directional evidence and its verifier role enter the Noise
prologue with purpose, endpoint identity, service, schema, limits, and
attachment policy. Semantic requests inherit only the principal established by
that endpoint policy. A payload cannot supply or replace uid, gid, pid, realm
role, or broker authority.

An established ComponentSession endpoint is never transferable by
`SCM_RIGHTS`, pidfd duplication, inheritance, or broker handoff. Only an
unaccepted listener or a fresh pre-handshake transport endpoint may be handed
to its declared owner. Once the first preface packet is sent or received, the
socket is bound to that process, session generation, directional evidence, and
Noise transcript until close.

Relay, TLS, vsock CID, and managed identity are likewise not mapped to a local
role.

### Guest bootstrap and static identity

A guest's one-time bootstrap lifecycle is:

1. the parent launches or adopts one exact runtime instance and records its
   opaque provider handle, executable/configuration identity, cgroup/pidfd
   identity where local, and the exact transport endpoint assigned to that
   instance;
2. the parent creates a fresh boot nonce and random operation-bound PSK bound to
   realm, workload ID, controller and workload generation, runtime-instance
   handle digest, transport endpoint digest, purpose, expiry, and replay nonce;
3. local guests receive the boot nonce and PSK through a one-shot read-only boot
   seed attached only to that launched instance; remote guests receive the
   corresponding approved material only through the bound infrastructure
   bootstrap handle;
4. guestd generates its static Noise private key inside the guest and seals it
   to that workload's vTPM and guest persistent state;
5. parent and guest complete `Noise_IKpsk2_25519_ChaChaPoly_SHA256`, binding the
   boot nonce, runtime instance, transport endpoint, and both generations into
   the transcript;
6. the parent stores only the enrolled public key, authoritative generation,
   runtime/transport binding digests, bounded attestation/enrollment result,
   active per-boot nonce digest, and revocation state;
7. both sides erase the PSK and withdraw the seed or rendezvous before normal
   service or route publication;
8. all normal guest sessions use static mutual `KK`, but admission also requires
   the connection to arrive on the currently bound parent-launched runtime and
   transport endpoint and to prove the active nonce that was freshly generated
   for that runtime boot and one authoritative workload/controller generation.

Only one runtime/guest identity generation may be authoritative for a workload.
An ambiguous duplicate, copied state, endpoint mismatch, stale boot nonce, or
old controller/workload generation publishes no route and requires explicit
parent-authorized re-enrollment or recreation.

No long-lived host/guest token file remains, and the host does not store the
guest private key. File-backed swtpm is nevertheless cloneable by host root or
by anyone able to copy the whole guest disk plus TPM state. Sealing a guest key
to that vTPM protects against ordinary guest-disk leakage without the TPM state;
it does not protect against malicious host root or a whole-state clone. The
parent runtime, endpoint, generation, and fresh-boot binding above makes the
sealed key alone insufficient and rejects a copy outside the currently selected
runtime under the ordinary non-malicious-host model. It does not prevent
malicious host root from cloning or substituting the complete runtime evidence
and is not non-copyable hardware attestation.

Consequently `realm-controller-host-v2` is denied for file-backed swtpm. A
future non-copyable hardware-backed vTPM or accepted cloud/confidential
attestation design requires a separate decision and conformance contract before
controller-host registration. Loss or replacement of a guest vTPM/private key
requires explicit re-enrollment with a new generation; there is no automatic
key repair.

### Record and queue rules

Noise handshake messages and protected records use a 16-bit big-endian wire
length. Protected ciphertext is at most 65,535 bytes and plaintext is at most
65,519 bytes after the 16-byte authentication tag. Larger logical frames are
fragmented only by ComponentSession.

Every admission, fragmentation, queue-credit, and pre-allocation calculation
uses checked arithmetic and includes all applicable wire overhead: the fixed
preface and canonical offer, the selected Noise pattern's handshake fields and
handshake AEAD tags as reported by `snow`, the two-byte record length, the
16-byte transport AEAD tag, ComponentSession record/fragment headers, and the
ttrpc/protobuf envelope. A buffer is reserved from the checked ciphertext bound,
never from a peer length plus unchecked overhead. Underflow, overflow, an
impossible `snow` output bound, or a value above the selected profile limit is a
typed pre-allocation rejection; it never reaches indexing, allocation, or a
panic.

Initial hard ceilings are:

| Resource | Hard ceiling |
| --- | ---: |
| Canonical handshake offer | 16 KiB |
| Logical ttrpc request or response | 1 MiB |
| Logical named-stream message | 1 MiB |
| Active named streams per session | 128 |
| Unix attachments in one packet | 32 |
| Accepted attachments for one request | 64 |
| Outstanding attachments for one operation | 128 |
| Outstanding attachments for one session | 256 |
| Process-global transferable attachment credits | 2,048, further bounded by `RLIMIT_NOFILE` |
| Host-wide outstanding transferable attachments | 8,192 |
| Reserved non-attachment control FD headroom | 64 |
| Queued plaintext per named stream, each direction | 256 KiB |
| Aggregate queued named-stream plaintext, each direction | 4 MiB |
| Reserved ttrpc-control queue, each direction | 2 MiB |
| Reserved session/attachment-control queue, each direction | 64 KiB |

Service profiles may set lower values, never higher ones. Declared lengths,
stream counts, and attachment counts are checked before allocation. Fragment
sequence, count, total plaintext length, channel, and reconnect generation are
authenticated. Truncation, duplication, reordering, overlap, nonce exhaustion,
or over-limit reassembly closes the session.

Unix transports are nonblocking, and their Tokio readiness contract is
explicit. After obtaining an `AsyncFd` read-ready guard, the sole read driver
repeats `recvmsg` through that guard until `EAGAIN` or `EWOULDBLOCK`; successful
packets do not clear readiness. The sole write driver likewise repeats
`sendmsg` while queued output remains until the queue is empty or the syscall
returns `EAGAIN` or `EWOULDBLOCK`. `try_io` clears readiness only for the
would-block result. If a bounded fairness budget stops either loop before
would-block while work remains, the driver retains the guard for immediate
continuation or leaves cached readiness uncleared and explicitly reschedules
itself. It never consumes one packet, drops readiness, and waits for a kernel
wakeup that may not recur. A seqpacket send succeeds only as one complete
packet; a stream short write retains the unsent suffix.

Exactly one driver reads and one driver writes the protected transport. Write
priority is:

1. fatal close, revocation, and session control;
2. ttrpc control and cancellation;
3. attachment acknowledgement;
4. named-stream data with bounded round-robin fairness.

No lock is held across `await`. A stalled data stream cannot consume reserved
control credit. Backpressure is typed and bounded. Reconnect always performs a
new handshake and increments the session generation. Calls retry only under the
provider/operation idempotency contract; streams resume only through an
explicit generation and cursor contract.

Every process has one global FD-credit allocator shared by accepts, sessions,
requests, operations, and provider dispatch. At startup and after any
`RLIMIT_NOFILE` reduction, its transferable pool is:

```text
min(2048, rlimit_nofile_soft - observed_nontransferable_open_fds - 64)
```

The final term reserves 64 descriptor slots exclusively for listeners,
session-control, cancellation, error reporting, and deterministic cleanup; it
is never lent to an attachment. Startup fails closed if the observed baseline
and reserve do not fit below the soft limit. The local-root allocator also
enforces the 8,192 host-wide aggregate ceiling across realm processes; a
generated host policy may lower it. That ceiling may lower process grants but
cannot raise any process above its own computed pool.

The sender reserves packet, request, operation, session, process-global, and
host-global credit before transfer. The receiver independently accounts the
actual ancillary FDs before semantic dispatch. An over-credit packet is drained
with bounded `recvmsg`, every received FD is closed, the request is rejected,
and no partial subset is delivered. Credits are owned by RAII objects and are
released exactly once on acknowledgement/ownership transfer, rejection,
cancellation, deadline, disconnect, handler failure, or process teardown.
Closing order is attachment children, request, operation, session; rejection
and cleanup are deterministic under every race.

### Unix seqpacket attachments

A Unix packet transport carries one encrypted ComponentSession record and its
ancillary data in one `sendmsg`. Before `recvmsg`, the receiver sizes its
payload buffer to the negotiated protected-packet ceiling. It computes
ancillary capacity from the negotiated per-packet attachment maximum, which
cannot exceed the hard maximum of 32: the worst case reserves one checked
`CMSG_SPACE(sizeof(RawFd))` slot per attachment plus a checked
`CMSG_SPACE(sizeof(ucred))` slot when credentials are allowed. Overflow or an
unrepresentable capacity fails before the syscall. The authenticated record
declares:

- attachment count;
- ordered index and closed attachment kind;
- service, method, request, operation, packet sequence, and purpose binding;
- required kernel object type and access policy;
- whether duplicate kernel objects are permitted.

The receiver uses `MSG_CMSG_CLOEXEC`. Immediately after every successful
`recvmsg`, its raw ancillary collector walks the returned control area before
examining payload validity or fatal message flags. It takes exactly-once RAII
ownership of every complete descriptor value exposed by every
`SOL_SOCKET`/`SCM_RIGHTS` header, including a partial list, extra rights header,
or rights attached to an unknown record or attachment kind. Every descriptor
actually installed and left open in the receiving process is represented by
one of those complete returned values. On Linux, descriptors omitted because
the supplied control area was exhausted are closed by the kernel; every
descriptor exposed in the returned control area is owned and closed by this
collector and is never left to that kernel rule.

Only after collection does the receiver reject `MSG_TRUNC`, `MSG_CTRUNC`,
malformed or partial control data, missing or extra control messages, missing
or extra FDs, unknown kinds, duplicate objects unless explicitly allowed,
non-`CLOEXEC` descriptors, wrong access mode, wrong socket family/type, wrong
filesystem/object class, and any operation-policy mismatch. It validates
pidfds, sockets, pipes, memfds, device FDs, TAP/KVM/vhost/fuse/hidraw handles,
and other kinds through exact method-specific policy. The collector immediately
charges the actual count to packet, session, process-global, and host-global
receive credit. Once the authenticated envelope identifies a request and
operation, their aggregate credits must also be acquired before semantic
dispatch. Failure at either stage drops the one ownership vector and releases
its credit exactly once; success moves each accepted descriptor once into its
typed attachment owner. No error path also closes a moved or already-dropped
numeric FD.

On any failure cleanup completes before the receiver returns the bounded fatal
error and closes the session. The sender retains its authority until a
transcript-bound attachment acknowledgement or explicit transfer rule says
otherwise. There is no split "JSON packet then later FD" path and no
post-receive `fcntl` race for `CLOEXEC`.

### Session observability

ComponentSession emits bounded metrics for active sessions, handshake result,
connect/reconnect attempts, close reason, control-credit exhaustion, aggregate
queue depth/capacity, scheduling delay, and rejected records. Metric dimensions
are limited to transport kind, purpose, channel class, Noise mechanism class,
and closed result/reason class. `channel class` is only
`session-control`, `ttrpc-control`, `attachment-control`, or `named-stream`;
there is no metric series per session, request, stream, realm, workload, or
provider instance.

Reconnect attempts increment counters, while repeated identical logs/audit
events are suppressed and summarized by bounded backoff/result class. Traces
and authorized audit may carry bounded operation, correlation, provider, and
generation IDs. No telemetry surface carries raw endpoint, resource ID, user
identity, key fingerprint, proof, credential, command, path, or payload.

### Operational health objectives

The following are initial v2 operational defaults and hard failure
classifications. They are implementation health objectives, not latency or
availability promises to an external customer. A generated service profile may
tighten them but may not loosen a hard deadline or failure threshold without a
versioned contract change. Percentiles use one-minute buckets over a rolling
five-minute window and exclude policy denials, caller cancellation, and requests
that arrived already expired.

| Dimension | Default objective | Typed degraded threshold | Hard failure classification |
| --- | --- | --- | --- |
| Scheduling | p99 enqueue-to-dispatch is at most 10 ms for session/ttrpc control and 50 ms for attachment/named-stream work. | The objective is exceeded for three consecutive one-minute buckets, or one otherwise-runnable item waits 1 second while it still has deadline budget: `degraded/scheduling-delay`, remediation `inspect-provider`. | Runnable work makes no dequeue progress for 10 seconds, or session control waits 5 seconds: close with `scheduler-stalled`, mark the endpoint `unavailable`, and use `restart-agent` for an agent or `replace-generation` for an in-process owner. |
| Queues | Each queue remains below 75% of its hard ceiling and backpressure rejects less than 1% of attempts over five minutes. | A queue remains at or above 75% for 30 seconds, or rejects reach 1% with at least 100 attempts (or 10 rejects with fewer attempts): `degraded/queue-pressure`, remediation `retry-bounded`. | Reserved session/ttrpc control credit is exhausted, or a full queue makes no dequeue progress for 5 seconds: close with `control-resource-exhausted` and mark the endpoint `unavailable`. An isolated named-stream ceiling rejects only that stream unless the no-progress rule also fires. |
| Handshake | Local handshakes have p99 at most 1 second; provider/remote handshakes have p99 at most 5 seconds. | The matching objective is exceeded for three consecutive buckets, or three consecutive transient transport/timeout failures occur: `degraded/handshake-timeout`, remediation `retry-bounded`. | Local and provider/remote handshake deadlines are 5 and 15 seconds respectively. Deadline expiry rejects that connection; three consecutive deadline expiries mark the endpoint `unavailable`. Authentication, transcript, purpose, role, schema, identity, or configuration mismatch rejects immediately and marks the configured endpoint `failed`, with no automatic retry. |
| Reconnect | A recoverable local endpoint re-establishes within 5 seconds and a provider/remote endpoint within 30 seconds, normally within three attempts. | The objective elapses or three consecutive attempts fail: `degraded/session-disconnected`, remediation `retry-bounded`. | Ten failed attempts or 5 minutes disconnected, whichever occurs first, marks the endpoint `unavailable`. An authentication/identity/configuration mismatch instead marks it `failed` immediately and requires `re-enroll-peer` or `repair-configuration`. |
| Provider health | Poll every 10 seconds; a local health call completes within 2 seconds, an agent/remote call within 10 seconds, and a successful observation is never more than 30 seconds old. | A provider reports degraded, three consecutive health calls fail, or the last success is older than 30 seconds: the matching `ProviderHealth` degraded reason and `inspect-provider`. | Agent disconnect is immediately `unavailable`. Ten consecutive health failures or a last success at least 5 minutes old is `unavailable`. Identity, configuration, generation, or capability mismatch is immediately `failed`; no operation is admitted and `repair-configuration` or `replace-generation` is required. |

An endpoint starts as `starting` and becomes `healthy` only after its first
successful handshake and, where applicable, provider-health observation.
`degraded` preserves safe admission with typed backpressure and visible
remediation. `unavailable` stops new calls, removes queued work, preserves or
quarantines ambiguous durable operations under their original operation IDs, and
continues only the bounded reconnect/health probe. `failed` performs no automatic
retry. Recovery from `unavailable` requires a fresh authenticated session and
health success. Recovery from `degraded` requires three consecutive successful
evaluation intervals, at least 30 seconds below 50% queue utilization, and no
remaining breached objective. Recovery from `failed` requires explicit
re-enrollment, configuration repair, or publication of a validated replacement
generation.

Status exposes state, closed reason, transition time, affected closed component
class, and closed remediation; it does not expose raw peer/provider output.
Metrics add only `locality`, `provider_type`, `health_state`, and closed
`result`/`reason` to the already allowed low-cardinality dimensions. Realm,
workload, provider ID, session, request, stream, operation, endpoint, and user
never become metric labels.

### Complete internal IPC migration matrix

| Current boundary | v2 purpose and service | Transport/attachment contract | Migration |
| --- | --- | --- | --- |
| CLI and duplicated toolkit public JSON to host daemon | `daemon-local` or `daemon-remote`; `d2b.daemon.v2` | Unix stream/seqpacket locally, provider transport remotely; named streams for exec, PTY, logs, console, audit export, and files | Delete public JSON, semver hello, feature intersection, shadow DTOs, and duplicate framing. |
| `PeerSession`, `SecurePeerSession`, gateway realm routing | `realm-peer` or `realm-bootstrap`; `d2b.realm.v2` | `KK` or one-time `IKpsk2`; named display, clipboard, log, file, and shortcut streams | Delete custom HMAC/AEAD record layer, hand codec, and second mux. |
| d2bd to guestd HMAC/ttrpc | `guest-control` or `guest-bootstrap`; `d2b.guest.v2` | Native/CH vsock transport; `KK` after one-time bootstrap; PTY/log/file/security-key streams | Delete guest protocol 6, auth transcript v1, long-lived token, and old bindings. |
| Controller to workload/third-party provider | `provider-agent`; `d2b.provider.v2` | Stream transport; provider-defined bounded streams; no credential bytes | Replace gateway and provider-specific control wires with generated provider service and trait proxy. |
| Daemon to privileged broker protocol 3 | `privileged-broker`; `d2b.broker.v2` | Unix seqpacket with transcript-bound typed FD attachments | Delete one-request JSON protocol, bootstrap feature, fallback bind, and old capability list. |
| CLI/session to per-user interaction helper | `user-agent`; `d2b.user.v2` | Local Unix session; prompt/session handles only | Replace helper-specific framing; password bytes stay inside the prompt/Secret Service path. |
| Daemon to unsafe-local helper | `runtime-systemd-user`; `d2b.runtime.systemd-user.v2` | Local Unix session with exact terminal/Wayland FDs and named streams | Rename provider and agent; delete helper protocol 3 and generation hello. |
| Runtime agent to shell supervisor | `shell-supervisor`; `d2b.shell.v2` | Inherited or named local session with PTY attachment | Delete unsafe-local shell v1, terminal protocol 1, supervisor protocol 1, and heartbeat framing. |
| d2b-owned clipd management and UI picker | `clipboard-control` and `clipboard-picker`; `d2b.clipboard.v2` and `d2b.clipboard.picker.v2` | Local session; picker receives metadata streams and no transfer FD | Delete newline JSON and picker protocol v1 while preserving UI-only trust. |
| Wayland proxy to clipd bridge/readiness | `clipboard-bridge`; `d2b.clipboard.v2`, and proxy control; `d2b.wayland.v2` | Unix packet session; exactly one validated transfer FD for an accepted transfer | Delete unversioned/readiness framing and raw VM-name socket construction. |
| Desktop observer, notification, and action callbacks | `desktop-observer`; `d2b.notify.v2` | Bounded event stream with authenticated actions | Delete callback tokens and old notify framing; state files remain projections only. |
| Guest FIDO frontend and host security-key controller | `security-key`; `d2b.security-key.v2` | Fixed bounded CTAPHID report stream; CTAPHID remains an external payload | Delete old protocol version/unversioned frames; preserve ceremony approval and closed command policy. |
| Activation helper and retained one-shot process helpers | Purpose-specific inherited socketpair; `d2b.activation.v2` or matching `.v2` service | Local `NN` session with exact inherited attachments | Fold into broker/provider methods where possible; otherwise delete bespoke hello/error framing. |
| TTY/raw-mode helper | `tty-helper`; `d2b.tty.v2` | Inherited local session with exact terminal FD | Delete bespoke framing and generation handling. |
| ACA, Relay, and display gateway helpers | `provider-agent`, `realm-peer`, or `daemon-remote`; matching `.v2` service | Provider transport plus ComponentSession and bounded named streams | Fold into typed provider agents and realm services; delete gateway-specialized wires. |

Discovery of another d2b-owned live IPC during implementation blocks the owning
wave until it is added to this matrix and either migrated or deleted. It cannot
be grandfathered as a specialized exception.

Durable exec specifications, records, and status files remain versioned
persistence records, not ComponentSessions. Waybar stdout JSON and other
desktop files may remain bounded presentation projections; no projection is an
authorization, repair, or live control channel.

## Filesystem identity, state, and audit

### Exact short-ID derivation

Human realm, workload, provider, and role names remain in canonical targets and
metadata. Runtime identities use the first 96 bits of SHA-256 output and the
lowercase unpadded RFC 4648 base32 alphabet:

```text
abcdefghijklmnopqrstuvwxyz234567
```

For each ID, construct one canonical printable-ASCII string with this grammar:

```text
encoded = "d2b-id-v2;" decimal ":" domain ";" decimal ";"
          *(decimal ":" part ";")
decimal = "0" / (nonzero-digit *digit)
```

The first decimal is the ASCII byte length of `domain`; the second is the exact
part count; each following decimal is the ASCII byte length of the immediately
following part. There are exactly that many part fields and no trailing bytes
after the final `;`. Zero is spelled only `0`; every positive value has no
leading zero. Domains and parts are non-empty in the four ID contracts below.
The prefix, delimiters, decimal lengths, domains, and canonical parts use only
printable ASCII bytes. There is no NUL, control-byte construction, binary
integer, locale conversion, escaping, or implicit concatenation. For example,
the part lists `["ab", "c"]` and `["a", "bc"]` serialize respectively with
suffixes `2;2:ab;1:c;` and `2;1:a;2:bc;`.

Nix constructs the string with interpolation and `builtins.stringLength`, then
passes it directly to `builtins.hashString "sha256"`. Rust emits the same ASCII
grammar into a byte vector and hashes it with SHA-256. Both take digest bytes 0
through 11 and encode them as exactly 20 lowercase unpadded base32 characters:

```text
realm-id:
  domain = "d2b-v2:realm"
  parts  = [canonical-realm-path]

workload-id:
  domain = "d2b-v2:workload"
  parts  = [realm-id, canonical-workload-name]

provider-id:
  domain = "d2b-v2:provider"
  parts  = [realm-id, provider-type, configured-provider-id]

role-id:
  domain = "d2b-v2:role"
  parts  = [realm-id, workload-id, canonical-role]
```

The canonical inputs are frozen:

- `canonical-realm-path` is exactly `local-root` for the root. A child appends
  its schema-validated lowercase ASCII kebab-case label before its parent's
  path, separated by one literal `.`, so the order is leaf to root:
  `personal-dev.dev.local-root`. Empty labels, repeated separators, a trailing
  separator, and the public target suffix `.d2b` are forbidden.
- `canonical-workload-name` and `configured-provider-id` are their exact
  schema-validated lowercase ASCII kebab-case spellings.
- `provider-type` is exactly one closed wire spelling: `runtime`,
  `infrastructure`, `transport`, `substrate`, `credential`, `display`,
  `network`, `storage`, `device`, `audio`, or `observability`.
- `canonical-role` is exactly one initial `RoleKind` wire spelling:
  `store-virtiofs-preflight`, `swtpm-pre-start-flush`, `swtpm`, `virtiofsd`,
  `video`, `gpu`, `gpu-render-node`, `audio`, `cloud-hypervisor`,
  `qemu-media`, `vsock-relay`, `guest-control-health`, `usbip`,
  `security-key-frontend`, or `wayland-proxy`. It is not a display label,
  provider implementation ID, or Rust variant spelling. A new role requires a
  versioned schema change, a new canonical vector, and regenerated endpoint
  proof before it may acquire a role ID.

No Unicode normalization, locale case folding, alternate root-to-leaf order,
`.d2b` suffix, separator substitution, or enum display spelling is accepted.

Renaming a realm, workload, provider instance, or role creates a new identity
and resource. It is not an in-place path rename.

Nix evaluation detects every collision in the complete generated configuration.
Runtime bundle loading independently recomputes every ID and rejects collisions
and duplicate globally scoped `ProviderId` values before opening a mutable
resource. Nix and Rust implement the derivation independently and compare
committed vectors; neither consumes IDs generated by the other. Collision
detection, not probability, is the correctness boundary.

The pure-Nix hexadecimal-to-base32 helper is specified by this implementation.
It intentionally uses arithmetic over individual bytes rather than constructing
binary strings or relying on a Nix-specific base32 alphabet:

```nix
let
  alphabet = "abcdefghijklmnopqrstuvwxyz234567";
  field = value:
    "${builtins.toString (builtins.stringLength value)}:${value};";
  encode = domain: parts:
    "d2b-id-v2;${field domain}${builtins.toString (builtins.length parts)};"
    + builtins.concatStringsSep "" (builtins.map field parts);
  mod = dividend: divisor:
    dividend - (builtins.div dividend divisor) * divisor;
  hexNibble = {
    "0" = 0; "1" = 1; "2" = 2; "3" = 3;
    "4" = 4; "5" = 5; "6" = 6; "7" = 7;
    "8" = 8; "9" = 9; "a" = 10; "b" = 11;
    "c" = 12; "d" = 13; "e" = 14; "f" = 15;
  };
  powersOfTwo = [ 1 2 4 8 16 32 64 128 ];
  nibbleAt = hex: offset:
    let c = builtins.substring offset 1 hex;
    in if builtins.hasAttr c hexNibble
       then builtins.getAttr c hexNibble
       else throw "short-id digest is not lowercase hexadecimal";
  byteAt = hex: index:
    16 * nibbleAt hex (2 * index) + nibbleAt hex (2 * index + 1);
  bitAt = hex: bit:
    let
      byte = byteAt hex (builtins.div bit 8);
      divisor = builtins.elemAt powersOfTwo
        (7 - mod bit 8);
    in mod (builtins.div byte divisor) 2;
  symbolAt = hex: index:
    let
      firstBit = index * 5;
      value = builtins.foldl'
        (acc: offset:
          acc * 2
          + (if firstBit + offset < 96
             then bitAt hex (firstBit + offset)
             else 0))
        0
        (builtins.genList (offset: offset) 5);
    in builtins.substring value 1 alphabet;
  base32First96 = hex:
    assert builtins.stringLength hex == 64;
    builtins.concatStringsSep ""
      (builtins.genList (index: symbolAt hex index) 20);
  shortId = domain: parts:
    base32First96
      (builtins.hashString "sha256" (encode domain parts));
in
  { inherit encode base32First96 shortId; }
```

`base32First96` accepts only the 64-character lowercase hexadecimal result from
`builtins.hashString "sha256"`. Bits are consumed most-significant first in RFC
4648 order; the last symbol contains the final digest bit followed by four zero
padding bits. It never converts the 96-bit prefix into one Nix integer. The Rust
implementation independently serializes the grammar, hashes it with the `sha2`
crate, and applies RFC 4648 base32 without padding; it does not call generated
Nix or consume Nix-produced IDs.

The following vectors are canonical. Each implementation verifies the encoded
ASCII string, its byte-for-byte hexadecimal form, full SHA-256 digest, and
20-character ID:

```text
realm
  parts   = ["dev.local-root"]
  encoded = "d2b-id-v2;12:d2b-v2:realm;1;14:dev.local-root;"
  bytes   = 6432622d69642d76323b31323a6432622d76323a7265616c6d3b313b31343a6465762e6c6f63616c2d726f6f743b
  sha256  = c2f477b152ecc7d1a89277a11d7465a8704f8ecfcb9282d3644e2b25ee46e04e
  id      = yl2hpmks5td5dkeso6qq

workload
  parts   = ["yl2hpmks5td5dkeso6qq", "personal-dev"]
  encoded = "d2b-id-v2;15:d2b-v2:workload;2;20:yl2hpmks5td5dkeso6qq;12:personal-dev;"
  bytes   = 6432622d69642d76323b31353a6432622d76323a776f726b6c6f61643b323b32303a796c3268706d6b7335746435646b65736f3671713b31323a706572736f6e616c2d6465763b
  sha256  = 874ff4ce132119f5501c996a6bf57b23f504bcb892928fe2c20198f4ce0cba90
  id      = q5h7jtqteem7kua4tfva

provider
  parts   = ["yl2hpmks5td5dkeso6qq", "runtime", "primary"]
  encoded = "d2b-id-v2;15:d2b-v2:provider;3;20:yl2hpmks5td5dkeso6qq;7:runtime;7:primary;"
  bytes   = 6432622d69642d76323b31353a6432622d76323a70726f76696465723b333b32303a796c3268706d6b7335746435646b65736f3671713b373a72756e74696d653b373a7072696d6172793b
  sha256  = 2ff3b5749b058cde6c0b4cf41c4dc286919a7cd46d5737d2692021c268182a41
  id      = f7z3k5e3awgn43aljt2a

role
  parts   = ["yl2hpmks5td5dkeso6qq", "q5h7jtqteem7kua4tfva", "cloud-hypervisor"]
  encoded = "d2b-id-v2;11:d2b-v2:role;3;20:yl2hpmks5td5dkeso6qq;20:q5h7jtqteem7kua4tfva;16:cloud-hypervisor;"
  bytes   = 6432622d69642d76323b31313a6432622d76323a726f6c653b333b32303a796c3268706d6b7335746435646b65736f3671713b32303a713568376a74717465656d376b756134746676613b31363a636c6f75642d68797065727669736f723b
  sha256  = fde214b9b2247677a3e78393063b638b5d3fe8d47e6f7333d1f1edc269553caa
  id      = 7xrbjonser3hpi7hqojq
```

For one million IDs in one domain, the 96-bit birthday bound is less than
`6.4e-18`:

```text
p <= n * (n - 1) / 2^97
```

Domain separation prevents a realm/workload/provider/role cross-type collision
from becoming the same typed identity even if rendered text matches.

W2 adds the ID evaluation measurement to the existing
`tests/unit/gates/performance-budgets.sh` gate; it does not add a new top-level
gate. With `D2B_PERF_STABLE=1` on the pinned x86_64-linux self-hosted runner, the
fixture derives 4,096 IDs (1,024 complete realm/workload/provider/role chains),
checks collisions, and expands every applicable socket-template row. After one
unmeasured warm-up, the median of three single `nix eval` runs has a 5,000 ms
budget (6,000 ms including the gate's declared 20% enforcement margin), and each
run has a 512 MiB peak-RSS ceiling. Correctness vectors, grammar rejection, and
the complete socket proof remain ordinary Layer-1 coverage on every PR even when
the stable-runner performance lane is unavailable.

### Unix socket-length proof

Linux pathname Unix sockets provide 108 bytes in `sockaddr_un.sun_path`,
including the terminating NUL, so pathname bytes must be at most 107.

The generated `socket-endpoints-v2` contract is a closed table. Every row names
one template ID, literal path template and leaf, endpoint purpose, protocol
owner, creator, accepting owner, mode/group, substitution types, maximum
substituted byte count, and repair owner. There is no unconstrained
`<socket-leaf>` field. The initial complete expanded template set is:

| Template ID | Exact template | Maximum pathname bytes |
| --- | --- | ---: |
| `local-root-public` | `/run/d2b/root.sock` | 18 |
| `local-root-broker` | `/run/d2b/broker.sock` | 20 |
| `child-realm-public` | `/run/d2b/r/<realm-id>/public.sock` | 43 |
| `child-realm-broker` | `/run/d2b/r/<realm-id>/broker.sock` | 43 |
| `provider-agent` | `/run/d2b/r/<realm-id>/p/<provider-id>/agent.sock` | 65 |
| `role-control` | `/run/d2b/r/<realm-id>/w/<workload-id>/roles/<role-id>/control.sock` | 94 |
| `role-api` | `/run/d2b/r/<realm-id>/w/<workload-id>/roles/<role-id>/api.sock` | 90 |
| `role-qmp` | `/run/d2b/r/<realm-id>/w/<workload-id>/roles/<role-id>/qmp.sock` | 90 |
| `role-virtiofs` | `/run/d2b/r/<realm-id>/w/<workload-id>/roles/<role-id>/virtiofs.sock` | 95 |
| `role-tpm` | `/run/d2b/r/<realm-id>/w/<workload-id>/roles/<role-id>/tpm.sock` | 90 |
| `role-audio` | `/run/d2b/r/<realm-id>/w/<workload-id>/roles/<role-id>/audio.sock` | 92 |
| `role-video` | `/run/d2b/r/<realm-id>/w/<workload-id>/roles/<role-id>/video.sock` | 92 |
| `role-wayland` | `/run/d2b/r/<realm-id>/w/<workload-id>/roles/<role-id>/wayland.sock` | 94 |
| `workload-guest` | `/run/d2b/r/<realm-id>/w/<workload-id>/sockets/guest.sock` | 73 |
| `workload-display` | `/run/d2b/r/<realm-id>/w/<workload-id>/sockets/display.sock` | 75 |
| `workload-security-key` | `/run/d2b/r/<realm-id>/w/<workload-id>/sockets/security-key.sock` | 80 |
| `user-agent` | `/run/d2b/u/<uid-decimal>/userd.sock` | 32 |
| `user-runtime-agent` | `/run/d2b/u/<uid-decimal>/runtime-agent.sock` | 40 |
| `clipd-control` | `/run/d2b/u/<uid-decimal>/clipd/control.sock` | 40 |
| `clipd-picker` | `/run/d2b/u/<uid-decimal>/clipd/picker.sock` | 39 |
| `clipd-bridge` | `/run/d2b/u/<uid-decimal>/clipd/bridge.sock` | 39 |
| `one-shot-inherited` | no pathname; one fresh socketpair from the declared parent | not applicable |

W2 owns one versioned machine-readable endpoint descriptor and generates from it
the Rust `SocketTemplateId`/descriptor table, the Nix table, the reference
schema, and a complete proof artifact. The proof expands every leaf separately
and records literal segments, substitution maxima, the longest rendered path,
its exact ASCII byte count, and remaining bytes before the terminating NUL. It
also proves both directions: every rendered system/user socket unit, bundle
endpoint, broker-created listener, and d2b-owned pathname bind references
exactly one template ID, and every generated template row is referenced or is
the explicit inherited-socketpair row.

Production pathname-listener APIs accept a `SocketTemplateId` plus typed
substitutions, never a free-form path or leaf. A Layer-1 source policy rejects a
d2b-owned pathname `bind` or socket-unit declaration outside that generated
surface. A contract test compares the complete rendered endpoint multiset with
the generated proof, so a new socket cannot be omitted by updating only one
consumer.

The proof substitutes 20 bytes for each derived ID and at most 10 ASCII digits
for a Linux `uid_t`. The longest row is `role-virtiofs` at 95 bytes, leaving 12
pathname bytes before the required NUL. Any new socket or leaf first extends the
generated closed table and its checked proof. Nix evaluates every instantiated
row and Rust independently recomputes every exact byte length before binding;
either rejects a path above 107 before side effects. The runtime socket root is
fixed at `/run/d2b`; abstract Unix sockets, user-configured roots, and
path-shortening symlinks are not used.

### Layout

The v2 layout is:

```text
/etc/d2b/
  host/
  r/<realm-id>/
    controller.json
    providers.json
    storage.json
    sync.json
    w/<workload-id>/

/var/lib/d2b/
  host/
    allocator/
    broker/
    audit/
  r/<realm-id>/
    controller/
    broker/
    audit/
    providers/<provider-id>/
    w/<workload-id>/
      state/
      disks/
      store-view/{live,meta,state,gcroots}/
      tpm/
      media/
      audio/
      keys/

/var/cache/d2b/
  host/
  r/<realm-id>/

/run/d2b/
  root.sock
  broker.sock
  r/<realm-id>/
    public.sock
    broker.sock
    locks/
    p/<provider-id>/
    w/<workload-id>/
      roles/<role-id>/
      sockets/
      leases/
  u/<uid>/
    userd.sock
    runtime-agent.sock
    clipd/
```

No raw canonical target, old VM name, configured provider label, endpoint,
device ID, bus ID, command value, or user-supplied string participates in path
construction. There are no compatibility symlinks from old roots or names.

### Single repair owner

Endpoint creation and ownership is complete and closed:

| Endpoint/path class | Path creator/binder | Connection owner | Unlink/repair owner |
| --- | --- | --- | --- |
| Fixed local-root public socket | PID1 from the generated `d2bd.socket` unit | Local-root `d2bd` receives only the activated listener FD | PID1/system unit lifecycle |
| Fixed local-root broker socket | PID1 from the generated `d2b-priv-broker.socket` unit | Local-root broker receives only the activated listener FD | PID1/system unit lifecycle |
| Child-realm public and broker sockets | Local-root allocator, using generated endpoint rows before parent spawn | Matching child controller and separate child broker receive their respective already-bound listener FDs | Local-root allocator |
| Fixed `userd.sock` and `runtime-agent.sock` | Exact user's systemd manager from generated user socket units | Matching `d2b-userd` or `systemd-user` runtime agent | Owning user manager/user unit lifecycle |
| Fixed clipd `control.sock`, `picker.sock`, and `bridge.sock` | Exact user's systemd manager from generated user socket units | The matching clipd component selected by endpoint policy | Owning user manager/user unit lifecycle |
| Dynamic provider `agent.sock` listener | Owning realm broker, using its delegated parent dirfd and generated endpoint row | Broker passes the already-bound listener FD to exactly one provider agent | Owning realm broker |
| Dynamic workload `guest.sock`, `display.sock`, and `security-key.sock` listeners | Owning realm broker, using delegated dirfds and generated endpoint rows | Broker passes each already-bound listener FD to the declared workload component | Owning realm broker |
| Dynamic role `control.sock` and external-protocol role sockets | Owning realm broker, using delegated dirfds and generated endpoint rows | Broker passes each already-bound listener FD to the declared runner/proxy | Owning realm broker |
| One-shot inherited/socketpair endpoint | Declared launching parent through the typed launch operation | The two declared endpoints only | Creating parent until handoff; then kernel close, with no path |

Systemd/tmpfiles creates only the fixed top-level directory anchors and the
per-user anchor needed for user-manager activation. Fixed local-root and user
socket nodes are the narrow exception to broker creation below an anchor:
socket activation binds them before the service starts, and their exact set is
enumerated in the generated endpoint contract. Of broker endpoints, only the
local-root broker is socket-activated by PID1. Child-realm public and broker
listeners are allocator-owned dynamic paths, pre-bound before parent spawn, and
are never adopted as PID1-owned paths.

All other paths below an anchor are created and repaired only by the owning
broker from generated storage, sync, and endpoint IDs. A daemon, controller,
provider agent, runner, proxy, user helper, or diagnostic command never binds,
unlinks, renames, chmods, chowns, or recreates a broker-owned socket path. It
receives a listener FD, validates the endpoint row, and accepts connections.
The local-root broker owns host-global allocator state; a child broker operates
only through its delegated dirfds and leases.

Daemons send opaque IDs, never raw paths, modes, owners, ACLs, or free-form
repair instructions. Broker path walking is anchored and fd-relative, using
`openat2` with `RESOLVE_BENEATH`, `RESOLVE_NO_SYMLINKS`,
`RESOLVE_NO_MAGICLINKS`, and `RESOLVE_NO_XDEV` unless a generated storage row
explicitly permits the crossing. Leaf operations use `O_NOFOLLOW`.

Activation, a user helper, a provider agent, and a diagnostic command cannot
repair broker-owned state. A diagnostic ledger is evidence, never authority.

### Atomic JSON state

V2 uses no embedded state database. A shared `d2b-state` crate owns all
authoritative JSON persistence and enforces:

- closed schema and version headers;
- `deny_unknown_fields`;
- bounded reads and decode allocation;
- writer identity and monotonic generation;
- exact owner, group, and mode checks;
- same-directory temporary creation;
- complete write and file fsync;
- atomic rename;
- parent-directory fsync;
- checksum where corruption detection is required;
- typed quarantine for corrupt or ambiguous state;
- no success-shaped default on missing or invalid authority.

This is an ordered durability protocol, not a list of optional best practices.
The writer must fsync the complete temporary file before rename, atomically
rename it, open and fsync the parent directory after the rename, and return
success only after that directory fsync succeeds. A file fsync without the
post-rename parent-directory fsync is not a successful authoritative write.

Each authority writes only its declared records. State adoption verifies the
live kernel/provider identity and generation. Pidfds, sockets, locks, session
keys, and Noise cipher state are never serialized.

After the one-time factory reset, ordinary daemon restart is again a
continuation event. An owner discovers and verifies adoptable resources before
cleanup. Ambiguity quarantines the narrow scope and provides repair-forward
remediation; it does not broad-sweep `/run/d2b`.

### Synchronization

Generated sync rows define every OFD lock, in-process lock, kernel object, and
FD-backed lease. Framework file locks use `F_OFD_SETLK`, open with
`O_CLOEXEC`, follow one generated total order, and cross a process boundary
only through an explicit ComponentSession attachment rule. Fork inheritance is
not ownership transfer.

### Audit segmentation

Audit is separate from state snapshots:

- local-root allocator/broker audit is under `/var/lib/d2b/host/audit`;
- each realm audit is under `/var/lib/d2b/r/<realm-id>/audit`;
- segments are append-only, versioned JSONL with sequence numbers and an
  internal hash chain;
- sealed segment summaries bind first/last sequence, previous segment digest,
  segment digest, realm/controller generation, creation, and prune metadata;
- retention and rotation are bounded; the default is 14 days and a realm may
  select a stricter policy but may not disable retention;
- signed realm checkpoints are used where a replaceable controller exports
  audit;
- an unavailable segment or forced loss creates an explicit gap record, never
  a fabricated successful continuation.

Cross-realm operations produce bounded correlated records in every authority
that made a decision. Raw payloads, commands, endpoints, credentials, keys,
proofs, paths, and user-provided labels are excluded. Audit checkpoints do not
give a parent repair authority over child state.

## User secrets and realm keys

### Interactive Secret Service

GNOME Keyring is an explicit NixOS dependency and the interactive Secret
Service source of truth. SDDM/PAM unlocks the login keyring for the Niri
session. The design does not rely on an incidental dependency pulled in by a
desktop module.

`d2b-userd` is the per-user interaction and secret broker. It:

- runs under the user's systemd manager;
- uses the Rust `oo7` client;
- accepts only `d2b.user.v2` ComponentSessions;
- owns trusted graphical prompts and interaction handles;
- returns secret status or opaque export/lease handles, not secret bytes, to
  d2bd;
- in the explicit pre-reset preparation mode, requires an existing unlocked
  owning session, deletes only fixed-attribute d2b items, revokes that user's
  scoped exports, proves both inventories absent, and emits the digest-bound
  completion-receipt payload consumed by the root receipt coordinator;
- never forwards Secret Service or the whole keyring into a guest.

`d2b secret unlock` accepts either:

- a no-echo TTY prompt: the CLI transfers only the already-open terminal handle
  to `d2b-userd`; after reading the password from that terminal, `d2b-userd`
  directly `execve`s the configured `gnome-keyring-daemon --unlock` backend
  with an inherited pipe as stdin, writes the password only to that pipe, and
  closes it; or
- a trusted graphical flow owned by `d2b-userd`, which may use the Secret
  Service prompt API.

The TTY backend uses no shell. Its argv contains only the fixed executable and
`--unlock`; the password is absent from argv and environment, and backend
stdout/stderr is not logged or returned. `oo7` is the Secret Service client
after unlock, not a claimed TTY-unlock API. W6 begins with an implementation
spike against the pinned GNOME Keyring version to prove stdin consumption,
login-collection unlock, process lifetime, and exit semantics. If those
semantics differ, implementation stops for a reviewed backend decision and
fails closed; it does not pass the password through ComponentSession or invent
a shell/argv/environment fallback.

Passwords never enter argv, environment, Nix, bundle JSON, logs, traces,
metrics, audit, or an ordinary ComponentSession payload.

An unlocked Secret Service is ambient to processes with the same uid under the
desktop's normal security model. `d2b-userd` does not claim to isolate secrets
from a malicious same-uid process. This is why the systemd-user runtime agent
is separate and has no keyring or credential authority.

### Scoped unattended export

An unattended agent receives only a secret explicitly exported for one
host/user/service tuple. Export produces a TPM2-sealed encrypted systemd
credential with:

- source item version;
- export generation;
- owning user and target service identity;
- allowed purpose;
- rotation and expiry metadata.

Cryptographic sealing is bound to TPM, host, user, and credential name and is
intentionally not bound to generation-sensitive PCR values that would break
every NixOS switch. It is not cryptographically bound to a systemd unit or
service identity. PID1 enforces unit/service isolation through the generated
credential assignment, unit identity, DAC, private mount namespace, and service
sandbox configuration.

At service start, systemd decrypts the credential and materializes it at a
manager-owned read-only `$CREDENTIALS_DIRECTORY/<credential-name>` path assigned
only to the target unit. The service reads that path; the plaintext is not an
environment variable, command argument, Nix value, or host bundle field. A host
or TPM replacement requires interactive re-export. Rotation creates a new
generation, restarts or reloads only the target service under explicit policy,
and revokes the old export after bounded handoff. Exporting one item never
unlocks or copies the whole keyring.

Factory reset does not treat TPM export deletion as a root-mode substitute for
user revocation. The owning `d2b-userd` revokes the export while its user
authority is live, and the root side removes the materialization through the
typed reset-preparation operation. Reset mode accepts only the authenticated
absence receipt; it has no Secret Service or export-unseal authority.

### Host-local realm Noise keys

Each host-local realm has a separate static Noise identity:

1. generate the private key in the owning controller process for that realm
   only;
2. for local-root, seal it as a TPM2-backed systemd credential and have PID1
   assign it only to local-root `d2bd`, with DAC, mount-namespace, and service
   sandbox isolation;
3. for a parent-spawned child controller, pass only its generated key-state
   dirfd and allocator-approved TPM resource-manager FD; the controller creates
   or unseals its realm-specific TPM object after entering its namespaces and
   cgroup leaf, while the allocator and brokers never receive a plaintext key
   buffer or credential file;
4. enroll only the public key and generation with parent/children;
5. rotate through an authenticated parent-authorized generation transition;
6. revoke the prior generation and close its live sessions.

Private realm keys never enter Nix, `/etc/d2b`, bundle JSON, provider state,
audit, logs, or another realm controller. Brokers do not read them. Loss of a
realm key requires explicit re-enrollment or realm recreation; there is no
ledger-based repair.

Host-local realm keys are distinct from user secrets and cloud/provider
credentials. D2bd and brokers never read user secret bytes.

## Public v2 surface

The public configuration is realm/workload/provider based. It has no productive
or removed-option compatibility declarations for:

- `d2b.vms`;
- user-facing `d2b.envs`;
- gateway objects;
- relay/provider-placeholder records;
- old local VM kinds or provider-kind aliases.

The v2 CLI begins with:

```text
d2b realm ...
d2b workload ...
d2b provider ...
d2b auth ...
d2b secret ...
d2b host ...
```

`d2b vm`, `d2b env`, bare VM aliases, node-qualified aliases, and legacy target
forms do not exist. Canonical workload targets are always fully qualified:

```text
personal-dev.dev.local-root.d2b
```

The normalized index computes realm ownership, workload roles, provider
bindings, transport connectors, generated storage IDs, and capability
requirements once. Guest modules consume only their own precomputed row and do
not recursively scan realm/provider configuration.

## Cargo workspace and toolkit ownership

### One workspace

All host and guest crates use the repository root Cargo workspace and one
`Cargo.lock`. The broker and guest helpers are not separate workspaces, and no
nested workspace or second lockfile is permitted.

The root sets:

```toml
[workspace]
resolver = "2"

[workspace.package]
version = "2.0.0"
```

Every publishable and internal crate uses `version.workspace = true`. Common
edition, license, rust-version, lints, and dependency versions are inherited
where Cargo supports it.

The portable shared crates `d2b-core`, `d2b-realm-core`, `d2b-contracts`,
`d2b-provider`, `d2b-provider-toolkit`, `d2b-session`, `d2b-state`, and
`d2b-client` each declare `[features] default = []`. Every consumer sets
`default-features = false` and names the exact features it needs. Host-only Nix
integration, host Unix/socket/SCM_RIGHTS integration, and cloud SDK integration
are optional dependency families classified respectively as `host-nix`,
`host-socket`, and `cloud-sdk`; none is enabled by a portable crate default.
Here `host-socket` does not include the guest's explicitly selected native
vsock transport.

`d2b-contracts` remains one canonical crate with no default features:

```text
session
daemon
realm
guest
provider
broker
bundle
```

Consumers use `default-features = false` and select only required families.
Feature combinations are compile-tested. No feature re-exports a d2b 1.x type.

The framework guest-artifact inventory initially contains `d2b-guestd` and
`d2b-exec-runner`. `d2b-guestd` exposes exactly one package feature,
`guest-control`, which selects only `d2b-contracts/session` and
`d2b-contracts/guest`; `d2b-exec-runner` has an empty feature set. Root tooling
runs these isolated commands, never `--workspace` or `--all-features`:

```text
cargo check --locked -p d2b-guestd --no-default-features --features guest-control
cargo build --locked -p d2b-guestd --no-default-features --features guest-control
cargo check --locked -p d2b-exec-runner --no-default-features
cargo build --locked -p d2b-exec-runner --no-default-features
```

An `xtask` feature-graph test resolves those exact roots and feature sets with
Cargo metadata and fails if `host-nix`, `host-socket`, `cloud-sdk`, or any
dependency reachable only through one of those families enters either guest
artifact. A new framework guest artifact cannot land until its exact isolated
check/build rows and forbidden-feature proof are added.

This guest inventory does not reclassify a configured cloud provider agent as
guest framework code. The `azure-container-apps` and `azure-relay` agents remain
separate, explicitly selected package closures in their configured
credential-owning workload, co-located with their private credential module as
required by the placement table. Their explicit cloud SDK features never enter
`d2b-guestd` or `d2b-exec-runner`.

Focused canonical crates include:

```text
d2b-client
d2b-contracts
d2b-core
d2b-provider
d2b-provider-toolkit
d2b-realm-controller
d2b-realm-core
d2b-realm-router
d2b-session
d2b-session-unix
d2b-state
type-first provider implementations
composition binaries: d2bd, broker, guestd, userd, CLI, clipd,
  notify, Wayland proxy, security-key frontend, runtime agent
```

The canonical `d2b-client` and `d2b-provider-toolkit` crates live in this
repository and are the only implementations of their respective v2 contracts.

### Distribution repositories

The existing sibling repository `vicondoa/d2b-toolkit` is renamed to
`vicondoa/d2b-client-toolkit`. All of its crates, package names, flake outputs,
and Nix share paths use `d2b-client-toolkit`. It consumes and re-exports the
exact canonical `d2b-client` source artifact from a d2b release. It contains no
duplicate DTO, framing, handshake, or resolver.
The current `d2b-wayland-proxy` package-name collision between repositories is
removed in this rename; one canonical package/binary owner remains.

A new sibling distribution repository,
`vicondoa/d2b-provider-toolkit`, packages the exact canonical in-tree
`d2b-provider-toolkit` source artifact plus:

- provider-agent templates;
- fake SDK examples;
- conformance entrypoints;
- provider author documentation;
- Nix packaging and integration.

Neither toolkit family publishes crates to crates.io. They ship through GitHub
release source artifacts and flake/path dependencies with exact source
fingerprints.

This GitHub/flake-only distribution is fixed. The canonical SDK closure,
including `d2b-client`, `d2b-provider-toolkit`, `d2b-contracts`,
`d2b-provider`, and `d2b-session`, sets `publish = false`; crates.io publishing
is not a supported fallback. Path dependencies into the fingerprinted release
source are expected and version-only registry dependencies are rejected.
`xtask` validates every canonical SDK manifest, rejects a release plan or
automation command containing `cargo publish`, and fails if any SDK crate
becomes publishable.

The v2 cutover atomically migrates:

- `d2b-client-toolkit`;
- `d2b-provider-toolkit`;
- `d2b-wlterm`;
- `d2b-wlcontrol`;
- the WeezTerm integration seam.

No old repository name, crate, package, share path, public framing
implementation, target alias, or protocol adapter remains in any sibling.

## Checked deletion inventory

Every box is a W10 or W11 seal requirement. Source deletion boxes require
source-policy or behavior evidence on the immutable W10 tree; physical
reset/private-manifest boxes close only on the final manifest-free W11 tree.

- [ ] Delete public JSON framing, little-endian length wire, semver hello,
      feature intersection, old public request parser, shadow client DTOs, and
      duplicate toolkit framing.
- [ ] Delete `configured-launch-v1`, `unsafe-local-provider-v1`,
      `unsafe-local-shell-v1`, and every other legacy feature string.
- [ ] Delete `PeerSession`, `SecurePeerSession`, custom realm HMAC/AEAD records,
      gateway-display HMAC, guest HMAC challenge, notification callback tokens,
      helper generation hellos, and the duplicate mux/codec.
- [ ] Delete guest protocol 6, schema v2, auth transcript v1, long-lived guest
      token files, old generated guest bindings, and SSH/old-generation
      compatibility branches.
- [ ] Delete broker protocol 3, bootstrap feature negotiation, fallback
      self-bind path, old capability list, and one-request JSON dispatcher.
- [ ] Delete unsafe-local helper protocol 3, terminal protocol 1, shell
      supervisor protocol 1, and specialized heartbeat/generation framing.
- [ ] Delete picker, readiness, notify, Wayland bridge, activation, TTY, and
      security-key old versions or unversioned frames.
- [ ] Delete `TargetName`, node alias fields, `legacy_vm_name`, fast paths,
      flat VM lookup, old target routing, bare aliases, and manifest 0.4
      compatibility.
- [ ] Delete duplicated provider DTOs, errors, traits, registries, synthetic
      facades, and every dead provider interface without a selected v2 owner.
- [ ] Delete or replace `d2b-realm-provider`, `d2b-realm-transport`,
      `d2b-realm-codec-protobuf`, `d2b-provider-host`, `d2b-provider-aca`,
      `d2b-provider-relay`, `d2b-gateway`, `d2b-gateway-runtime`,
      `d2b-daemon-access`, and `d2b-unsafe-local-helper` after their useful
      invariants move.
- [ ] Delete every old provider/gateway crate and Nix package reference,
      including axis-free aliases and re-export-only wrapper crates.
- [ ] Delete `d2b.vms`, user-facing `d2b.envs`, gateway, relay,
      provider-placeholder, old workload-kind, and provider-kind option
      declarations, examples, tombstones, and aliases.
- [ ] Delete `d2b vm`, `d2b env`, old launch/target verbs, bare aliases, and all
      old client routing.
- [ ] Delete `/var/lib/d2b/vms`, `/run/d2b/vms`, `/run/d2b-gpu`,
      `/run/d2b-video`, `/run/d2b-wlproxy`, flat guest-control/key/TPM-marker/
      media/migration/gateway/lock/audit/daemon-report roots, user-global
      unsafe-local ledgers, activation ACL repair, per-VM path construction,
      legacy socket names, and compatibility symlinks.
- [ ] Delete old workload disks, TPM state, keys, tokens, caches, ledgers,
      audits, user-helper state, sockets, locks, and generated runtime material
      on the physical host through the digest-confirmed factory reset.
- [ ] Boot the physical host through the persistently selected dedicated reset
      generation, prove all d2b units/sockets and live references absent, and
      complete the locked/inhibited fd-relative reset before final-v2 boot.
- [ ] Before selecting reset mode, complete the confirmed live-session
      `d2b-userd` phase for every configured UID, revoke scoped exports, close
      the write barrier, and carry the exact authenticated receipt set into
      reset mode; no reset-mode Secret Service or unlock path may exist.
- [ ] Keep released reset code limited to generic generated current-anchor
      deletion. If W11 needs a literal outlier-root manifest, keep exactly one
      audited data-only copy in the private reset closure and remove the
      manifest, its closure roots, and every literal before the W11 final seal.
- [ ] Delete obsolete activation and group-migration helpers after their
      operations move to broker/provider contracts.
- [ ] Delete v1 golden outputs, fixtures, schema directories, state-migration
      tests, compatibility tests, and policy exceptions after v2 replacements
      are pinned.
- [ ] Delete stale shipped documentation describing old protocol, provider,
      daemon, filesystem, CLI, toolkit, or compatibility behavior.
- [ ] Rename the sibling client toolkit and delete every old crate, share path,
      flake output, release artifact, and consumer reference.
- [ ] Run pinned `cargo-udeps` and remove unused dependencies/features; deny dead
      production code except narrow generated-binding allowances.
- [ ] Prove no production parser, config, service registry, state reader,
      package, CLI, or test can recognize or successfully consume d2b 1.x.

Historical ADRs and released changelogs may describe v1 as history. Shipped v2
source comments, reference/how-to/explanation docs, CLI output, examples, and
schemas describe only v2.

## Delivery plan

### Dependency graph

```text
W0 ADR 0045 decision closure, Proposed panel, and Accepted/index panel
  -> W1 delivery/test/panel/stack tooling
      -> W2 v2 workspace, IDs, contracts, schemas, service definitions
          -> W3 session/provider/state/client foundations
              -> W4 first-party providers, provider registry, mapped local
                    runtime routing, and Azure VM scaffolding
                  -> W4-F frozen final-review candidate
                      -> P post-W4 shared contracts/tooling root
                          -> {W5 core control services,
                              W6 user/desktop/device services,
                              W7 remaining realm-native host boundaries}
                              in parallel
                      -> W9 toolkit/sibling cutover in parallel
content-frozen W5 -> W6 -> W7 delivery linearization
  -> W8 integrated realm/provider behavior and current-feature parity
W8 + W9
  -> W10 v1 purge and destructive reset tooling
      -> W11 private integrated physical-host cutover and hardening
          -> W12 merge train, v2.0 release, toolkit releases, final host pin
              -> W13 evidence-driven delivery streamlining
```

`W4-F` is the exact W4 head after focused preflight passes, the PR is updated,
and immutable validation, CI, and the final panel start. The post-W4 shared root
and W9 start from that head without waiting for W4 to merge or seal. A W4
content correction invalidates W4's candidate and rebases all speculative
branches. W4 and every content-changed descendant lose their snapshot,
validation, CI, panel, and seal evidence.

### Post-W4 parallel execution correction

W4 closed provider-facade findings by pulling several prerequisite and
integration surfaces forward. Later waves treat the resulting code as baseline:

| W4 baseline | Later-wave correction |
| --- | --- |
| Private `provider-registry-v2` Rust/Nix/schema artifact and bundle v12 integrity | W7 extends the existing binding/emitter for remaining axes; it does not create another provider registry artifact. |
| Startup-owned registry construction, exact effect bindings, and daemon restart on provider generations | W8 consumes the existing registry and never adds a parallel registry or reload authority. |
| Cloud Hypervisor and qemu-media mapped lifecycle routing, adoption, cancellation-safe cleanup, serialization, and complete lifecycle budgets | W8 retains parity coverage and removes the temporary unmapped fallback; it does not reimplement local runtime routing. |
| Bounded observability results, local observability mapping, and durable bounded export semantics | W7/W8 extend projection sources and routing without defining a second observability contract. |
| User-agent provider placement and owner-correct credential leases | W6 implements userd/keyring behavior against the W4 contract without changing its ownership model. |

The post-W4 execution rules are:

1. Create `adr0045-post-w4-contracts` from `W4-F`. It exclusively owns
   cross-wave DTO/protobuf/schema changes, anticipated workspace dependencies,
   `Cargo.lock`, shared policy/tooling, and per-wave delivery-manifest support.
   Its ownership checker and policy are trust inputs: a wave candidate never
   supplies the executable or policy that judges its own diff.
   It freezes the allocator boundary: W5 owns allocator service/dispatch
   implementation; W7 owns Nix/process/resource emission against that API.
2. Create W5, W6, and W7 as Git Town children of the shared root and open draft
   sibling PRs after wave-local prep commits. Wave-local prep owns only local
   shared files and file-ownership maps. Any newly discovered cross-wave
   contract returns to the root and rebases all three branches.
3. Shared tooling adds separate checked-in manifests under
   `delivery/manifests/w<N>.json`; sibling waves do not contend for one delivery
   manifest. Slice agents never edit the workspace lock, cross-wave generated
   contracts, shared policy, or another wave's files. The policy positively
   partitions implementation prefixes as follows:
   - W5: core CLI/client/daemon, realm, guest, provider-agent, broker, host, and
     allocator crates;
   - W6: userd, systemd-user/shell, clipboard, notify/wlcontrol, Wayland,
     security-key, activation, TTY, and retained helper crates;
   - W7: `nixos-modules/`, `pkgs/`, `examples/`, `templates/`, and Nix
     eval-test emission.
   Each wave's foreign set is exactly the union of the other two sets. Existing
   W4 implementation prefixes are frozen. An implementation path not positively
   classified to the current wave, including an exact prefix-root symlink or
   gitlink change, fails closed. Root shared contracts remain protected, and
   general exceptions are limited to explicit documentation paths/prefixes and
   the current wave manifest.
4. Before a wave branch is published, run ownership verification from a clean
   worktree at the exact immediate Git Town parent commit and pass the wave
   worktree only as the candidate. The parent-built checker derives the
   `adr0045-w5`, `adr0045-w6`, or `adr0045-w7` wave from the verified branch,
   corroborates Git Town parent/base and local head with the policy-pinned
   repository's unique open ordinary GitHub PR, loads policy from that exact
   parent commit, and diffs parent to head. Every wave ancestor is walked and
   corroborated to the shared root. All Git object and graph operations disable
   replace objects and bypass graft/shallow traversal; either worktree is
   rejected if `refs/replace`, `info/grafts`, or `shallow` metadata exists.
   Ownership/canonical diffs and cleanliness checks force submodule handling to
   `none`, overriding local `diff.ignoreSubmodules` configuration so gitlink
   additions and type changes remain visible. The checker accepts no
   caller-selected wave or base and rejects a self/`HEAD` base. Before
   linearization the shared root is valid for all three; after linearization
   only W5 -> W6 -> W7 is valid.
5. W5/W6/W7 remain siblings during implementation. At content freeze, delivery
   is deterministically linearized W5 -> W6 -> W7 through Git Town parent
   changes **before W6/W7 create final snapshots or run final panels**. W6 and
   W7 run fresh validation, CI, and panels against their larger integrated
   trees; pre-linearization evidence is never reused. Later history-only
   retargets may reuse only panel records after the existing byte-identical
   content proof; new-history CI and manifest-declared validation rerun.
6. W9 proceeds independently for W4/root-frozen contracts. A sibling feature
   that consumes a W5/W6 service remains on a dependent child branch. W4/root
   corrections propagate root-to-leaf and invalidate their own and every
   content-changed descendant's candidate evidence.
7. W8 integration prep starts when W5/W6/W7 publish content-frozen APIs. Create
   `adr0045-w8-integration` as a Git Town child of the linearized W7 head; its PR
   base is W7 and its manifest records the ordered W5/W6/W7/W8 chain. Recreate
   or rebase it whenever a dependency changes.
8. Every wave still receives a separate immutable candidate, required tests,
   exact-head CI, and end-of-wave ten-role panel. A pending earlier panel never
   idles later speculative implementation.
9. Slice worktrees and real Cargo targets are removed immediately after
   integration. Retain only the primary clone and currently active shared-root
   or wave integration worktrees, including W8 after it starts.
10. A Rust `xtask` per-development-UID semaphore owns two OFD-locked slots across
   all worktrees. Its trusted directory is
   `${XDG_RUNTIME_DIR}/d2b-heavy-gates` when available, otherwise
   `${TMPDIR:-/tmp}/d2b-heavy-gates-$UID`. The selected parent is pinned by a
   CLOEXEC directory FD and accepted only when it is an invoking-UID-owned
   non-symlink directory without group/other write or a root-owned sticky
   world-writable directory. The per-UID mode-0700 directory is created/opened
   with `mkdirat`/`openat` plus `O_NOFOLLOW`; the exact files `slot-0.lock` and
   `slot-1.lock` are opened relative to its pinned FD with
   `O_RDWR|O_CREAT|O_CLOEXEC|O_NOFOLLOW` mode 0600 and verified regular,
   same-owner, single-link, and still bound to their names. Parent, directory,
   and slot identities are revalidated to reject rename-based split
   namespaces. The helper tries slot 0 then slot 1 with nonblocking OFD write
   locks every 250 ms for at most 30 minutes; unsupported OFD locking, unsafe
   metadata, or timeout fails closed with no `flock` fallback. Its parent
   retains the original CLOEXEC slot FD. Before `exec`, the gate child
   duplicates that same locked open-file description to a designated
   `D2B_HEAVY_GATE_FD` and clears `FD_CLOEXEC` on the duplicate, so the gate
   process hierarchy also retains the permit if the parent crashes. Before
   spawning, the wrapper replaces inherited `SIGCHLD=SIG_IGN` or
   `SA_NOCLDWAIT` state with a caught handler that `exec` resets to default, so
   the leader remains waitable. The parent runs the gate in a child process
   group, forwards termination signals, and parses `/proc/<pid>/stat` as bytes.
   Any wait or process-group observation failure keeps the permit while the
   wrapper kills the group. The exited leader remains unreaped as the PID/PGID
   identity anchor through every descendant-membership check and any repeated
   group signal; only after the last bare-PGID operation does the wrapper reap
   the leader and close its FD. Five consecutive process-table failures cause
   one final anchored `SIGKILL`, then leader reap and failure; killed
   descendants retain the locked gate FD until they exit. This ordering
   prevents a reused PGID from targeting an unrelated process group without an
   unbounded cleanup loop. Slot files persist and are never unlinked during
   acquisition. It wraps
   `make check`,
   `make test-integration`, `make test-host-integration`, `make test-hardware`,
   full-workspace final `cargo test`, and build-producing `nix flake check`.
   Focused tests and CI do not consume local permits. The shared root implements
   this wrapper before parallel final gates.
11. Serial ownership stops at the smallest connected component of the actual
    file-overlap graph. Once a shared prep commit lands, every dependency-ready
    independent component and wave launches concurrently. One persistent agent
    cannot accumulate unrelated axes or later review rounds merely to preserve
    context. At each final-stage entry and review round, the integrator records
    ready, launched, and concretely blocked components; conflict avoidance alone
    is not a valid blocker.

### Exact wave tasks

#### W0 - ADR decision closure

- Expand ADR 0045 to this implementation-ready decision.
- Keep ADR 0045 status `Proposed`; leave the eight historical ADR status headers
  and `docs/adr/README.md` unchanged for every Proposed candidate.
- Create evidence with the exact
  [external W0 bootstrap template](#external-evidence-storage-and-w0-bootstrap-template)
  and run the full ten-role panel against that exact Proposed content tree.
- Address safety and completeness findings without reopening the no-v1 decision.
- After the Proposed tree has 10/10 signoff, prepare a second immutable
  candidate that changes ADR 0045 to `Accepted`; changes the `Status` headers of
  ADRs 0010, 0015, 0028, 0032, 0034, 0042, 0043, and 0044 to
  `Superseded by ADR 0045`; updates all eight rows plus ADR 0045 in
  `docs/adr/README.md`; and includes the final generated/index artifacts. These
  prospective supersession edits do not land on a Proposed tree.
- Run a second full ten-role panel against that exact Accepted/index candidate.
  Its panel records and seal stay external to the candidate. The unmerged status
  headers become authoritative only after this second panel is also 10/10 and
  the candidate lands.
- If either panel finds a content defect, make the correction on a Proposed
  candidate and repeat both panel lanes. Dispatch no v2 code before the final
  Accepted/index candidate has both required signoffs.

#### W1 - Delivery, test, panel, and stack tooling

Implement Rust `xtask` commands for:

- Layer-1 manifest validation, parallel local execution, and workflow rendering;
- Git Town parent-graph validation and ordinary GitHub PR integration;
- immutable wave snapshots;
- test evidence import and command/result hashing;
- panel JSON validation;
- wave sealing;
- retarget/rebase preflight;
- fail-closed merge-train status and merge eligibility.

Retire replaced Python/Bash orchestration. Update AGENTS process rules, the old
three-unit invariant, tests/AGENTS reviewer/validator rules, Make targets,
Layer-1 manifests/workflows, stacked-PR base triggering, PR dependency/evidence
fields without AI attribution, and Nix dev tooling for Git Town, pinned
nightly `cargo-udeps`, and `cargo-semver-checks`.

AGENTS must require the integrator to open or update the wave PR immediately
after the immutable candidate is committed and focused preflight passes, before
starting the final long local/host validation and full panel. GitHub CI, final
local validation, and panel inspection then run concurrently against the same
tree hash. A PR may show those lanes as pending; it cannot merge until every
required CI/local/host result and the 10/10 panel are present in the seal.

#### W2 - Workspace and canonical contracts

W2 first runs the blocking dependency/API-fit spike for exact
`ttrpc = "=0.9.0"`, `protobuf = "=3.7.2"`,
`ttrpc-codegen = "=0.6.0"`, and `protobuf-codegen = "=3.7.2"`. It must prove
generated asynchronous server/client compatibility and nonblocking Tokio
behavior before service schemas freeze. W2 fails closed if the spike does not
pass; no contract slice starts around an incompatible or blocking binding.

Parallel contract slices:

1. unify every host and guest crate under one workspace/lockfile and 2.0.0
   metadata;
2. add typed human names, exact SHA-256 IDs, cross-language vectors, collision
   checks, generated endpoint rows, and path proofs;
3. refactor `d2b-contracts` into no-default feature families;
4. define ComponentSession preface, Noise profiles/vectors, errors, records,
   attachment descriptors, limits, purposes, roles, and capabilities;
5. define every `.v2` protobuf service and generated client/server binding;
6. define provider descriptors, contexts, plans, handles, observations, health,
   registries, and all eleven types;
7. define complete storage, sync, state, and audit contracts;
8. bump all contract generations and regenerate schema/reference artifacts.

W2 owns the v2 reference and schema documentation for IDs, roles, endpoints,
ComponentSession, services, providers, storage, synchronization, state, and
audit. It commits generated JSON/Markdown and drift coverage together with each
contract change, including the cross-language ID vectors, socket proof, and
evaluation budget. No old type is re-exported.

#### W3 - Session, provider, state, and client foundations

- `d2b-session-unix`: stream/seqpacket, drain-correct `AsyncFd` readiness,
  directional local identity with parent-prearmed `SO_PASSCRED`, ancillary
  capacity derived from negotiated hard maxima, packet atomicity, truncation
  scavenging, and exact bounded FD validation and credit cleanup.
- `d2b-session`: `snow`, vectors, record protection, fragmentation, replay,
  keepalive, close, authenticated absolute expiry, per-hop relative ttrpc
  timeout, request-ID cancellation, named streams, scheduler, and metrics.
- `d2b-provider`: traits, typed registries, optional capabilities, operation
  context, RPC proxy, generation lifecycle, and shutdown.
- `d2b-provider-toolkit`: agent server, registration, redaction, fixtures, and
  conformance.
- `d2b-state`: atomic JSON, quarantine, path-safe I/O, generations, locks, and
  audit segment helpers.
- `d2b-client`: resolver, transport/session setup, generated clients, typed
  errors, idempotency, attachments, and named streams.

#### W4 - First-party providers

Use one independently reviewable slice per provider axis or implementation.
Wrap real behavior, run common conformance, and advertise only live capability:

- Cloud Hypervisor, qemu-media, systemd-user, and ACA runtime providers;
- Unix/vsock/CH-vsock/Relay transport providers;
- NixOS/Linux substrate providers;
- Secret Service/Entra/managed-identity credential adapters in correct owners;
- real Wayland display provider;
- local-realm network provider;
- local storage provider;
- host-mediated TPM/USBIP/FIDO/GPU/video device provider;
- PipeWire/vhost-user audio provider;
- bounded local observability provider;
- Azure VM runtime/infrastructure fake-SDK scaffold with the infrastructure
  create/power/adopt/bootstrap/delete and runtime workload-deploy/exec/inspect
  authority split compile-checked and production capability denied.

W4 also owns the initial private provider registry artifact, bundle v12
integrity, startup registry activation, generation-triggered daemon restart,
local runtime and observability mappings, and mapped Cloud Hypervisor/qemu-media
lifecycle routing. Those surfaces are frozen inputs to W5-W9.

#### W5 - Core control-plane service migration

W5 starts from the post-W4 shared root in parallel with W6, W7, and W9. The
shared root freezes daemon/realm/guest/provider-agent/broker/allocator service
DTOs and generated bindings. W5's local prep owns only its file map. W5 owns
core service dispatch and CLI/client migration; it does not edit
provider-registry Nix mappings.

Parallel slices:

- local/remote daemon service and CLI;
- realm controller/router/bootstrap/shortcut service;
- guest bootstrap, vTPM identity, exec, shell, file, and stream service;
- provider-agent service;
- privileged broker service and typed FD attachments;
- local-root allocator and child-broker lease service, including dedicated
  child UIDs/namespaces, pre-bound child public/broker listeners, direct
  controller/broker cgroup-leaf placement, typed parent spawn, pidfd handoff,
  zero initial-namespace capabilities, and FD-only delegation.

W5 owns reference documentation for the daemon, realm, guest, provider-agent,
broker, allocator, and client service APIs plus operator how-to changes needed
by those service migrations. No slice keeps an old handshake or fallback.

#### W6 - User, desktop, device, and helper migration

W6 starts from the post-W4 shared root in parallel with W5, W7, and W9. It
consumes W4's user-agent placement, credential leases, provider contracts,
transport contracts, and observability result contract unchanged. Its local
prep owns only its file map and edge-local composition.

Parallel slices:

- `d2b-userd`, direct no-shell `gnome-keyring-daemon --unlock` TTY spike,
  post-unlock `oo7` Secret Service interaction, TPM2/systemd export, and the
  no-unlock pre-reset deletion/revocation completion statement;
- renamed systemd-user runtime agent and shell supervisor;
- clipboard control, picker, bridge, and readiness;
- notify/wlcontrol event and action flow with bounded projections;
- Wayland proxy control and FD paths;
- security-key report streams, trusted intent, cancellation, and command policy;
- activation, TTY, and retained one-shot helper channels;
- any additional d2b-owned IPC found by the W0 inventory rule.

W6 owns the user-secret, unattended-credential, systemd-user, shell, clipboard,
desktop, Wayland, FIDO/security-key, activation, and TTY reference and how-to
documentation. Each migrated boundary updates or removes its old instructions
in the same slice.

#### W7 - Realm-native host configuration and resources

W7 starts from the post-W4 shared root in parallel with W5, W6, and W9. Bundle
v12, the private provider registry, local runtime mappings, local observability
mappings, and the frozen allocator API are existing contracts. W7 owns
Nix/process/resource emission against that allocator API and extends the
provider registry with transport, substrate, display, network, storage, device,
and audio mappings. It does not introduce another registry, generation model,
runtime route, or allocator service protocol.

Implement:

- realm/workload/provider options only;
- the local-root PID1 units plus generated home/dev/work child process and
  ordering records, with no child realm `.service` or `.socket` units;
- per-realm users, internal cgroup groups, generated socket ownership rows,
  confined child brokers, identities, namespaces, state, and audits;
- generated local-root allocator listener/lease requests plus child
  controller/broker process, cgroup, namespace, and ownership records; W5 alone
  owns allocator service dispatch, runtime listener creation, typed spawn,
  pidfd supervision/adoption, and lease execution;
- process-free per-realm cgroup roots with `controller/`, `broker/`, and
  `workloads/` children, direct `CLONE_INTO_CGROUP` placement, and write
  delegation that ends at the realm root;
- recursion-safe normalized role/provider/storage index;
- short-ID paths and complete bundle artifacts;
- broker-only directory/ACL repair;
- realm-scoped network, storage, device, and audio resources;
- canonical-target-only desktop metadata;
- deletion of `d2b.vms`, `d2b.envs`, gateway, relay, and placeholder wiring.

W7 owns the v2 Nix option reference, generated option/schema artifacts,
filesystem and endpoint layout, realm/process/allocator/storage/network
reference, and realm configuration how-tos. Option examples must include the
mandatory destructive-cutover acknowledgement and no old option path.

#### W8 - Integrated behavior and current-feature parity

At content freeze, Git Town linearizes W5 -> W6 -> W7. W8 is a child of that
W7 head, uses W7 as its PR base, and records the ordered four-node chain in its
delivery manifest. W4 already owns mapped Cloud Hypervisor/qemu-media lifecycle
and local observability. W8 verifies their parity, removes the temporary
unmapped direct-dispatch fallback during destructive integration, and
concentrates new routing on the remaining providers and services.

Wire providers and services through real lifecycle flows:

- local VM and qemu-media parity, adoption, and final fallback removal;
- systemd-user exec and persistent shell;
- realm routing and policy;
- ACA and Azure Relay behavior formerly in gateway crates;
- work realm interactive provider executor;
- shared-fabric shortcut authorization, revocation, and audit;
- display, clipboard, audio, storage, network, USBIP, FIDO, GPU, and video;
- daemon/controller/broker restart continuation;
- bounded status, remediation, observability, and audit export.

#### W9 - Toolkit and sibling cutover

W9 starts when `W4-F` freezes the client/provider contracts and proceeds in
parallel with W5-W7. It does not wait for those waves unless a specific sibling
feature consumes one of their new service APIs.

Across private sibling branches:

- rename and migrate `d2b-client-toolkit`;
- create the `d2b-provider-toolkit` distribution;
- migrate `d2b-wlterm`;
- migrate `d2b-wlcontrol`;
- migrate the WeezTerm seam;
- update flake follows, exact source rewrites, package names, share paths,
  generated artifacts, and release automation;
- run `cargo-semver-checks` against the intentional 2.0 major-break baseline.

W9 owns in-tree client/provider toolkit reference material and the corresponding
README, API, release, and migration documentation in every sibling repository.
Toolkit docs consume the generated v2 contracts and contain no copied wire
definition.

#### W10 - V1 purge and reset tooling

- Complete every checked deletion item.
- Run pinned `cargo-udeps`.
- Enable production dead-code denial with generated-code-only exceptions.
- Implement the exact reset-mode-only digest-confirmed factory reset command and
  dedicated no-d2b-service boot target.
- Implement the pre-reset user intent, confirmation command, root-owned receipt
  authentication, exact configured-user receipt gate, export-revocation
  operation, and post-receipt barrier. Reset mode has no Secret Service client
  or unlock path.
- Keep the released reset implementation generic: it may delete only generated
  current d2b anchor classes and may not contain a literal legacy outlier name.
- Add source policy proving old identifiers, paths, crates, services, options,
  protocols, and branches do not remain.
- Complete a Diataxis-wide rewrite/removal audit of `README.md`, examples, and
  every current `docs/reference`, `docs/how-to`, and `docs/explanation` page.
  Rewrite live v2 guidance, delete stale pages rather than preserving migration
  stubs, and retain v1 text only in historical ADRs and released changelogs.
- Validate persistent reset boot selection, crash-to-reset behavior, unit/socket
  absence, lock/inhibitor handling, cgroup/process/session/fd/mount quiescence,
  under-lock digest recomputation, held-dirfd recursive deletion, symlink and
  mount-crossing refusal, fatal mount-point `EBUSY`, bounded `ENOTEMPTY`
  re-enumeration, optional topology-change alarms, fsync, and final-v2 boot
  selection without parsing old state.

#### W11 - Private physical-host cutover and hardening

Before implementation PRs merge:

1. build the complete integrated private branch, final v2 boot generation, and
   dedicated v2 reset boot generation;
2. update a private `/etc/nixos` branch to v2 realm-only configuration without
   selecting final v2 yet;
3. if required, embed exactly one audited data-only outlier-root manifest in the
   private reset closure; no other source or artifact may contain those names;
4. while every configured owning session and keyring is available, generate
   the reset intent, run the confirmed `d2b-userd` preparation for every UID,
   delete d2b-owned keyring items, revoke scoped TPM exports, verify the exact
   authenticated receipt set, terminate the prepared user sessions/keyrings,
   and close the post-receipt barrier;
5. install, mark successful, and persistently select the reset generation as
   the boot default only after step 4, then reboot into `d2b-reset.target`;
6. prove every v1/v2 operational d2b system/user service and socket unit absent
   or masked (with only the inert reset target active), acquire the reset
   lock/inhibitor, verify every receipt without accessing or unlocking Secret
   Service, prove cgroup/process/session/fd/mount quiescence, dry-run, and apply
   the recomputed digest-confirmed reset through the bounded fd-relative
   traversal with no backup;
7. allow the reset binary to select final v2 and reboot only after deletion,
   receipt revalidation, fsync, and absence proofs succeed; any interruption
   reboots to reset mode;
8. initialize and verify GNOME Keyring login unlock, `d2b-userd`, and scoped
   TPM2 exports;
9. start the local-root units and verify that its allocator pre-binds and
   parent-spawns separate home, dev, and work controller/broker processes into
   their declared cgroup leaves with pidfd supervision and no child PID1 units;
10. recreate `personal-dev` in `dev` and the interactive work provider executor
   in `work`;
11. validate complete local desktop parity and available cloud parity;
12. remove the private outlier manifest, its closure/GC roots, and every literal
   legacy outlier name from production artifacts, then prove the final
   integrated production code/configuration/schema/test set and release closure
   contain only generic current-anchor reset logic;
13. repair forward on the private stack until validation and a final full panel
   seal the manifest-free integrated tree.

#### W12 - Merge, release, and final host pin

- Merge sealed PR stacks through GitHub, root to leaf.
- Retarget/restack and rerun required CI after each merge.
- Reject a merge whose resulting tree differs from its sealed tree.
- Reject any production source/configuration/schema/test tree or release
  closure containing the private W11 reset manifest or a literal legacy outlier
  root; historical ADR prose is not executable reset inventory.
- Release d2b 2.0.0.
- Release both toolkit distributions and sibling migrations in dependency
  order.
- Before any release is published, own and complete the 2.0 release notes and
  destructive migration guide. They name
  `d2b.acceptDestructiveV2Cutover = true`, the no-backup reset procedure, generic
  unknown-option behavior, absence of `mkRemovedOptionModule` tombstones, and
  repair-forward posture.
- Move `/etc/nixos` from the private branch to final merged tags/releases.
- Switch once more without resetting state and rerun focused host smoke.
- Commit the host lock/config update separately.

#### W13 - Evidence-driven delivery streamlining

W13 is the final ADR 0045 wave. It does not invent a new architecture or reopen
accepted product decisions. It turns delivery friction observed in W4-W12 into
reviewed, tested improvements to the plan compiler, validation tooling, stack
workflow, and agent/worktree hygiene. New friction remains eligible only when
it cites a concrete wave, command or tool path, observed failure mode, and
measurable cost. Add entries throughout delivery; do not wait until W13 to
reconstruct them from memory.

The initial W13 backlog is grounded in these observed failures:

| Observed in | Friction | Required W13 outcome |
| --- | --- | --- |
| W4 | Superseded slice targets consumed about 70 GiB, the retained integrated cache reached about 117 GiB, and reviewer/slice worktrees left additional targets and untracked artifacts. | Give every component and immutable validator an external, bounded target allocation with ownership metadata, quota checks, automatic cleanup, stale-process reaping, and a final no-leak assertion. |
| W4 | A cached `xtask` embedded `CARGO_MANIFEST_DIR` from a removed slice worktree, so integrated drift generation failed only after all component tests passed. | Bind generated-tool provenance to the current checkout, reject binaries rooted in another or missing worktree, and rebuild automatically before generation. |
| W4 | Root corrections required repeated manual Git Town propagation through the shared root, W5/W6/W7, and W9; duplicated parent commits and recurring changelog conflicts were resolved by hand. | Add a graph-aware propagation command that reports invalidated descendants, drops byte-identical parent duplicates, queues conflict owners, verifies every PR base/head, and records ready/launched/blocked counts. |
| W4 | Fresh command processes lost working-directory state, causing correctly chosen Cargo tests to run from the repository root and fail before testing code. | Make plan commands carry an explicit repository-relative cwd and have the runner reject a missing manifest before consuming a validation slot. |
| W4 | Panel corrections arrived across multiple rounds and forced repeated candidate invalidation, worktree integration, focused preflight, restacking, resnapshotting, receipt generation, and cleanup. | Add one correction-round coordinator that builds the file-overlap graph, dispatches every ready component, invalidates old evidence, tracks finding-to-commit closure, and prepares the replacement candidate without manual bookkeeping. |
| W4 | Panel signing and attestation depended on tools such as OpenSSL and Git Town being present through ad-hoc shell entry. | Make the delivery wrapper expose and self-check every required executable before a snapshot, panel import, seal, or merge begins. |
| W5-W7 | Layer-1 and Rust scripts stopped at the first failing target, hiding dozens of independent stale fixtures and policy failures behind serial reruns. | Add a diagnostic mode that runs independent Layer-1 shards, Rust feature matrices, fixture contracts, CLI contracts, and both-system evals to completion, then emits one deduplicated root-failure report. |
| W6-W7 | Validation output was retained in a framed binary payload that required manual offset decoding, while `validation-run` returned `"status":"ok"` even when the recorded validation result was `"failed"`. | Add a canonical evidence output decoder/tail command and make the command envelope surface the validation result unambiguously, with an explicit require-pass exit mode for interactive use. |
| W6-W7 | A GitHub check could remain `in_progress` while carrying a success conclusion, and exact-head status required manual API inspection and a history-only retry commit. | Add exact-head CI diagnosis that distinguishes pending, contradictory, stale-head, and terminal states and supplies the permitted recovery path without content churn. |
| W7 | The monolithic Lix flake evaluator repeatedly lost a `git+file` source path while every bounded shard passed. | Make the bounded manifest-driven shard runner the default local flake path and retain the monolith only as an explicit diagnostic compatibility mode. |
| W7 | Nix-unit pins, the migration ledger, schema prose, manifest baselines, and contract fixtures drifted independently and were discovered only after earlier shards passed. | Add a preflight freshness command that regenerates into an isolated tree and reports every stale generated or pinned artifact together before long compilation starts. |
| W7 | Plan ledgers were internally consistent while naming deleted files, missing tests, stale source paths, orphan modules, dead selection predicates, and a valid network guest module with no composition call site. | Extend plan policy to verify file existence/deletion, actual diff completeness, test selector existence, source-path reachability, module-graph imports or explicit retirement, and resource/process rows reaching final composition. |
| W7 | Aggregate diagnostics created an 86 GiB temporary Cargo target before cleanup. | Reuse content-addressed external build caches across diagnostic lanes under a hard size budget; never multiply full workspace targets per feature lane. |
| W7 | Implementation agents occasionally produced feature commits without the required trailing wave tag despite explicit prompts. | Enforce commit-subject traceability mechanically before integration or push instead of relying on prompt compliance. |
| W7 | Read-only panel reviewers detached both the integration and primary worktrees, left untracked review artifacts, and created build targets despite explicit no-edit/no-test instructions. | Run reviewers against disposable read-only checkouts, deny write/build tools by construction, and mechanically restore branch attachment, cleanliness, and target budgets after every review lane. |
| W7 | The parent-trusted ownership check required an ambient `git-town` executable and then rejected `main` after the immediately preceding wave had merged, even though the delivery snapshot correctly recognized the merged predecessor. | Ship the ownership checker with its complete tool closure and teach parent authority resolution to accept the exact landed predecessor recorded by the manifest and ordinary PR state. |
| W7 | A panel role initially reported child-realm PID1 units that did not exist; only a manual full-file challenge caused it to retract and inspect the actual sandbox matrix. | Require every actionable review recommendation to carry verified file/line evidence and run a read-only contradiction check against the exact candidate before the result can become an attestation. |
| W7 | The canonical `xtask` merge command sealed and verified W7's candidate but refused to merge because GitHub's PR API exposed only the expected head SHA with no executable auto-merge match, forcing `gh pr merge --match-head-commit` plus manual parent/tree re-verification outside the sealed tool chain. | Ship a supported atomic, fail-closed merge path in the delivery wrapper that performs the exact-head match and merge itself instead of requiring an unassisted `gh` fallback. |
| W9 | wlterm CI cloned toolkit revision `800c` while Cargo, the flake input, and cross-repo coordination state all pinned `3d6`, and the mismatch was discovered only as a compile failure. | Add a preflight cross-surface pin-consistency check comparing the toolkit revision in Cargo.lock, flake.lock/inputs, and coordination state, failing before a stale or mismatched pin reaches CI. |
| W8 | The local shared Cargo registry cache held a partially unpacked `cc` crate; the resulting build failure required manual targeted cache purge and refetch of just that crate. | Add a registry cache integrity check/repair step that detects partially unpacked or corrupt entries and performs a targeted purge/refetch automatically. |
| W8 | W8 could not launch because `wave_policy` hardcoded exactly the w5/w6/w7 wave set and its trusted-parent design correctly prevented a W8 candidate from authorizing its own policy extension, requiring a separate predecessor policy-root change before any component could start. | Make the plan compiler generate and land the next wave's policy entry plus manifest scaffold as a predecessor dependency-readiness precondition, so the trusted parent already recognizes the new wave before its components are scheduled. |

W13 must deliver:

1. a machine-readable, append-only friction ledger and `xtask` commands to add,
   list, validate, assign, and close entries against concrete commits/tests;
2. the plan and reachability checks named above, including a mandatory final
   streamline wave in every new ADR-scale delivery graph;
3. aggregate no-fail-fast diagnostics with concise root-cause grouping and
   exact rerun commands;
4. hermetic cwd/tool/target management with disk budgets and automatic cleanup;
5. delivery evidence and exact-head CI diagnostics that cannot look successful
   when the underlying validation failed;
6. graph-aware correction/restack coordination and mechanical commit-tag checks;
7. regression fixtures reproducing the W4-W7 failures in this table; and
8. before/after measurements for elapsed operator steps, repeated compilation,
   peak disk, hidden failures per rerun, and manual stack/evidence operations.

W13 uses the normal immutable candidate, required validation, exact-head CI,
ten-role panel, seal, and GitHub merge process. The ADR is not delivery-complete
until W13 either closes each accepted friction entry with a tested improvement
or records a concrete external blocker and owner. W13 cannot waive or defer a
functional or security defect from an earlier wave.

### Worktrees, stacks, and immutable evidence

All implementation uses dedicated worktrees and private feature branches.
Git Town owns stack topology, proposing ordinary PRs, synchronization,
restacking, and retargeting. Changes
merge through GitHub only, root to leaf; no implementation branch pushes or
merges directly to `main`.

Seal, panel, and test/validation evidence is never part of the Git content tree
being reviewed. It lives only in external session state, GitHub artifacts, and
GitHub checks, keyed by the exact integrated Git tree hash and carrying SHA-256
digests for each evidence payload. It is never copied into a branch, generated
repository artifact, release source archive, or commit merely to make the seal
self-describing. A PR may link to external evidence; it may not embed that
evidence as reviewed-tree content.

#### External evidence storage and W0 bootstrap template

W0 precedes `xtask wave snapshot`, so it uses a checked bootstrap evidence
procedure rather than pretending the tool already exists. The exact local
storage template is:

```text
$HOME/.copilot/session-state/<session-id>/evidence/w0/<git-tree-hash>/
  candidate.json
  candidate.sha256
  validation/<validation-id>.json
  validation/<validation-id>.sha256
  panel/<role>.json
  panel/<role>.sha256
  seal.json
  seal.sha256
```

The directory name is the full output of `git rev-parse <head>^{tree}`. An
optional GitHub mirror uses artifact name
`d2b-w0-<proposed|accepted-index>-<git-tree-hash>` and checks that report the
same tree hash and payload SHA-256. Neither location is inside a repository
worktree.

For each Proposed and Accepted/index candidate, `candidate.json` records:

- repository identity, base commit, head commit, and `git rev-parse
  <head>^{tree}` content-tree hash;
- the complete repository set, which is exactly the d2b repository for W0;
- the name and digest of every changed/generated artifact;
- the exact dependency-file diff and contract/index diff, including an explicit
  empty result;
- each validation ID, exact command digest, exit status, output-artifact digest,
  and external locator already produced for that tree.

`candidate.sha256` contains the digest of `candidate.json`; the candidate does
not contain its own digest and does not list panel records that do not yet exist.
Each panel record binds the tree hash and `candidate.sha256`. Once all ten panel
records exist, `seal.json` lists the candidate digest and the sorted validation
and panel record digests; `seal.sha256` hashes the seal. The seal does not contain
its own digest. This one-way construction has no self-inclusion cycle.

The integrator independently checks that the worktree is clean at the recorded
head, re-derives the tree and artifact digests, and supplies those values to
every reviewer. Reviewers inspect but do not run validation. The Proposed
candidate, panel records, and seal are retained externally when constructing the
Accepted/index candidate; they are evidence for the first required panel, not a
seal for the second tree. The Accepted/index candidate has a different tree-hash
directory and a fresh candidate, panel set, and seal. Its status/index/header
edits are already in that immutable candidate, while the resulting panel and
seal remain external. Thus the second panel can approve the exact prospective
Accepted tree without requiring evidence to be committed into the tree it
approves. Any correction creates a new tree-hash directory and invalidates the
old candidate rather than mutating or reusing it.

For each wave, `xtask wave snapshot` records:

- base commit;
- ordered repository/PR/branch dependency graph;
- each head commit;
- integrated tree hash;
- required validation matrix;
- repository set for cross-repository waves.

The W1 tool writes those records and the later validation/panel/seal payloads to
the same class of external tree-hash-addressed storage or to GitHub artifacts and
checks; its output is not a commit candidate.

After focused preflight passes, the integrator opens or updates the PR for the
exact snapshot before starting the final lanes. Three activities then proceed
concurrently against that exact immutable tree:

1. GitHub runs the PR's required CI shards;
2. validators run the required local/host tests and import hash-bound
   command/results;
3. the full panel inspects the tree, plan, and live evidence status but runs no
   tests.

Every wave uses all ten roles:

```text
software
test
nixos
networking
security
rust
product
docs
observability
kernel
```

Every role uses the provider and model version bound by the candidate's panel
request and returns `signoff: true` with an empty recommendation list. `xtask
wave seal` succeeds only with all required validation evidence plus 10/10 panel
signoff bound to the same tree hash, heads, base, dependency graph, generated
artifacts, dependency diff and contract fingerprints, and repository set.

Any content change, including source, documentation, generated output,
dependency metadata, contract/index content, or repository-set membership,
invalidates both validation and panel lanes. Reviewers do not run tests.
Validators do not substitute for review. A later speculative wave may continue
while an earlier wave is under review.

A history-only rebase or PR retarget may reuse panel evidence only after W1
`xtask` proves that the integrated content tree, every generated artifact,
dependency diff and contract fingerprint, and complete repository set are
byte-identical to the sealed candidate. Commit IDs and graph edges may differ;
content may not. Required CI is rerun on the new Git history. Before that proof
tool exists, including during W0, a rebase or retarget reruns the applicable
panel. Any content difference, however small, invalidates both lanes and is not
eligible for this exception.

## Validation and acceptance

All coverage follows `tests/AGENTS.md`: Rust unit/integration/contract/policy
tests and nix-unit cases first, then container, host, hardware, and live tests
where the behavior requires them. No ad hoc top-level shell gate is added.

### Required test-tier mapping

Layer-1 proofs remain mandatory even when a higher tier covers the integrated
behavior: type 1 nix-unit cases own option values/eval rejection (including the
destructive acknowledgement), types 2-3 own pure state machines, framing,
scheduling, reset planning, and hermetic binary/session behavior, type 4 owns
rendered Nix/Rust contract parity, type 5 owns complete-inventory and no-v1
source policy, and type 6 owns realized example/flake evaluation. Generated
artifacts use the existing drift gate; performance uses the existing
`performance-budgets.sh` gate. No higher tier substitutes for these proofs.

The behavior that genuinely needs a booted or physical system maps exactly as
follows:

| `tests/AGENTS.md` tier | Mandatory d2b 2.0 coverage |
| --- | --- |
| Type 10, runNixOSTest (`tests/host-integration/*.nix`, `make test-host-integration`) | Disposable reset-generation boot and reboot/crash-to-reset simulations; final-v2 selection only after success; systemd unit/socket absence and persistent boot selection; systemd credential materialization and keyring/unlock process lifecycle; local-root plus per-realm controller/broker boot; child-broker user/mount/network namespaces, UID/GID maps, capability sets, FD delegation, and cgroup placement. Reset tests delete only disposable VM disks and synthetic d2b anchors. |
| Type 11, live host (`tests/integration/live/*.sh`, `D2B_LIVE=1`, manual only) | The actual destructive physical-host cutover, real reset reboot/lock/inhibitor/quiescence/deletion/final-boot flow, real desktop login/keyring continuity, and credentialed ACA/Azure Relay or other cloud flow. These checks never run in CI and require the private-host procedure and explicit operator control. |
| Type 12, hardware (`tests/host-integration/hardware/*.sh`, manual only) | Physical TPM sealing/unsealing and reset behavior, YubiKey/USBIP/FIDO mediation, and real GPU/video/device passthrough. Software swtpm and synthetic device-policy cases stay in lower tiers; only actual hardware claims close here. |

Type 9 containers are used only for a provider/helper that must prove foreign
non-Nix userland behavior; they are not a shortcut around type 10 boot/systemd
coverage. Every Layer-2 test records why Layer 1 cannot prove the same kernel,
boot, cloud, or device property.

### Contracts and workspace

- reproducible protobuf/schema generation;
- exact `.v2` service IDs and schema fingerprints;
- exact authenticated-expiry/cancellation metadata and no epoch-valued
  `timeout_nano`;
- exact ttrpc/rust-protobuf runtime and codegen pins, plus the pre-schema
  async-generated API-fit/nonblocking Tokio spike;
- resolver 2 feature-family dependency direction, no-default portable builds,
  exact isolated guest check/build feature sets, and feature-graph exclusion of
  host Nix/socket and cloud SDK families from framework guest artifacts;
- one workspace, one lockfile, and version 2.0.0 inheritance, with no nested
  guest or broker workspace;
- independent Nix/Rust printable-ASCII grammar, pure-Nix lowercase RFC 4648
  base32 helper, exact encoded/digest/ID vectors, malformed/non-canonical grammar
  rejection, global `ProviderId` uniqueness, and the pinned 4,096-ID evaluation
  time/RSS budget;
- generated socket contract bidirectional completeness and byte-length/NUL
  headroom proof for every separately expanded row and literal leaf;
- `cargo-udeps` clean;
- source policy rejects old crates, types, versions, and service IDs.

### ComponentSession

- fixed `snow` vectors for NN, KK, and IKpsk2;
- transcript downgrade, cross-purpose, role, schema, limit, and channel-binding
  rejection;
- pathname acceptor `SO_PEERCRED`, responder socket/unit/inode provenance,
  parent-prearmed two-ended `SO_PASSCRED` before fork/handoff, immediate-child
  first-packet `SCM_CREDENTIALS`, launch-binding/pidfd/cgroup/executable
  mismatch, and established-session transfer rejection;
- replay, truncation, duplicate/reordered fragment, nonce exhaustion, malformed
  preface, and over-limit rejection;
- checked Noise handshake, handshake-tag, record-tag, fragment, and envelope
  overhead at every allocation boundary, including adversarial boundary
  property tests that prove rejection without panic;
- 30-second skew and 15-minute lifetime boundaries, wall-clock jumps,
  ingress/queue/dispatch expiry, per-hop capped relative ttrpc timeout, and
  request-ID cancellation before and during dispatch;
- generated handlers awaiting borrowed `async_trait` futures only in the
  session-owned request task, with compile coverage that every detached path
  captures an owned `Arc` provider/context or explicit owned operation object;
- deterministic scheduler coverage for priority, fairness, backpressure,
  cancellation, and no lock across await;
- paused-time boundary tests at one unit below, exactly at, and one unit above
  every scheduling, queue, handshake, reconnect, stale-health, and recovery
  threshold; status transition/hysteresis and closed remediation tests;
- low-cardinality metric-schema tests proving that only transport, purpose,
  channel class, Noise class, locality, provider type, health state, and closed
  result/reason labels exist and that IDs never become labels;
- Unix readiness burst tests prove read and write drivers drain through
  `EAGAIN`/`EWOULDBLOCK` or preserve the guard/cached readiness and explicitly
  continue, with no one-packet-per-wakeup stall;
- Unix packet tests derive ancillary capacity at every negotiated boundary and
  cover `MSG_TRUNC`, `MSG_CTRUNC`, partial descriptor delivery, malformed or
  unknown control/attachment data, missing/extra FD, duplicate object, wrong
  type/access/purpose, absent `CLOEXEC`, and disconnect. Stable before/after
  process-FD counts prove zero leaks and close-once ownership on every fatal
  path;
- exact packet/request/operation/session/process/host FD-credit boundaries,
  `RLIMIT_NOFILE` reduction, 64-FD emergency reserve, rejection, and close-once
  cleanup under cancel/disconnect races;
- fuzz/property tests for preface, handshake, record, fragment, attachment, and
  ttrpc-boundary parsers.

### Providers

- base descriptor and health;
- exact `healthy`/`degraded`/`unavailable`/`failed` provider-health transitions,
  poll/deadline/staleness thresholds, agent-disconnect behavior, recovery
  requirements, and no automatic retry from an invariant failure;
- object-safe trait construction for every primary and optional interface;
- global provider-ID collision, duplicate
  (`ProviderType`, `ImplementationId`) factory rejection, and multiple
  configured-instance acceptance;
- fallible `Result`-returning factory/capability/instance registration,
  typed duplicate and configuration errors, transaction abort, and no
  configuration-triggered panic;
- descriptor/registry claim parity and per-instance duplicate rejection;
- generated placement coverage for every initial implementation ID;
- operation-scope widening rejection;
- idempotency, deadline, cancellation, and retry classes;
- adoption identity/configuration/generation mismatch;
- secret-canary flow only through the co-located private non-serializable
  interface and absence from process/session/persistence, Debug, errors, logs,
  traces, audit, metrics, and state boundaries;
- real conformance for every advertised implementation/capability;
- call-graph proof that production enters the registry;
- Azure VM infrastructure-only create/power/adopt/bootstrap/delete and
  runtime-only bound-handle workload deploy/exec/inspect negative authority
  tests;
- Azure VM scaffold never registers or advertises live support.

### Realm, Nix, storage, and audit

- controller ownership and cycle rules, including only the narrow
  independently-running-controller exception;
- local-root allocator lease isolation, pre-bound child public/broker listener
  provenance, typed parent spawn, pidfd supervision, and restart adoption;
- local-root broker as the only PID1 socket-activated broker, with no child
  realm `.socket`/`.service` units or `SD_LISTEN_FDS` path;
- child-broker dedicated uid/user/mount/network namespace setup, zero
  initial-namespace capability sets, closed UID/GID mappings, FD-only delegated
  access, global-path/`setns` denial, and local-root-only host mutation;
- child-controller realm-key generation/unseal only after direct placement,
  using its key-state dirfd and TPM resource-manager FD with no plaintext key
  buffer/file in the allocator or either broker;
- process-free realm/workload cgroup interiors, direct controller/broker and
  runner leaf placement, controller/broker-group write only at the realm common
  ancestor and workload subtree, same-realm-only moves, and `EACCES` outside
  that delegated root;
- per-realm user, socket, broker, key, state, audit, cgroup, and network
  separation;
- omission/false rejection and true acceptance for
  `d2b.acceptDestructiveV2Cutover`, with no reset artifact generated on
  rejection and no old-option tombstone;
- independent Nix/Rust SHA-256 vectors, collision rejection, and the generated
  complete per-endpoint Unix socket-length proof;
- no path built from a raw human/provider/device value;
- complete storage/sync/endpoint inventory, fixed local-root/user
  socket-activation exception, allocator-owned child listeners, broker-only
  other dynamic creation, and no daemon unlink/rebind path;
- restart adoption before cleanup and typed quarantine;
- atomic JSON crash and simulated power-loss consistency at every write phase,
  proving that success is impossible before the post-rename parent-directory
  fsync and that recovery observes exactly the prior or complete new document;
- segment chain, checkpoint, retention, and gap behavior;
- no old options, aliases, tombstones, or three-unit invariant.

### Service integration

- client to local and remote daemon;
- realm bootstrap, enrollment, routing, and shortcut;
- guest one-time bootstrap followed by static-key reconnect bound to the
  parent-launched runtime, exact transport endpoint, controller/workload
  generation, and fresh boot nonce;
- copied file-backed swtpm/guest state rejection and
  `realm-controller-host-v2` denial without accepted non-copyable attestation;
- broker typed FD operations;
- provider-agent proxy parity;
- direct no-shell `gnome-keyring-daemon --unlock` TTY behavior, graphical Secret
  Service prompt, post-unlock `oo7`, and fail-closed backend-spike mismatch;
- pre-reset `d2b-userd` fixed-attribute deletion and export revocation through
  an authenticated owning session, with no secret-value read and no unlock
  attempt;
- systemd credential materialization under read-only
  `$CREDENTIALS_DIRECTORY`, TPM/host/user/credential-name binding, and
  PID1/DAC/namespace assignment isolation;
- systemd-user runtime and shell;
- clipboard control, picker, bridge, and transfer FD;
- notify/wlcontrol and Wayland control;
- FIDO report stream, approval, cancellation, queue/lease bounds, and
  destructive-command denial;
- activation/TTY one-shot channels;
- mixed old bytes fail the v2 preface before semantic dispatch, without a v1
  parser or diagnosis branch.

### Release and sibling consumers

- `cargo-semver-checks` records the intentional 2.0 API break;
- toolkit and sibling flakes build from exact released source artifacts;
- every canonical SDK crate remains `publish = false`, `xtask` rejects
  `cargo publish`, and fingerprinted GitHub/flake path dependencies work
  without a crates.io publication path;
- no old repository, crate, package, share, or protocol path remains;
- final production source/configuration/schema/test set and release closure
  contain no private W11 reset manifest or literal legacy outlier root, and
  reset code deletes only generic current anchors;
- generated docs and examples use only canonical realm targets.

### Reset and seal safety

- reset and final v2 closures are built before cutover;
- module and reset-generation evaluation require the explicit v2 destructive
  acknowledgement, while setting it alone performs no mutation;
- before reset selection, the canonical intent and confirmed per-user
  preparation delete d2b-owned Secret Service items, revoke scoped exports,
  authenticate exactly one digest/nonce/inventory-bound receipt per configured
  UID, terminate the prepared login/user-manager/keyring processes, and close
  the no-new-write barrier;
- unavailable/locked user sessions, missing/duplicate/stale receipts, failed
  export removal, or a barrier failure prevent reset selection, and retry uses
  a fresh nonce and complete receipt set;
- reset generation is the persistent boot default, not a one-shot, and every
  interruption/reboot before completion returns to reset mode rather than v1;
- all operational d2b system/user service and socket units are absent or masked
  in reset mode, with only the inert reset target active;
- reset mode verifies root-owned receipts but has no D-Bus, `oo7`, user-child,
  Secret Service access, or keyring-unlock path;
- reset lock and shutdown inhibitor fail closed;
- cgroup, process, ComponentSession, open-fd, cwd/root/executable/map, lease, and
  mount quiescence checks cover every declared root;
- plan digest is recomputed under lock after anchored root dirfds are opened and
  held;
- deletion is recursive and fd-relative; directory descent uses the required
  `openat2` resolve flags, `unlinkat` never follows a final symlink, a mount
  point fails `EBUSY`, and a third `ENOTEMPTY` after two complete
  re-enumerations fails closed;
- `/proc/self/mountinfo` and `STATX_MNT_ID_UNIQUE`, when available, are tested
  only as defense-in-depth topology alarms and never as an atomic
  `fstatat`-then-unlink identity proof;
- final v2 is selected and booted only after fsync and absence proofs;
- W0 has a 10/10 Proposed-tree panel and a separate 10/10 Accepted/index-tree
  panel using the external tree-hash-addressed bootstrap evidence template;
- history-only reuse requires `xtask` proof of identical integrated tree,
  generated artifacts, dependency diff/contracts, and repository set, followed
  by rerun CI; any content change invalidates both lanes.

### Required gates before merge

- `make check`;
- `make test-integration`;
- `make test-host-integration`;
- `make test-hardware`;
- applicable live-host tests;
- sibling repository equivalents;
- all GitHub CI contexts on stacked and final-main bases;
- the complete physical-host matrix;
- a 10/10 immutable-tree panel seal for every wave, plus both full W0 panels.

### Physical-host acceptance matrix

- Niri login unlock and shell/graphical `d2b secret unlock`;
- direct TTY backend and graphical unlock plus allowlisted systemd credential
  consumption from `$CREDENTIALS_DIRECTORY` without whole-keyring unlock;
- pre-reset deletion/revocation with one authenticated receipt per configured
  UID, fail-closed locked/unavailable-user cases, post-receipt session/keyring
  termination and write denial, and reset-mode proof that no Secret Service
  access or unlock is attempted;
- local-root daemon/broker health and pidfd-supervised home/dev/work
  controller/broker health, with no child realm PID1 units;
- child brokers have zero initial-namespace capabilities/global path access and
  operate only on delegated namespace/dirfd/device leases;
- controller, broker, and workload processes begin in their declared cgroup
  leaves; same-realm movement succeeds where authorized, while writes or moves
  through `d2b.slice`, root, and peer realms fail;
- `personal-dev` lifecycle, restart, exec, persistent shell, graphics, Wayland,
  audio, clipboard, storage, network, USBIP, FIDO, and daemon restart adoption;
- work interactive provider executor;
- provider/session/realm/workload status and repair-forward remediation;
- ACA and Azure Relay live flow when credentials are available;
- security-key per-ceremony intent and closed denial behavior;
- no old units, sockets, directories, state, CLI verbs, protocol, provider
  names, toolkit paths, or aliases;
- reset-generation reboot/crash continuation, final-v2 boot only after reset
  proof, and ordinary post-reboot continuation;
- no private reset manifest or literal legacy outlier root in production
  artifacts/closure, and no file-backed-swtpm controller-host capability;
- final merged release behaves identically to the sealed private integrated
  tree.

## Key risks and controls

| Risk | Required control |
| --- | --- |
| A byte-only session abstraction loses seqpacket and FD security. | Packet/attachment capability is first-class; broker migration waits for kernel-level truncation, type, flag, policy, and disconnect coverage. |
| An `AsyncFd` driver consumes one packet per readiness wake and strands queued control or cancellation work. | Drain `recvmsg`/`sendmsg` under the readiness guard through would-block, or preserve cached readiness and explicitly reschedule when a fairness budget yields. |
| A Unix initiator mistakes socket-activation peer credentials for responder identity. | Keep NN, but use directional evidence: acceptor-observed `SO_PEERCRED`, initiator-verified path/inode/unit provenance, and parent-prearmed two-ended `SO_PASSCRED` plus first-packet credentials and a launch binding for inherited endpoints. Never transfer an established session FD. |
| An absolute epoch is encoded as a relative ttrpc timeout or expires while queued. | Authenticate issue/expiry separately, enforce 30-second skew and 15-minute lifetime, intersect wall/monotonic remaining time at every queue boundary, set only capped relative `timeout_nano`, and cancel by request ID. |
| Ancillary truncation or an FD flood exhausts `RLIMIT_NOFILE`, leaks a partially delivered descriptor, or prevents cancellation/cleanup. | Size control space from the negotiated hard maximum, collect every installed descriptor before honoring truncation or parse errors, enforce packet/request/operation/session/process/host credits, keep 64 emergency control slots, and close/release each ownership slot exactly once. |
| Multiple realm brokers race over global host resources or a child escapes its lease. | Local-root alone retains initial-namespace/global authority; each child has a dedicated uid and user/mount/network namespaces, zero initial-namespace capabilities, and only allocator-issued dirfds/FDs/leases. |
| Reset mode cannot delete a locked user's Secret Service items and cutover deadlocks. | Delete fixed-attribute items and revoke scoped exports in a mandatory confirmed pre-reset phase while every configured owning session is unlocked; authenticate digest-bound receipts, close the write barrier, and refuse to select reset mode without the exact set. Reset mode never accesses or unlocks Secret Service. |
| PID1 and the allocator can independently start the same child broker with incompatible namespace/listener ownership. | Socket-activate only the local-root broker. The allocator pre-binds child listeners and parent-spawns separate child controllers/brokers through typed operations; local-root `d2bd` supervises their pidfds, and no child realm PID1 unit exists. |
| A child controller cannot move a runner from a sibling systemd cgroup or gains write to all of `d2b.slice`. | Create a process-free per-realm root with controller, broker, and workload children; place controller/broker directly with `CLONE_INTO_CGROUP`; delegate the common realm ancestor/workload subtree only; and birth runners in role leaves or move them only within that realm root. |
| Recursive reset deletion treats a pre-unlink stat or mount ID as an atomic object-identity guarantee. | Hold the exclusive reset lock and quiescence proof, walk held dirfds with `openat2` beneath/no-symlink/no-xdev resolution, unlink only relative to the parent dirfd, fail mount points with `EBUSY`, and bound `ENOTEMPTY` re-enumeration. Mount observations are alarms only. |
| The no-backup reset crashes after deletion and boots v1. | Build both closures first, persistently select a no-d2b-service reset generation, hold a reset lock and shutdown inhibitor through bounded fd-relative deletion, and select final v2 only after fsync/absence proof. Every interruption reboots reset mode. |
| Legacy outlier names become a permanent compatibility inventory. | Permit one audited data-only manifest only in the private W11 reset closure, never parse records, and remove the manifest, closure roots, and literals before the final seal/release. |
| File-backed swtpm is treated as non-cloneable identity. | State that host root/whole-state copy can clone it, bind guest admission to the parent runtime/transport/generations/fresh boot nonce, and deny controller-host capability absent accepted non-copyable attestation. |
| An unlocked Secret Service is ambient to same-uid processes. | Document the same-uid limit, separate `d2b-userd` from the runtime agent, use a direct stdin-only keyring unlock backend, export only scoped systemd credentials, and never forward Secret Service into guests. |
| TPM sealing is mistaken for cryptographic systemd-unit binding. | Bind TPM/host/user/credential-name only; rely on PID1 assignment, DAC, mount namespace, and service sandbox for unit isolation, and test the actual `$CREDENTIALS_DIRECTORY` path. |
| Truncated IDs collide or a socket path overflows. | Use the exact domain-separated SHA-256 encoding, independent Nix/Rust vectors and collision rejection, a generated proof for every literal endpoint row, and pre-side-effect bundle validation. |
| Provider traits become facades while production bypasses them. | Require call-graph evidence and real conformance for every advertised implementation and capability. |
| Sibling repositories ship a different protocol/client artifact. | Use exact release source fingerprints, no duplicate clients, cross-repository immutable snapshots, and dependency-ordered releases. |
| Azure VM authority is duplicated or schema is mistaken for working support. | Infrastructure alone owns VM create/power/adopt/bootstrap/delete; runtime is limited to workload actions through a bound handle. Never register/advertise either scaffold and test both negative boundaries. |
| A rebase preserves prose but changes generated/dependency/repository content. | Reuse review only after `xtask` proves identical integrated tree, generated artifacts, dependency diff/contracts, and repository set; rerun CI, and invalidate both lanes for any content change. |

## Security invariants

1. Realm, workload, provider, guest, and local principal identity is
   authenticated before state lookup, credential resolution, provisioning, FD
   acceptance, or semantic dispatch.
2. Local NN identity evidence is directional: acceptors use kernel
   `SO_PEERCRED`; initiators use trusted endpoint provenance; inherited
   endpoints have `SO_PASSCRED` enabled on both ends by the parent before
   fork/handoff and also use first-packet credentials and parent launch
   binding. An established session endpoint is not transferable.
3. Transport evidence establishes reachability only. It never grants local
   daemon role, broker authority, or realm identity.
4. Authenticated absolute expiry is independent of ttrpc relative timeout.
   Remaining time is rechecked at every queue/dispatch boundary, and
   request-ID cancellation aborts outstanding dispatch and cleanup. Unix
   readiness handling drains to would-block or preserves readiness for explicit
   continuation, so control and cancellation never depend on a second edge.
5. Attachment count, kind, object type, flags, operation binding, and
   packet/request/operation/session/process/host credits are exact. Emergency
   control FD headroom cannot be consumed by attachments. Every descriptor
   installed by a receive is collected before truncation/parser failure returns
   and is closed or transferred exactly once.
6. A provider cannot widen an already-authorized operation or invoke the broker
   directly. `ProviderId` is globally unique; factory uniqueness is only by
   (`ProviderType`, `ImplementationId`), allowing distinct configured
   instances.
7. Cloud/provider secret material remains in its owning co-located process,
   may cross only the private non-serializable module interface, and crosses no
   ComponentSession, persistence, or telemetry boundary.
8. A remote peer never receives a local broker protocol or raw host-mutation
   capability.
9. Each host-local realm has a separate controller/broker/state/audit/resource
   boundary. Child controllers and brokers are separate parent-spawned,
   pidfd-supervised processes, not PID1 units. Child brokers have no
   initial-namespace capability or global path access and operate only on
   allocator-approved namespace/dirfd/device leases.
10. Local-root alone performs host-global mutation. Allocation of narrow leases
    does not become peer-realm lifecycle authorization. A child
    controller/broker group can write only its realm common cgroup ancestor and
    workload subtree, never `d2b.slice`, root, or a peer realm; processes are
    born in destination leaves or move only within that realm root.
11. Fixed local-root/user listeners are created only by their declared
    system/user socket unit. The local-root allocator creates and pre-binds
    child public/broker listeners; the owning broker creates other dynamic
    listeners. Every listener is handed to its declared owner as an FD, and a
    daemon never recreates an allocator/broker-owned path.
12. File-backed swtpm is cloneable by host root/whole-state copy. Guest admission
    additionally binds the parent runtime, transport endpoint, controller and
    workload generation, fresh boot nonce, and one-authoritative-generation
    rule; file-backed swtpm cannot host a realm controller.
13. TPM/systemd credential cryptography binds TPM, host, user, and credential
    name. PID1 assignment, DAC, mount namespace, and service sandbox, not TPM
    cryptography, isolate the receiving unit.
14. The selected service, schema, roles, purpose, Noise profile, limits,
    transport evidence, and attachment policy are authenticated before API
    dispatch.
15. Private keys, PSKs, credentials, proofs, endpoints, paths, commands,
    payloads, and user-provided labels do not enter telemetry. Metric labels use
    only closed low-cardinality classes.
16. A normal v2 daemon restart is a continuation event. Before factory reset,
    each configured user's live `d2b-userd` deletes fixed-attribute d2b items,
    revokes scoped exports, and yields an authenticated intent-bound receipt.
    Factory reset runs only in the persistently selected no-d2b-service reset
    generation, never accesses or unlocks Secret Service, refuses without the
    exact receipt set, and selects final v2 only after locked, inhibited,
    digest-confirmed, quiescent fd-relative deletion that refuses symlink
    traversal and mount crossing, and absence proof. Mount observations are
    defense in depth, not unlink identity authority.
17. Final v2 production code/configuration/schema/test and release artifacts
    contain no legacy reset inventory. One private W11 data-only outlier
    manifest is permitted only until the manifest-free final seal; historical
    ADR prose is not executable inventory.
18. Module and reset-generation evaluation require the explicit v2 destructive
    acknowledgement. It cannot trigger deletion by itself and does not register
    or recognize an old option.
19. No v1 compatibility recommendation can weaken these invariants.

## Definition of done

The d2b 2.0 program is complete only when:

- ADR 0045 is Accepted only after a 10/10 panel on the Proposed tree and a
  second 10/10 panel on the final Accepted/index tree using external
  tree-hash-addressed W0 bootstrap evidence that is never committed into either
  reviewed tree;
- every d2b-owned live IPC boundary uses ComponentSession v2 with directional
  local identity, authenticated absolute expiry, per-hop relative ttrpc
  timeout, request-ID cancellation, parent-prearmed inherited credentials,
  readiness-correct I/O, bounded aggregate FD credits, and close-once
  truncation cleanup;
- ComponentSession scheduling, queues, handshakes, reconnect, and provider
  health enforce the typed operational objectives, hard failure thresholds,
  transitions, remediation, and low-cardinality telemetry defined here;
- every production lifecycle/component path enters the appropriate typed
  provider registry;
- all eleven provider axes wrap current functionality, every initial
  implementation has explicit placement, globally unique provider instances
  and pair-scoped factories behave as specified, and all pass conformance;
- Azure VM remains clearly non-production scaffold with infrastructure-only VM
  authority and runtime-only bound-handle workload authority;
- every host-local realm runs a separate controller/broker boundary with
  local-root resource allocation and pre-bound listeners; every child
  controller and broker is a separate parent-spawned, pidfd-supervised
  non-PID1 process born in its declared cgroup leaf, while child brokers have
  dedicated uid/user/mount/network namespaces, no initial-namespace
  capabilities, and FD/lease-only delegated access;
- every dynamic path uses independently derived SHA-256 short
  realm/workload/provider/role IDs over the printable-ASCII grammar with
  pure-Nix/Rust vector parity and the evaluation budget; every expanded socket
  row has a bidirectionally complete generated length/ownership proof; and only
  the declared manager/broker creates it;
- d2b-userd, GNOME Keyring, TPM2 scoped exports, and host-local realm key
  lifecycle work from Niri, direct TTY backend, graphical prompt, and
  unattended `$CREDENTIALS_DIRECTORY` delivery with the documented
  cryptographic, local-root PID1, and parent-spawned child isolation split, and
  pre-reset preparation produces one authenticated absence/revocation receipt
  for every configured UID without reset-mode Secret Service access;
- guest identity rejects copied/stale runtime state through exact
  runtime/transport/generation/boot-nonce binding, and file-backed swtpm never
  advertises controller-host capability;
- every v2 host configuration explicitly sets
  `d2b.acceptDestructiveV2Cutover = true`, omission/false fails before reset
  generation, and no `mkRemovedOptionModule` or other old-option tombstone
  exists;
- all d2b 1.x code, options, protocols, crates, state, paths, tests, aliases,
  tombstones, wrappers, and current shipped docs are gone, except historical
  ADR and released-changelog records that cannot act as input;
- `d2b-client-toolkit`, `d2b-provider-toolkit`, `d2b-wlterm`,
  `d2b-wlcontrol`, and the WeezTerm seam are v2-only;
- W2/W5/W6/W7/W9 documentation ownership is complete, W10 has closed the full
  current Diataxis rewrite/removal audit, and W12 has published the destructive
  release and migration guidance before release;
- every wave has all required tests and a 10/10 immutable-tree panel seal, and
  history-only evidence reuse occurs only after identical-content `xtask`
  proof plus rerun CI; seal, panel, and validation artifacts remain external to
  the sealed Git tree;
- the physical host has completed the confirmed live-user deletion/revocation
  phase and exact receipt gate, booted the persistent reset generation,
  locked/inhibited/quiescent, mount-refusing, recursively fd-relative reset with
  no Secret Service access and no backup, booted final v2 only after receipt,
  fsync, and absence proof, and been validated on the private integrated
  branch;
- the final sealed production source/configuration/schema/test set and release
  closure contain only generic current-anchor reset logic and no private
  outlier manifest/literals, and the host is pinned to merged d2b 2.0.0
  releases.

## Consequences

### Positive

- One session/authentication/limit/attachment model replaces many bespoke
  protocol stacks.
- Provider authority becomes explicit and production behavior is testable
  through common registries and conformance.
- Realm boundaries become literal process, privilege, identity, state, audit,
  and resource boundaries.
- Dynamic paths are short, deterministic, collision-checked, and independent of
  user-controlled names.
- User interaction, unattended credentials, provider credentials, guest keys,
  and realm keys have distinct owners.
- Client and provider toolkit distributions consume canonical in-tree source
  instead of duplicating contracts.

### Negative

- Every d2b workload disk, TPM identity, key, token, cache, audit record, and
  persistent session is destroyed once.
- There is no rollback to d2b 1.x after the physical-host reset.
- Cutover cannot select reset mode until every configured user has an available
  unlocked session and completes the destructive receipt-producing preparation.
- The first implementation touches nearly every crate, Nix surface, IPC path,
  sibling repository, and host integration.
- Multiple host-local realm brokers require a carefully verified local-root
  allocator, parent-spawn path, pidfd supervisor, and cgroup delegation.
- Universal encrypted local sessions and exact FD validation add implementation
  complexity and memory/latency overhead.
- Provider and toolkit APIs intentionally break at 2.0.

## Rejected alternatives

### Retain a compatibility release

Rejected. A parser, tombstone, alias, state reader, wrapper, or mixed protocol
would preserve the old authority boundary in production and make deletion
unverifiable.

### Use `mkRemovedOptionModule` for friendlier old-option errors

Rejected. `lib.mkRemovedOptionModule` is a tombstone: it keeps the old option
path executable specifically so it can recognize and diagnose that path. D2b
2.0 deliberately leaves old paths undeclared, accepts the generic Nix
unknown-option error, and puts all cutover guidance in release and migration
documentation.

### Preserve disks, TPM state, or keys offline

Rejected. The old state would remain a rollback dependency and an untested
import temptation. The accepted recovery posture is build first, reset once,
and repair forward.

### Keep broker and helper protocols specialized

Rejected. Locality and seqpacket FD semantics are transport capabilities, not a
reason to retain unauthenticated or differently framed d2b protocols.

### Use one host-global broker with realm tags

Rejected. A realm tag inside one privileged process does not provide the
process, socket, state, audit, and delegated-resource boundaries selected by
ADR 0043.

### Put every provider in-process

Rejected. Third-party and credential-owning implementations would enter the
controller TCB and collapse workload credential boundaries.

### Put every provider out-of-process

Rejected. It adds process/session overhead to trusted local adapters without
creating a useful security boundary. The same trait and conformance contract
supports both placements.

### Return credentials through the provider RPC

Rejected. Secret-bearing RPC would make controllers, session buffers, logs,
crash state, and unrelated adapters part of the credential boundary. Only a
co-located opaque lease is supported.

### Use an embedded state database

Rejected. The state is small, authority-partitioned, and benefits from
human-inspectable bounded records. One shared atomic JSON library plus segmented
audit is sufficient and keeps repair ownership explicit.

### Advertise Azure VM as experimental production support

Rejected. Schema and fake-SDK conformance do not prove live provisioning,
credential, adoption, restart, or remote-controller safety. The initial v2
capability remains unadvertised.
