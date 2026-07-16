# AGENTS.md

Operating manual for AI coding agents (Copilot CLI, GitHub Copilot,
Cursor, …) and human contributors working on **`vicondoa/d2b`
itself**. If you are *consuming* d2b in your own NixOS host
config, start at [README.md](./README.md) instead — this file is for
people changing the framework.

## What this is

d2b is an opinionated NixOS desktop microVM framework that
owns its microVM substrate end-to-end. The accepted d2b 2.0 control
plane is realm-local: PID1 owns only the fixed local-root systemd
endpoint set, while the local-root allocator pre-binds listeners and
parent-spawns a separate controller and broker process for every child
host-local realm. Those child processes are pidfd-supervised, not PID1
units. Each realm controller supervises its workload DAGs; there are no
per-workload systemd templates and no legacy bash CLI. See
[ADR 0045](./docs/adr/0045-provider-and-transport-framework.md) for the
binding architectural decision.

What the framework provides: per-env isolated networks with an
auto-declared NAT/DHCP "net VM", a per-VM `/nix/store` hardlink farm,
toggleable per-VM components (graphics, TPM, USBIP, audio), and the
versioned bundle/manifest contract that grounds the broker dispatcher.
See [README.md](./README.md) and
[`docs/explanation/design.md`](./docs/explanation/design.md) for the
full picture and threat model.

## Repo layout

```
.
├── README.md                       <- consumer-facing entry point
├── AGENTS.md                       <- this file
├── SECURITY.md                     <- disclosure policy + threat-model summary
├── CHANGELOG.md                    <- Keep a Changelog, grouped under `## [Unreleased]`
├── LICENSE                         <- Apache-2.0
├── flake.nix                       <- public surface: nixosModules / templates / checks
├── flake.lock
├── .github/workflows/              <- CI-only checks that stay out of root `flake.checks`
├── nixos-modules/                  <- THE framework
│   ├── default.nix                 <- aggregator imported as nixosModules.default
│   ├── options.nix / options-*.nix <- option schema (site / envs / vms)
│   ├── assertions.nix              <- eval-time invariants (CIDR overlap, platform gate, …)
│   ├── lib.nix                     <- internal helpers (subnetIp, mkMac, …)
│   ├── index.nix                   <- normalized internal VM/env/component index
│   ├── host.nix / host-*.nix       <- host activation, users, polkit, sidecars, keys, audit
│   ├── network.nix / net.nix       <- per-env bridges + auto-declared net VM
│   ├── store.nix                   <- per-VM /nix/store hardlink farm
│   ├── manifest.nix                <- JSON manifest emitter (versioned contract)
│   └── components/                 <- toggleable per-VM features
│       ├── graphics.nix            <- virtio-gpu + Wayland cross-domain
│       ├── tpm.nix                 <- per-VM swtpm 2.0
│       ├── usbip.nix               <- YubiKey USBIP passthrough
│       ├── home-manager.nix        <- HM-as-NixOS-module inside the guest
│       └── audio/{guest,host}.nix  <- vhost-user-sound + PipeWire mediation
├── pkgs/                           <- patched cloud-hypervisor / crosvm / vhost-device-sound
├── packages/                       <- Rust workspace; pinned rust-toolchain.toml
│   ├── d2b-core/              <- shared bundle DTOs, typed errors, privilege metadata
│   ├── d2b-host/              <- host-side lifecycle primitives (argv, hardlink farm, ifnames)
│   ├── d2b-contracts/          <- public + private wire contracts
│   ├── d2b-session-unix/       <- async Unix transport, credentials, and descriptor validation
│   ├── d2b-session/            <- authenticated ComponentSession runtime
│   ├── d2b-provider/           <- provider traits, registries, lifecycle, and RPC
│   ├── d2b-provider-toolkit/   <- provider-agent adapter and conformance kit
│   ├── d2b-state/              <- atomic state, locks, leases, and audit segments
│   ├── d2b-client/             <- typed async resolver, session, and service clients
│   ├── d2b/                   <- rust-native CLI
│   ├── d2bd/                  <- unprivileged public daemon / supervisor
│   ├── d2b-priv-broker/       <- privileged broker for audited host mutations
│   ├── d2b-guest-shell-runner/ <- static guest helper for persistent shell feasibility
│   └── xtask/                     <- schema/docs codegen + Layer-1/delivery workflows
├── tests/                          <- see "Test layout" below
├── examples/                       <- minimal / graphics-workstation / multi-env / with-entra-id
├── templates/default/              <- `nix flake init -t github:vicondoa/d2b`
└── docs/                           <- Diataxis tree (explanation / how-to / reference)
                                       plus `docs/adr/` architecture decision records
```

New behaviour belongs in a focused file under `nixos-modules/`
(or `nixos-modules/components/` for per-VM toggles), wired in
from `nixos-modules/default.nix`. Don't fatten existing files.

## Build & validate

Use the top-level `Makefile` targets. The shell scripts under `tests/`
are implementation details unless a target or `tests/AGENTS.md` tells
you to run one directly.

```bash
# Sub-60s syntax + shellcheck loop for docs/shell-only edits.
make check-tier0

# Layer-1 local development umbrella: lint, Rust, proofs, flake,
# drift, and policy gates. CI runs these sub-targets in parallel.
make test-unit

# Focused Layer-1 shards when iterating on one surface.
make test-lint
make test-rust
make test-proofs
make test-flake
make test-drift
make test-policy

# PR-equivalent Layer-1 gate. Uses tests/layer1-jobs.json to run
# independent make test-* shards locally with bounded parallelism.
make check

# Legacy/full-static monolithic gate retained for explicit use.
make check-static

# Local Layer 1 + container integration.
make test
```

Before opening or updating a wave PR, commit the candidate and run the
smallest focused preflight that can catch an obviously broken tree.
After that preflight passes, open or update the PR, then snapshot the
exact open PR/stack state. Run the final local/host validation matrix
only after the PR is open, concurrently with GitHub CI and the
end-of-wave panel:

```bash
make test-integration       # Layer 2 container tests; needs podman
make test-host-integration  # runNixOSTest VM checks; NixOS + KVM host
```

`make test-host-integration` is x86_64-linux only and may fall back to
slow TCG if `/dev/kvm` is absent. Hardware and live-host tests remain
explicit manual tiers (`make test-hardware` or `D2B_LIVE=1 bash
tests/integration/live/<name>.sh`) and require a host with the matching
devices or deployed d2b state. These final validator lanes may be
pending when the PR opens, but every required result must be present in
the tree-bound seal before merge.

For where tests live, when to add or retire each kind of test, and
which pins/ledgers to update, read [`tests/AGENTS.md`](./tests/AGENTS.md).
[`tests/README.md`](./tests/README.md) is the human quick-start for the
same test model.

## Development workflow

## Changelog & Releases

Every PR that changes code **must** update `CHANGELOG.md`. The CI gate
enforces this.

### Format

[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Add entries under
`## [Unreleased]`. When ready to release, rename the section to
`## [X.Y.Z] - YYYY-MM-DD`.

### Auto-release

Merging to `main` with a new version header in `CHANGELOG.md` triggers:
1. Auto-creation of git tag `vX.Y.Z`
2. Build of all host binaries (`d2bd`, `d2b`, `d2b-priv-broker`,
   `d2b-wayland-proxy`, `d2b-activation-helper`)
3. GitHub Release with changelog notes + binary tarballs + `SHA256SUMS`

Consumers can fetch pre-built binaries from the release instead of
building from source.

### Versioning

Follow semver. The version in `CHANGELOG.md` is the single source of truth.

### Worktrees for parallel agents

Use one dedicated worktree and private feature branch per independently
reviewable scope. Worktrees isolate concurrent changes; they are not a
license to merge implementation branches directly into local `main`.

```bash
# From the primary clone, one worktree per concurrent scope:
git worktree add -b phase-<name> ../d2b-<name> main
```

Each scope owner commits in that worktree and hands the branch to the
integrator. For ADR-scale work, completion means the branch is represented in
the Git Town parent graph and its ordinary GitHub PR state is current; it does
**not** mean merging the branch into the primary clone.

#### Finish-of-work invariant: GitHub merge, then fast-forward

ADR-scale changes merge only through GitHub PRs, root to leaf. Never locally
merge or octopus-merge an implementation branch into `main` before GitHub has
merged its PR. After the GitHub merge, restore the clean primary clone to
`main` and fast-forward it:

```bash
git switch main
git pull --ff-only
```

Keep unrelated operator work in its own worktree rather than stashing it around
an integration merge. Audit sibling worktrees for abandoned or superseded
branches and flag them; do not silently drop them.

### Speculative stacked-wave workflow

ADR 0045 must be Accepted before d2b 2.0 implementation begins. Once it is
Accepted, later dependent waves may proceed speculatively as soon as their exact
stable contract dependencies exist; they need not wait for an earlier wave's
validation or panel lanes to finish. Speculation never grants merge eligibility.
Every wave must independently restack or rebase onto its landed dependencies,
snapshot the resulting tree, complete all required validation, receive the full
panel seal, and pass merge-eligibility checks before it merges.

This is a positive launch requirement, not merely permission. When a wave enters
its immutable final lanes, the integrator MUST query the dependency graph and
launch every newly ready speculative wave in the same coordination cycle.
Leaving a ready wave idle requires a concrete contract, file-ownership, disk, or
tooling blocker recorded in the plan and task database; avoiding possible merge
conflicts or keeping one agent's context warm is not a blocker.

Use this shape:

1. Open one private branch/worktree per independently reviewable slice and use
   Git Town for stack topology, graph inspection, proposing, synchronization,
   restacking, and retargeting. Do not replace it with ad-hoc `gh`/Git stack
   scripts. Propose noninteractively with
   `git town propose --stack --non-interactive --no-browser`.
2. Stack only real dependencies. Independent branches target `main`; dependent
   branches target the exact contract PR they consume.
3. Use the hardened Rust `xtask` commands for snapshots, validation run/import,
   panel request/attestation, sealing/verification, history proof, retarget
   preflight, eligibility, and merge. The machine-readable command index is:

   ```bash
   cd packages
   cargo xtask delivery wave help
   ```

   Do not omit the `delivery` namespace or infer options from a generic
   `--help`; the command index above is authoritative.
4. After focused preflight, open or update the PR and then create the immutable
   snapshot of that exact open PR/stack state. GitHub CI, validators, and
   reviewers work concurrently on the snapshotted tree. Pending lanes permit
   an open PR, never a merge.
5. Keep validation, panel, and seal payloads external to Git. PR bodies contain
   only dependency, base/head/tree, `candidate_id`/`content_id`, and check-status
   summaries and may link to external evidence. Never commit or embed raw
   evidence or panel output, and never include AI, assistant, tool, or model
   metadata.
6. If any content changes—including generated output, dependency metadata,
   contracts, or repository membership—both validation and panel results are
   invalid. A history-only rebase or retarget may reuse panel results only after
   the canonical `xtask` proof establishes byte-identical content and required
   CI reruns on the new history.
7. The integrator owns CI follow-up, restacking, conflict resolution, root-to-leaf
   merge order, branch deletion, and post-merge fast-forward of the primary
   clone.

### Screenshot and visual artifact hygiene

Screenshots used as validation evidence live in external evidence storage and
may be linked from the PR; do not commit or embed them in the reviewed tree or
PR body. Screenshots that are legitimate shipped documentation assets must also
be redacted before use:

- Remove or black out all secrets, credentials, API keys, and tokens visible in
  any terminal, browser, or UI window.
- Remove or replace personally identifiable information (PII): real names, email
  addresses, employee ids, user ids, and similar identifiers.
- Replace or black out sensitive command output: stack traces with host paths,
  raw error messages with internal node names or realm principals, clipboard
  content, and any window title or app metadata that names a real person or
  organization.
- Use generic placeholder names (e.g., `alice`, `corp-vm`, `work`) matching the
  conventions in the Don'ts section above.

Panel reviewers may inspect redacted screenshots through their external
evidence locators. If a screenshot cannot be adequately redacted without losing
the information being demonstrated, use a text description or a synthetic
reproduction instead.

### Local host validation after updating d2b

When a host configuration switches to a new d2b checkout (for
example a local `path:/home/paydro/projects/d2b` input), the host
switch updates `/etc/d2b/*` and the system packages and may restart
`d2bd`. That daemon restart is a continuation event: VMs must stay
running, protected by `KillMode=process`, and the restarted daemon
re-adopts their runner pidfds. Before runtime validation, make sure the
notify-ready daemon is active on the updated generation:

```bash
sudo systemctl restart d2bd.service
```

Then restart affected VMs with the normal lifecycle commands (on this
host, prefer `d2b down <vm> --apply` followed by
`d2b up <vm> --apply`; `d2b switch <vm>` is not reliable here).

#### Stable-contract preparation

When parallel scopes share DTOs, schemas, or other contracts, put those shared
contracts in the root PR of the Git Town graph. Dependent scopes may start
once that committed contract is stable enough to consume, but they remain
speculative until it lands. Contract changes require dependent branches to
restack and lose any prior validation or panel seal; never land a prep commit
directly on local `main`.

The post-W4 shared root is the exclusive owner of the frozen ComponentSession
service DTOs/bindings, allocator and child-realm spawn wire, workspace dependency
table and `Cargo.lock`, delivery tooling, and `delivery/shared-contracts.json`.
Its policy defines disjoint implementation prefixes: W5 owns the core
daemon/CLI/client, realm, guest, provider-agent, broker, host, and allocator
crates; W6 owns userd, systemd-user/shell, clipboard, notify/wlcontrol, Wayland,
security-key, activation, TTY, and one-shot helper crates; W7 owns
`nixos-modules/`, `pkgs/`, `examples/`, and `templates/` emission. Every wave
lists the other two sets as foreign and fails closed on those paths. W7 extends
the existing `provider-registry-v2` family through its narrow protected-path
exceptions; no wave creates a second registry.

W5, W6, and W7 each edit only their authority at
`delivery/manifests/w<N>.json`; `delivery/manifest.json` remains the unchanged W4
authority. Ownership verification is itself parent-authoritative. Run the target
from a clean trusted worktree checked out at the candidate's exact immediate
Git Town parent commit, never from the candidate:

```bash
make -C "$TRUSTED_PARENT_ROOT" wave-policy-check \
  CANDIDATE_ROOT="$WAVE_WORKTREE"
```

The trusted checker derives the candidate branch and wave from the canonical
`adr0045-w5`, `adr0045-w6`, or `adr0045-w7` branch stem. It obtains the immediate
parent from Git Town, discovers the branch's unique open ordinary GitHub PR, and
requires the policy-pinned repository and exact local/PR base and head OIDs. It
walks every wave ancestor to the shared root and verifies each Git Town edge
against that branch's unique ordinary PR. It accepts no caller-selected wave or
base, rejects `HEAD` as its own base, and requires its own clean source worktree
to be that exact immediate parent. Before linearization every wave may use the
shared root; afterward only the complete W5 -> W6 -> W7 chain is valid. The
check rejects another wave's implementation, another wave's
manifest, the workspace lock/shared dependency table, frozen cross-wave
contracts, or shared delivery/policy tooling. A newly required shared contract
returns to the shared-root PR and all consumers restack.

#### Anti-serialization invariant

Serial ownership ends at the smallest coherent shared contract boundary. A
shared DTO/schema/lockfile change may require one prep commit; it does **not**
justify serializing every implementation or later wave that consumes it.

Apply these rules to every plan and finding round:

1. Build a file-overlap graph for all ready scopes. Each connected component may
   be internally ordered, but distinct components MUST run concurrently in
   separate worktrees. Partition by actual files/contracts, not by a desire to
   avoid all future conflict.
2. After a prep commit freezes a shared API, immediately dispatch all
   dependency-ready components and waves. Use sibling stacked PRs over the
   shared root. If overlapping follow-ups require order, create a short
   micro-stack for only those files while unrelated components continue.
3. A persistent agent owns one coherent component. Do not repeatedly expand one
   long-lived agent into an umbrella owner for unrelated provider axes,
   protocols, Nix modules, daemon routing, policy, documentation, and later
   review rounds merely because it retains context. Reawaken it only for the
   same component; start new agents/worktrees for independent components.
4. The integrator owns shared prep, merge/conflict resolution, lockfile
   reconciliation, generated artifacts, delivery authority, and cross-component
   tests. The integrator is not the default implementation sink for work that
   can be assigned to an independent component.
5. At final-stage entry and after every review round, record the ready component
   count, launched component count, and any blocked component with its exact
   blocker. A launch count below the ready count without recorded blockers is a
   process failure and must be corrected before more serial implementation.
6. Resource limits constrain heavy validation, not implementation parallelism.
   Use the plan's bounded heavy-gate semaphore for full builds/tests; do not keep
   code work idle solely because another worktree is validating.
7. Remove a slice worktree and its real Cargo target immediately after its
   commits are integrated. Retain only active integration worktrees, so
   parallelism does not become abandoned-worktree disk pressure.

Exception: a security-sensitive cross-cutting invariant may stay serial only
when the plan names the exact files, invariant, and unblock commit. Dispatch all
downstream components as soon as that commit lands; the exception cannot expand
silently into the rest of the wave.

#### Heavy local validation gate

`cargo xtask heavy-gate -- <command> [args...]` is the sole host-wide heavy-lane
semaphore. It owns two per-UID OFD-locked slots in
`${XDG_RUNTIME_DIR}/d2b-heavy-gates`, or
`${TMPDIR:-/tmp}/d2b-heavy-gates-$UID` when no runtime directory is available.
The selected parent is pinned with a CLOEXEC directory FD and must be either an
invoking-UID-owned non-symlink directory without group/other write or a
root-owned sticky world-writable directory. The per-UID `0700` directory is
created and opened relative to that FD with `mkdirat`/`openat` and
`O_NOFOLLOW`. The persistent `slot-0.lock` and `slot-1.lock` files must be
regular, invoking-UID-owned, single-link `0600` files opened relative to the
pinned gate FD with `O_RDWR|O_CREAT|O_CLOEXEC|O_NOFOLLOW`. Parent, directory,
and slot name-to-inode bindings are revalidated so rename or replacement cannot
silently split the lock namespace.

Acquisition tries slot 0 then slot 1 with nonblocking OFD write locks every
250 ms for at most 30 minutes. Unsupported OFD locking, unsafe metadata, or
timeout fails closed; there is no `flock` fallback. The parent retains its
original CLOEXEC descriptor. The child receives a duplicate of the same locked
open-file description at the numeric FD named by `D2B_HEAVY_GATE_FD`, with
CLOEXEC cleared. Before spawning, the wrapper replaces inherited
`SIGCHLD=SIG_IGN` or `SA_NOCLDWAIT` state with a caught handler; `exec` resets
that handler to default, while the wrapper can retain a waitable leader. The
child runs in its own process group; the wrapper forwards termination signals,
normally waits for the complete group, then closes its original FD.
`/proc/<pid>/stat` is parsed as bytes. Any pidfd wait, process-table, namespace,
or process-group observation failure retains the parent permit while the
wrapper kills the group and keeps its exited leader unreaped as the PID/PGID
identity anchor while inspecting and terminating descendants. The leader is
reaped only after the final group signal and membership check, so a reused PGID
can never be targeted. Five consecutive process-table failures trigger one
final anchored `SIGKILL` followed by leader reap and failure; descendants keep
their inherited locked FD until that kill takes effect. Thus a wrapper crash
does not release a permit while its child hierarchy lives. Slot files are never
unlinked during acquisition.

Use `make heavy-check`, `make heavy-test-integration`,
`make heavy-test-host-integration`, `make heavy-test-hardware`,
`make heavy-cargo-test`, or `make heavy-flake-check` for the expensive local
lanes. Existing focused targets and CI do not consume a slot, and the ungated
targets retain their prior semantics.

### Edit → commit → validate

Commit before running `static.sh` / the smoke evals. Two reasons:

1. Untracked files are invisible to `nix flake check` (and to any
   eval that follows the same code path). Forgetting to `git add` a
   new module is the #1 "why doesn't my change apply?" pitfall.
2. Consumer hosts that vendor d2b tend to ship auto-backup
   tooling that catch-all-commits any dirty tree. That's a
   consumer-side concern, but the habit of committing-then-building
   is the right one to carry into framework work too.

For plan-driven work, green tests are not enough to merge. See
[Wave validation and panel seal](#wave-validation-and-panel-seal).
Speculative dependent work may advance on stable contracts, but each wave must
independently rebase, validate, and seal its final tree before merge.

### "Existing code is canon"

When the spec, plan, README, or any reference doc disagrees with the
**code that is actually committed and passing tests**, the code
wins. Document the drift, don't silently re-align the code to the
prose.

- If you are working in a Copilot CLI session with a `plan.md`
  under `~/.copilot/session-state/<session-id>/`, add a row to the
  plan's "Spec corrections" table describing the discrepancy and
  which side you kept.
- Otherwise, mention the drift in the commit message body
  (e.g. `Spec correction: docs/reference/cli-contract.md claimed
  exit code 3 for "VM not found"; code returns 2. Kept code.`).

This rule applies to AGENTS.md too: if you change a load-bearing
behaviour described here, update this file in the same commit.

### Naming conventions

Crates that implement a common API use `<base>-<implementation>` names so every
family sorts together: `d2b-provider-{aca,host,relay}`,
`d2b-realm-codec-protobuf`, and `d2b-session-unix`. Do not place the
implementation before the base (for example, no `d2b-unix-session` or
`d2b-host-providers`). Keep the root workspace member list alphanumerically
sorted; the workspace policy tests enforce both rules.

The accepted d2b 2.0 local-root endpoint set consists of these four
unsuffixed PID1 units:

| Resource | Pattern |
| --- | --- |
| Local-root public socket | `d2bd.socket` |
| Local-root controller | `d2bd.service` |
| Local-root broker socket | `d2b-priv-broker.socket` |
| Local-root broker | `d2b-priv-broker.service` |

`d2b-priv-broker.socket` is the only PID1 broker socket activation. For every
child host-local realm, the local-root allocator pre-binds the public and broker
listeners and parent-spawns separate `d2bd-r-<realm-id>` controller and
`d2bbr-r-<realm-id>` broker processes. They are pidfd-supervised children, not
`.socket` or `.service` units. A realm controller owns its workload DAGs; there
is no `d2b@<workload>`-style or other per-workload systemd unit.

VM names are validated at eval time:

- Regex: `^[a-z][a-z0-9-]*$`.
- Reserved prefix: `sys-` (only the framework declares `sys-*` VMs).
- Reserved exact name: `launcher`.

Breaking any of these is a hard assertion in
`nixos-modules/assertions.nix`.

For the canonical glossary of internal identifiers (DAG node names,
bundle-relative artefact paths, broker op IDs) see
[`docs/reference/naming-conventions.md`](./docs/reference/naming-conventions.md).

### Component split & sibling flakes

The **core framework** in this repo covers: graphics, tpm, usbip,
audio, network, the auto-declared net VM, the per-VM store, the
CLI, the manifest contract.

Anything **identity- or workload-specific** lives in a sibling
flake and is composed per-VM:

- [`vicondoa/entrablau.nix`][entrablau] — Microsoft Entra ID
  joins (Himmelblau + TPM-bound machine credential).

Optional **desktop companion** pieces also live in sibling flakes:

- `vicondoa/d2b-toolkit` — shared Rust/Nix client DTOs, public-socket
  framing, redaction wrappers, Wayland color parsing, and Waybar helpers for
  desktop integrations.
- `vicondoa/d2b-wlterm` — Home Manager module and user-session launcher for
  persistent guest shells.
- `vicondoa/weezterm` — WeezTerm package/provider integration used by the
  terminal launcher when a d2b-aware terminal build is desired.

Consumer flakes that combine these pieces keep a single nixpkgs and toolkit
revision by using `inputs.d2b.inputs.nixpkgs.follows = "nixpkgs"`,
`inputs.d2b-toolkit.inputs.nixpkgs.follows = "nixpkgs"`, and
`inputs.d2b-wlterm.inputs.d2b-toolkit.follows = "d2b-toolkit"`. WeezTerm
follows only `nixpkgs`; its flake does not expose a toolkit input. The exact
copy-paste boilerplate lives in
[`docs/how-to/configure-desktop-terminal-integration.md`](./docs/how-to/configure-desktop-terminal-integration.md).

The composition pattern is intentionally one-way: d2b core does not import
identity, workload, or desktop companion flakes. Identity/workload flakes can
stay d2b-agnostic; desktop companions consume only d2b's public CLI/socket
contracts. Consumers compose workload modules on a specific VM:

```nix
d2b.vms.work.config.imports = [
  inputs.entrablau.nixosModules.default
];
```

If you're tempted to add a new sibling-shaped concern (e.g. a
specific desktop environment, a particular dev-shell flavour) to
the core framework, consider whether it belongs in its own flake
instead. The bar for landing it in core is: "every d2b user
plausibly wants this, and the framework cannot do the right thing
without it."

[entrablau]: https://github.com/vicondoa/entrablau.nix

### Workload lifecycle (realm-controller supervised)

Each realm's `d2bd` controller is the sole supervisor for that realm's workload
DAGs. The local-root controller is a PID1 service; child realm controllers and
brokers are separate parent-spawned processes born directly in their declared
cgroup leaves. The local-root controller adopts those children by verified
pidfd state after restart. Workload runners appear only in per-role leaves
beneath their owning realm.

Privileged effects remain broker-mediated. A child realm broker receives only
its pre-bound listener and allocator-approved namespace, cgroup, resource, and
lease FDs. Host-global mutation remains with the local-root broker. Controllers
and brokers are separate identities and processes; adding a realm tag to one
host-global broker is not an acceptable substitute.

#### Adding new per-workload behaviour

New per-workload work belongs inside the owning realm controller's DAG and
typed provider registry. Route realm-confined privileged effects through that
realm's broker and host-global allocation through a closed local-root allocator
operation. Do not add a per-workload `systemd.services.*` declaration or a
standalone host daemon. New process roles still require matching typed process
builders and role-matrix contract coverage.

## Wave validation and panel seal

### Concurrent end-of-wave gate

ADR 0045 wave work uses one full panel at the end of each wave, against the
exact integrated candidate tree. There is no preliminary plan panel and no
serial prohibition on later speculative work once the ADR is Accepted and its
required stable contracts exist. W0's separate Proposed and Accepted/index
panels are the completed bootstrap exception.

The integrator follows this order:

1. integrate and commit the candidate and run focused preflight;
2. open or update the PR, then create the immutable `xtask` snapshot for that
   exact base, heads, dependency graph, repository set, and prospective merge
   trees;
3. run three concurrent lanes on the snapshot: required GitHub CI, final
   local/host validators, and the full panel;
4. import validator command/result evidence and panel records into the external
   candidate-ID-addressed state directory;
5. run the canonical `xtask` wave seal and merge-eligibility checks; and
6. merge through GitHub only after every lane is complete.

A pending CI, validator, or panel lane is valid while the PR is open. It never
permits merge. Reviewers inspect the tree, plan, dependency/contract changes,
and live evidence status; they never run tests, builds, evals, or other
validation commands. Validators execute the required commands and import their
results; they do not review or substitute for panel signoff.

Validation, panel, and seal records are external artifacts bound to the exact
tree. Never commit them, copy them into generated artifacts, paste raw output
into the PR body, or include them in a release archive. The PR body contains
dependency, base/head/tree, `candidate_id`/`content_id`, and check-status
summaries only, with optional links to external evidence and no AI, assistant,
tool, or model metadata.

Every content change invalidates both the validator and panel lanes, including
documentation, generated output, dependency metadata, contract/index content,
or repository-set membership. Re-snapshot the tree and rerun both lanes. A
history-only rebase or retarget may reuse panel records only when the canonical
delivery history proof/tooling verifies byte-identical integrated content,
generated artifacts, dependency diff, contract fingerprints, and repository set.
Required CI still reruns on the new history.

Use the integrated delivery binary's generated command index for every delivery
operation. Do not infer options from a generic `--help`:

```bash
cd packages
cargo xtask delivery wave help
```

`cargo xtask delivery wave panel-request` writes the candidate-bound request.
`cargo xtask delivery wave panel-attest` validates a directory containing
exactly one record for every required role. Supply one
`--repo LOGICAL_ID=CHECKOUT_ROOT` mapping for every repository in the wave.
The request binds the candidate/content identities, snapshot digest, exact
ten-role roster, and required model.

Each role then supplies one strict 13-field attestation shaped like:

```json
{
  "artifact_kind": "d2b-delivery/panel-receipt",
  "schema_version": 1,
  "role": "software",
  "candidate_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "content_id": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
  "snapshot_sha256": "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
  "model_version": "gemini-3.1-pro-preview",
  "provider": "github-copilot",
  "run_id": "run-001",
  "receipt_locator": "github-copilot://runs/run-001/software",
  "output_sha256": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
  "signoff": true,
  "recommendations": []
}
```

The delivery tool rejects unknown or missing fields, the wrong model or
candidate binding, duplicate provider/run provenance, and inconsistent
`signoff`/`recommendations`. Attestations and raw review output never enter Git,
generated source, a PR body, or a release archive.

By policy, `signoff` is `true` iff `recommendations` is `[]`; otherwise the
recommendations are actionable findings. Any finding requires a content change,
which creates a new snapshot and invalidates every prior validation and panel
record. Green tests never waive review. Every wave, including documentation or
small waves, requires unanimous 10/10 signoff before merge.

### Required ten-role panel

Every role uses the provider and model version bound by the candidate's
`panel-request.json`:

| Role              | Focus |
|-------------------|-------|
| `software`        | Shell + Nix shape of every new module, daemon instrumentation, idempotency of sidecars, error handling in metric exporters. |
| `test`            | Coverage of new option schema, vsock CID collision cases, restart-policy gates, manifest schema drift, and what could regress invisibly. |
| `nixos`           | Module wiring, `lib.mkForce` / `lib.mkDefault` correctness, option declarations, systemd unit composition, and activation ordering. |
| `networking`      | Network surface changes, firewall posture across envs, DHCP/DNS regressions, bridge isolation, and routing invariants. |
| `security`        | Attack surface, host-relay trust posture, capability sets / syscall filters, authz boundaries, telemetry-label PII review, and retention defaults. |
| `rust`            | Rust API shape, error propagation, unsafe/FFI boundaries, schema generation, workspace dependency direction, and testability. |
| `product`         | Operator UX, naming surface, migration/deprecation policy, default-off opt-in shape, and actionable error messages. |
| `docs`            | Diataxis adherence in `docs/{reference,how-to,explanation}/`, CHANGELOG entries, schema md↔json drift, and AGENTS.md updates landing with load-bearing changes. |
| `observability`   | Cardinality of metric labels, span attribute hygiene (no secrets/cmd output/store paths), log/audit shape, retention, and dashboard/exporter correctness. |
| `kernel`          | pidfd, cgroup, namespace, mount, signal, ioctl, and filesystem semantics; kernel-version assumptions and Linux API edge cases. |

The wave seal requires all ten records to report `signoff: true` with empty
recommendations and to bind the same `candidate_id`, `content_id`, and
`snapshot_sha256`.
Historical smaller rosters do not apply to ADR 0045 waves.

### Commit-tag mapping

The tag examples in [Commit conventions](#commit-conventions) use this
mapping, and every commit that comes out of a panel-fix round MUST
carry the relevant tag:

- `Wn` = wave / phase number from the plan's parallelization graph
- `Wnfu` = first follow-up round on wave `n` after the first panel
  findings land
- `Wnfu<M>` = follow-up round `M` on wave `n` when a specific
  follow-up round must be named (for example `W5fu1`)
- `CN`, `HN`, `MN`, `LN` = finding ordinal `N`, prefixed by the
  severity letter from the JSON output (`critical` → `C`, `high` →
  `H`, `medium` → `M`, `low` → `L`)

Example: `( W1fu1 H3 )` means "wave 1, follow-up round 1,
addresses finding ranked HIGH-3."

Inline references to a specific commit in prose elsewhere may
use the compact form `(W2fu4 H10)` for readability — that's
shorthand for citing a commit, not the literal trailing tag
that the commit subject must end with. The trailing-tag form
in the commit subject itself always uses the spaced canonical
form (e.g. `... ( W2fu4 H10 )`).

### Tooling note

Rust `xtask` owns candidate snapshots, validation run/import, panel-request
generation, strict panel-attestation validation, identical-content proof,
sealing, and merge eligibility.
Run `cd packages && cargo xtask delivery wave help` for the generated,
machine-readable command/option index. Use those canonical commands and
external artifacts rather than host-local panel scripts or repository-resident
evidence.

## Test layout

The test tree has a binding local operating manual:
[`tests/AGENTS.md`](./tests/AGENTS.md). Read it before adding,
moving, or retiring test coverage. It defines the closed Layer-1 set,
the Layer-2 exceptions, the exact file locations, and the pin/ledger
updates required for each change.

At a glance:

| Location | Role |
| --- | --- |
| `tests/test-*.sh`, `tests/static.sh`, `tests/runner.sh` | Make-target entry points/wrappers; the manifest and Rust `xtask` own orchestration. Do not add a new top-level shell gate unless `tests/AGENTS.md` explicitly permits it. |
| `tests/unit/nix/cases/` | Auto-discovered nix-unit eval cases. After adding/removing one, run `make nix-unit-pin`. |
| `tests/unit/nix/eval-cases/`, `tests/unit/smoke/` | Flake-check and smoke-eval definitions. After adding/removing a flake check, run `make flake-matrix-pin`. |
| `packages/<crate>/src/**`, `packages/<crate>/tests/*.rs` | Rust unit and binary integration tests. Prefer these over shell gates when behaviour is hermetic. |
| `packages/d2b-contract-tests/tests/` | Rendered-artifact contract tests and policy lints. |
| `tests/unit/gates/`, `tests/unit/meta/` | Drift and meta gates; closed set. Regenerate affected artifacts with the matching `xtask gen-*` command instead of adding another gate. |
| `tests/integration/containers/` | Container integration tests run by `make test-integration`; final local validator lane after the PR opens. |
| `tests/host-integration/*.nix` | runNixOSTest VM checks run by `make test-host-integration`; final NixOS/KVM validator lane after the PR opens, not GitHub CI. |
| `tests/integration/live/`, `tests/host-integration/hardware/` | Live-host and hardware tests. Manual only; require deployed state or real devices. |

## CI / `flake.checks`

The root flake exposes these eval-only checks under
`flake.checks.<system>`:

| Check name             | What it evaluates                                                         |
| ---------------------- | ------------------------------------------------------------------------- |
| `eval-minimal`         | `examples/minimal/configuration.nix` against the framework module set.    |
| `eval-multi-env`       | `examples/multi-env/configuration.nix` (two isolated envs).               |
| `eval-template`        | `templates/default/configuration.nix` with sentinel fields overridden so the assertion block passes (TODO 2/3 substitutes). |
| `eval-graphics`        | `examples/graphics-workstation/configuration.nix`. **x86_64-linux only** — the framework's `checkVmPlatform` gate refuses graphics on aarch64. |

`with-entra-id` is intentionally absent from the root `flake.checks`
because it depends on the sibling `entrablau` input, which the
core flake does not (and should not) pull in. Its own flake is
still eval-checked by `tests/static.sh` during the per-example
iteration step, and CI also runs
`.github/workflows/eval-with-entra-id.yml` to execute
`nix flake check --no-build --all-systems --no-write-lock-file`
inside the example directory without coupling the root flake to the
sibling input.

## Versioning & changelog

The project follows [Semantic Versioning](https://semver.org/) and
[Keep a Changelog](https://keepachangelog.com/). The CHANGELOG is
organised **by version**, never by development phase.

### Changelog lifecycle

- **While a version is in development**, entries accumulate under the
  top `## [Unreleased]` block. Because `[Unreleased]` is a
  pre-release staging area, it MAY carry fine-grained process detail
  (wave/phase/follow-up/finding notes) if that helps the people
  cutting the release reason about what landed.
- **When a version is cut**, the `[Unreleased]` block is renamed to
  `## [X.Y.Z] - YYYY-MM-DD` and its contents are **summarised by
  version**:
  - Collapse any per-wave/per-phase substructure into the standard
    Keep-a-Changelog groups (`Added`, `Changed`, `Fixed`,
    `Deprecated`, `Removed`, `Security`). There are no
    `### Added (W6)`-style subsection headers in a released section.
  - Strip every internal process marker — wave/phase/revision/
    follow-up/panel/round/finding tags such as `W3`, `W4-fu`,
    `( W1fu3 H20 )`, `P6`, `D5/P2.3` — from the released prose.
  - Each released section reads as a coherent, consumer-facing
    summary of what changed, not as a log of how the work was
    organised internally.
- A fresh empty `## [Unreleased]` block is left at the top after a
  cut. `manifestVersion` / `bundleVersion` bumps and breaking
  changes always get an explicit released entry.

### Process markers stay out of shipped artifacts

Internal development bookkeeping — wave tags (`W3`, `W4-fu`,
`W2-followup`), phase tags (`P0`–`P7`, `v1.1-P4`, `ph6-…`),
decision codes (`D5/P2.3`), follow-up/round/finding refs
(`fu3`, `H20`, `(rust-1)`) — is for organising work, not for
shipping. Do **not** introduce these markers into:

- source comments in `nixos-modules/`, `pkgs/`, or `packages/`;
- shipped docs prose under `docs/{reference,how-to,explanation}/`,
  `README.md`, `SECURITY.md`, or example READMEs;
- any user-facing CLI surface (`clap` `about`/`help`/`long_help`
  text, error/observed-state messages, JSON envelope fields);
- CI workflow names, job names, step names, and test output that a
  contributor sees in GitHub Actions logs. CI labels should describe
  the behavior being validated (for example, "ADR index coverage
  guard" or "host validate dry-run"), not historical phase/process
  codes;
- released CHANGELOG sections.

These markers are still expected and welcome in the contexts where
they are load-bearing:

- planning artifacts (a session `plan.md`, the wave/parallelization
  graph) and pre-release CHANGELOG `[Unreleased]`;
- this file and the other process docs (Wave validation and panel seal, Commit
  conventions, `## Realm-local control-plane end state`) that
  *document* the methodology;
- `docs/adr/**` — ADRs are dated historical records and may name the
  wave/phase that produced a decision;
- commit messages and PR descriptions on in-development feature
  branches (see Commit conventions).

Note the deliberate exception: the consumer-facing
`d2b.defaultSwitchReadiness.<wave>` option namespace (keys
`w4Fu`…`p7`), its `readinessWaveSpecs` schema, and the
`/var/lib/d2b/validated/<wave>.json` evidence contract use
`wave`/phase tokens as **functional identifiers**. Those are part of
the public option/schema surface and are not bookkeeping; leave them.

### Landing changes (PR workflow)

`main` is protected: changes land through GitHub pull requests, never a direct
push or a pre-merge local integration. Use Git Town and ordinary GitHub PRs for
dependent work. Direct `git push` may publish commits only after Git Town owns
and verifies that branch's immediate parent; it must never create or change
stack topology or retarget a PR. After
the committed candidate passes focused preflight, immediately open or update
the PR and create its immutable snapshot from that open PR/stack state; do not
wait for final long local/host validation or the panel.

GitHub CI, validators, and the ten-role panel run concurrently against that
exact tree. The PR may report final lanes as pending while open, but it may not
merge until canonical `xtask` seal and merge-eligibility checks confirm all
required results. Any content change resets both validator and panel status.

PR bodies contain dependency, base/head/tree, `candidate_id`/`content_id`, and
check-status summaries only. Panel records, command output, validation payloads,
and seals remain external and may be linked, not embedded. Do **not** tag or list
the AI agent, assistant, tool, or model used to author or review a change, and
do not add PR-template fields requesting that metadata.

The detailed wave-tag commit convention in
[Commit conventions](#commit-conventions) applies to in-development commits on
feature branches; `main` itself is maintained as a by-release history.

## Commit conventions

> The trailing wave-tag scheme below applies to in-development
> commits on feature branches / worktrees, where wave/phase tags are
> load-bearing planning context. It does not license process markers
> in shipped code, docs, or released CHANGELOG sections — see
> [Versioning & changelog](#versioning--changelog).

- **Subject.** Short, imperative, prefixed with the touched
  area: `net: fix 10-eth-dhcp neutralization`,
  `manifest: bump manifestVersion to 2`,
  `cli: tighten exit-code table`.
- **Body.** Wrap at ~72 cols. Explain *why*, not what — the diff
  shows the what.
- **Traceability — canonical tag form (forward, W2fu4+).**
  Every commit subject MUST end with a trailing parenthesized
  tag in one of these exact forms:

  - `( W<N> )` — wave-N implementer work (no finding ref)
  - `( W<N>fu<M> )` — wave-N follow-up round M integrator
    merge (no finding ref); merge-shape suffixes like
    `octopus` are NOT permitted in the tag
  - `( W<N>fu<M> <S><N> )` — single finding fixed in
    follow-up round M. The finding-tag is `<S><N>` where
    `<S>` is the severity letter from the reviewer JSON
    (`C` = critical, `H` = high, `M` = medium, `L` = low)
    and `<N>` is the ordinal within that severity. Example:
    `( W2fu1 H3 )` = wave 2, follow-up 1, HIGH-3.
  - `( W<N>fu<M> <S1><N1> <S2><N2> ... )` — multi-finding
    follow-up commit when two or more findings genuinely express
    one coherent change and scattering them would not add
    review value. The trailing tag enumerates every finding
    closed by the commit, separated by single spaces. The commit
    body MUST explicitly call out the multi-finding scope (which
    findings are closed and why batching them in one commit
    aids review). Example: W3fu3 `( W3fu3 H4 H5 H6 )` aligned
    three docs (`privileges.md`, `AGENTS.md`,
    plan.md "Spec corrections") to point at `schemas/v2/` as
    the current bundle baseline in a single coherent commit.
    Reach for the single-finding form by default; reach for
    multi-finding only when the alternative is three or more
    trivially-small commits that all express the same
    statement.
  - `( W<N> <S><N> )` — single finding fixed inside the
    wave itself (rare; usually findings come during follow-ups)
  - `( W<N>a-<H> )` or `( W<N>a H<H> )` — post-wave **opening
    phase** that closes specific Spec-corrections deferrals or
    ships infrastructure work. Used when the work is genuinely
    pre-wave-N+1 prep rather than an in-wave follow-up. Examples:
    `( W3a-1 )` for the W3a-1 testing-infra batched harness,
    `( W4a H1 )` for the W4a-H1 audit retention commit. The
    spelling with the space (`W4a H1`) is what the W4a
    landings used and is the canonical form going forward; the
    dash-form (`W3a-1`) is permitted as a historical exception
    for the W3a commits that already shipped. Multi-finding
    follow-ups within an opening phase use the same
    `( W<N>afu<M> <S1><N1> <S2><N2> ... )` shape as a normal
    wave round (e.g. `( W4afu1 H1 H2 )` for a W4a follow-up
    closing R1 findings).

  Docs-only commits that don't close a specific finding (e.g.
  CHANGELOG.md grouping, AGENTS.md operating-manual updates after
  a wave closes) MAY omit the trailing tag when the subject
  itself is unambiguous about the scope (e.g. `CHANGELOG: W3fu4
  H1 H2 H3 H4 H5 grouped entry (R4 closure)`). Reach for the
  tag form whenever doing so would aid traceability; treat omitting
  it as the exception, not the default.

  No leading-tag form. No partition/topic words inside the
  parenthesized tag — those go in prose. Every commit
  produced in a panel-fix round MUST carry the relevant
  tag; see [Wave validation and panel seal](#wave-validation-and-panel-seal)
  for the mapping and wave-seal policy.

  Historical exception: pre-W2fu4 commits in W0/W1/W2 carry
  some leading-tag variants (`(W2 s3) ...`) and some merge
  subjects with topic words (`(W2fu1 ipc)`, `(W2fu2 octopus)`).
  These remain in history for reference; future waves use the
  canonical form above. See the
  `docs: codify trailing-tag canonical form` commit
  (W2fu4 H10) for the full retrospective.

- **Signing.** Sign-offs / GPG signing are not used.
- **AI/tool attribution.** Do not tag or list the AI agent, assistant,
  or model used in commit subjects, commit bodies, PR descriptions,
  changelog entries, or shipped docs. Do not add `Co-authored-by`
  trailers for AI tools unless the human explicitly requests one for
  that change.
- **Atomicity.** One logical change per commit. Mechanical
  reformat or rename passes go in their own commit so the
  human-reviewable diff stays small.

## Disk hygiene contract

- Test eval expressions MUST resolve the flake via `git+file://$ROOT`
  (use the `d2b_flake_ref` helper in `tests/lib.sh`), **never**
  `builtins.getFlake (toString $ROOT)`. A bare path makes Nix use the
  `path:` fetcher, which copies the ENTIRE working tree into the store —
  including the multi-GiB `packages/target` cargo artifacts (measured:
  ~36 GB / 5+ min per cold eval, re-triggered every time a cargo build
  churns `target/`). `git+file://` copies only git-tracked files
  (`target/` is gitignored), turning a 5-minute eval into <1 s. Caveats:
  (a) `nix eval` is pure by default and needs `--impure` with git+file;
  `nix-instantiate --eval` is impure by default and needs no flag.
  (b) When a script captures eval output via `2>&1` into a variable it
  then parses (jq, etc.), add `--quiet --no-warn-dirty` so the git+file
  `fetching git input` / `Git tree is dirty` stderr diagnostics don't
  corrupt the parsed JSON. (c) git+file sees uncommitted edits to
  TRACKED files but NOT untracked files — identical to `nix flake check`,
  so "commit before building" still holds (see "Edit -> commit ->
  validate").
- Every test script that creates repo-local scratch state MUST use
  `d2b_mktemp` from `tests/lib.sh`; do not call raw
  `mktemp -d -p "$ROOT"`.
- Per-process bookkeeping (`cleanups.<PID>`, `scratch-registry`)
  lives in `${D2B_BOOKKEEPING_DIR:-${TMPDIR:-/tmp}/d2b-bookkeeping}`,
  NOT in `$ROOT`. Parallel-test timing log/status files live in
  `${TMPDIR:-/tmp}/d2b-static-timing.$$/`. Both moves are
  required so volatile files can't race
  `builtins.getFlake (toString $ROOT)` source-capture during
  flake-eval gates (W2fu4 H8/H9).
- Every maintained host and guest crate uses the single workspace and lockfile
  under `packages/`. Focus broker feature passes with
  `-p d2b-priv-broker`; focus the persistent-shell helper's real bridge with
  `-p d2b-guest-shell-runner --features real-libshpool`.
- After each wave merge, the integrator MUST complete the whole post-wave
  cleanup sequence:
  1. Delete the merged remote feature branch.
  2. If the finished worktree has a real `packages/target/`, clean that build
     output; otherwise confirm the target is the shared-cache symlink or absent.
  3. Remove the finished local worktree.
  4. Delete the corresponding local feature branch.
  5. Run `nix-collect-garbage` and verify `git worktree list` contains only
     active work.
- For the operator host running heavy iteration: prune OLD
  NixOS system generations periodically:

  ```
  sudo nix-collect-garbage --delete-older-than 7d
  ```

  Old `/nix/var/nix/profiles/system-N-link` symlinks are auto-gcroots;
  each pins ~1-2 GiB of unique closure. Without periodic pruning a
  host doing frequent rebuilds (today's W2fu4 baseline: 383
  generations from 10 days of work, pinning 471 GiB) silently fills
  its disk. The gate's default post-`nix store gc` only removes
  unreferenced paths, never old generations.
- `tests/static.sh` can run an opt-in deep GC after the gate:

  ```
  D2B_POST_GATE_DEEP_GC=1 bash tests/static.sh           # user gens only
  D2B_POST_GATE_DEEP_GC=1 \
  D2B_POST_GATE_DEEP_GC_SUDO=1 \
  bash tests/static.sh                                  # + system gens
  ```

  `D2B_POST_GATE_DEEP_GC_SUDO=1` uses `sudo -n` and skips fail-open
  with a clear log if passwordless sudo isn't available. Threshold
  defaults to 7 days; override with `D2B_POST_GATE_DEEP_GC_DAYS=N`.
  Off by default — this is operator policy, not gate policy.
- `D2B_SKIP_WITH_ENTRA_ID=1` skips the per-example flake check for
  `examples/with-entra-id` when its pinned `vicondoa/entrablau.nix`
  input fails the per-example cargo fetch with a transient crates.io
  403 against `libhimmelblau-0.8.18` / `kanidm-hsm-crypto-0.3.6`.
  `tests/static.sh` performs one in-band retry before failing the
  example; the skip knob is an explicit, panel-justifiable W3
  carve-out used only after the retry also fails. Added with the W3
  integration merge; re-evaluate once the entra-id input bumps past
  the affected revision.
- Before `git worktree remove`, confirm the worktree's
  `packages/target/` is the shared-cache symlink (or absent), not a
  real per-worktree directory. Clean a real target before removal.
- `tests/tools/preflight-disk-space.sh` fails the wave when free disk under
  `$ROOT` drops below 10 GiB. Runs after the orphan reapers but BEFORE
  the rust toolchain bootstrap so the fail-closed guard cannot be
  bypassed by disk-consuming setup (W2fu4 H2).
- `nix flake check` now builds real `cargo-deny` + `cargo-audit`
  derivations (via `checks.${system}.rust-deny` / `.rust-audit`).
  Each derivation fetches the pinned RustSec advisory DB snapshot
  from the Nix store (no network at build time) and runs cargo-deny /
  cargo-audit against `packages/Cargo.lock`. The advisory DB is a
  `fetchFromGitHub` pinned to a specific commit; update the rev + hash
  in `flake.nix` periodically to pick up new advisories. Wall-clock
  impact: seconds per check (no compilation, just lockfile analysis).

## Critical subsystems — handle with care

Touch these only with a clear plan and a corresponding test run.

| System                              | Where                                                                                  | Risk if broken                                                            |
| ----------------------------------- | -------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| Net VM networking / firewall        | `nixos-modules/net.nix` (the `lib.mkForce` neutralization of `base.nix`'s `10-eth-dhcp`, plus the per-env MTU/MSS and east-west wiring) | Net VM dual-stacks DHCP on its uplink, breaks NAT, or weakens same-env isolation unexpectedly. Validate with `tests/net-vm-network-eval.sh`. |
| Per-VM `/nix/store` hardlink farm   | `nixos-modules/store.nix`, `/var/lib/d2b/vms/<vm>/store{,-meta}/`, `nixos-modules/processes-json.nix` (`virtiofsdRunner` ro-store `--shared-dir`), daemon `StoreSync` op + broker `store_view_farm` | The guest's `/nix/store` MUST be the per-VM closure-only farm `/var/lib/d2b/vms/<vm>/store`, never the host's full `/nix/store`: virtiofsd-ro-store's `--shared-dir` points at that farm (the `share.source == "/nix/store"` string stays as the eval-time sentinel — do not "simplify" it back to serving `/nix/store`, that re-leaks the whole host store to every guest). Requires `/var/lib/d2b` and `/nix/store` on the **same filesystem** — hardlinks can't cross FS boundaries; if split, `d2b vm switch` refuses with a fatal error. The broker builds the farm inside a private mount namespace where `/nix/store` is lazily detached (NixOS bind-mounts `/nix/store` on itself, so a same-`st_dev` cross-vfsmount `link(2)` returns `EXDEV` — recoverable, distinct from a fatal different-filesystem `EXDEV`); a `link(2)` `EMLINK` on a `--optimise`d store's saturated empty-file inode falls back to a byte copy. The daemon owns the sync; there is no per-VM `store-sync` unit. |
| TPM persistence (per-VM swtpm)      | `/var/lib/d2b/vms/<vm>/swtpm/`; spawned via broker `SpawnRunner` from `packages/d2b-host/src/swtpm_argv.rs` and supervised by `d2bd` as a child of the VM's DAG. The broker **provisions + hardens** this dir on first start (`packages/d2b-priv-broker/src/ops/swtpm_dir.rs`, gated on `seccomp_policy_ref == "w1-swtpm"`): fd-safe create (owner `d2b-<vm>-swtpm`, mode 0700, inherited ACLs cleared), reconcile-in-place on a correct-owner existing dir, fail-closed on owner/type/symlink mismatch, ancestor `--x` traverse ACL, stale `tpm.sock` unlink — emitting the path-free `PrepareSwtpmDir` audit op. | Holds the per-VM TPM 2.0 NVRAM + EK seed. **Wiping it looks like device tampering to any IdP** (Entra ID, Intune, Bitlocker-style policies) and forces re-enrollment. Never zero it casually. The per-VM state root is `3770` (setgid **+ sticky**) so a non-owner role UID cannot rename/replace the `swtpm/` entry; an identity-bound, root-owned marker at `/var/lib/d2b/swtpm-markers/<vm>` makes a *previously-provisioned-then-missing/replaced* dir **fail the VM start closed** (`previously-provisioned-swtpm-state-missing`) rather than silently re-creating an empty TPM. The state directory's ACLs are asserted by `tests/unit/smoke/smoke-eval-tpm.nix`; the broker hardening by `packages/d2b-priv-broker/src/ops/swtpm_dir.rs` tests. |
| USBIP passthrough                   | `nixos-modules/components/usbip.nix` (eval-time gating) + broker `UsbipBindFirewallRule` + `SpawnRunner` (per-busid attach process supervised by `d2bd`) | Eval-time gating still scopes attach to opted-in envs (validated by `tests/usbip-gating-eval.sh`). At runtime, attach/detach runs through the broker — there is no per-env `d2b-sys-<env>-usbipd-*` socket. Misrouted attaches expose a YubiKey to the wrong env. |
| GPU sidecar (graphics VMs)          | `nixos-modules/components/graphics.nix` + broker `SpawnRunner` for cloud-hypervisor on graphics VMs; pidfd handed back via `OpenPidfd` and supervised by `d2bd` | Graphics VMs run cloud-hypervisor with the GPU device attached. Restarting `d2bd` no longer terminates CH — pidfd handoff means the child outlives a daemon reconnect — but the broker spawn path is the only audited place CH is launched. Bypassing it breaks the audit trail. Validate with `tests/video-sidecar-hardening-eval.sh`. |
| Video sidecar (graphics VMs)        | `nixos-modules/components/video/guest.nix`, `nixos-modules/processes-json.nix`, `pkgs/vhost-user-video/`, `packages/d2b-host/src/video_argv.rs`, broker `SpawnRunner{role: Video}` | `graphics.videoSidecar = true` is an explicit opt-in H264 decode path: guest `virtio_media` + patched Cloud Hypervisor `--vhost-user-media` + patched crosvm `device video-decoder --backend vaapi`. There is no per-VM video systemd unit, no stock crosvm/CH fallback, and no free-form video extra args. The video runner MUST use the dedicated `d2b-<vm>-video` principal, not `d2b-<vm>-gpu`, so broker/activation ACLs can deny host Wayland/PipeWire/Pulse sockets to video without breaking GPU cross-domain. The broker masks `/dev` for the video runner and exposes only the declared device allowlist: default `/dev/dri/renderD128`, plus `/dev/nvidiactl`, `/dev/nvidia0`, and `/dev/nvidia-uvm` only when `graphics.videoNvidiaDecode = true`. `virtio_media` is a guest module, not a host `/proc/modules` preflight requirement. Firefox/VA-API uses the separate experimental `graphics.virglVideo` GPU path; it is default-off and must not be treated as stable video-sidecar coverage. Validate with `tests/video-contract-eval.sh`, `tests/video-argv-shape.sh`, and `tests/minijail-validator-video.sh`. |
| UI color contract / niri backend    | `nixos-modules/ui-colors.nix`, `nixos-modules/niri-vm-borders.nix`, `docs/reference/ui-colors.{md,json}`, `tests/unit/nix/cases/niri-vm-borders.nix`, and sibling consumers such as `vicondoa/d2b-wlcontrol` | The compositor-agnostic `d2b.site.ui` / `d2b.envs.<env>.ui` / `d2b.vms.<vm>.ui` color model is the source of truth for host/env/VM/state colors. Generated `/etc/d2b/ui-colors.json` and `/etc/d2b/ui-colors.css` are public presentation metadata, not authz or policy inputs. Niri-specific settings belong only under `d2b.site.ui.compositors.niri`; do not add compositor-specific color source options. Keep the JSON schema, reference docs, GTK CSS `@define-color` names, and nix-unit artifact-shape tests in sync. Downstream tools must fail visibly but remain usable when the artifact is missing or malformed, without reading root-owned d2b state directly. |
| Unsafe-local provider, launcher, and persistent-shell helper | `nixos-modules/options-realms-workloads.nix`, `nixos-modules/unsafe-local-workloads-json.nix`, `packages/d2b-core/src/unsafe_local_workloads.rs`, `packages/d2b-contracts/src/unsafe_local_wire.rs`, `packages/d2b-unsafe-local-helper/src/{shell_runtime,shell_supervisor,shell_socket,output_ring,tty_exec}.rs`, and `docs/reference/unsafe-local-provider.md` | `unsafe-local` is explicit and default-denied. It runs only as the exact authenticated requesting uid and provides no isolation boundary. Public metadata never carries configured argv or shell policy; those come only from the integrity-pinned private bundle. A persistent-shell supervisor in a verified transient USER scope—not the reconnectable helper or d2bd—owns the login-shell PTY, bounded merged-output ring, attachment, and private same-UID listener. Ledger adoption preserves ambiguous sessions as degraded; teardown closes the PTY and signals only the exact re-verified scope. The helper-wide ring reservation is bounded, terminal responses transfer exactly one CLOEXEC stream fd, and shell names, supervisor ids, paths, environment, process/unit identity, and bytes stay out of Debug/errors/audit. Do not add cross-uid execution, a direct compositor fallback, VM state/network/device semantics, a root service, per-VM unit, broker op, free-form shell command, or broad same-UID cleanup. |
| Manifest contract                   | `docs/reference/manifest-schema.{md,json}` + `nixos-modules/manifest.nix`               | Version-pinned via `manifestVersion`. Adding, removing, or renaming a per-VM field requires bumping the version, updating the schema, and noting it in the CHANGELOG. The `static.sh` md↔json drift gate catches partial updates. |
| Manifest bundle — private artifacts | `docs/reference/manifest-bundle.md` + `docs/reference/schemas/v2/*.json` + `packages/d2b-core/src/{bundle,host,processes,privileges,closures,minijail_profile}.rs` + `packages/d2b-contracts/src/provider_registry_v2.rs` + `nixos-modules/{bundle,bundle-artifacts,host-json,processes-json,privileges-json,closures-json,minijail-profiles,provider-registry-v2-json}.nix` + `packages/xtask/src/main.rs` (`gen-schemas`) | Sensitive bundle artifacts install at `root:d2bd` 0640 and ground every broker/sandbox/runner behaviour. `d2b-core` DTOs are canonical; the provider registry DTO is canonical in `d2b-contracts`; `d2b._bundle` is the typed internal artifact table that owns JSON data, install names, classifications, and `/etc/d2b` materialization for every bundle artifact. `provider-registry-v2.json` carries only canonical IDs and opaque existing bundle intent IDs: never add argv, host paths, or credentials. Add new bundle artifacts through `nixos-modules/bundle-artifacts.nix` instead of hand-writing parallel install logic in each emitter. Committed schemas under `docs/reference/schemas/v2/` ARE the contract and the `tests/unit/gates/drift-check.sh` gate enforces `xtask gen-schemas` + `git diff --exit-code` through `make test-drift`. Breaking the schema without an intentional `bundleVersion`/`schemaVersion` bump silently breaks every downstream consumer. |
| Realm-local control plane | `packages/d2bd/**`, broker/provider/session crates, generated endpoint/process/storage contracts, and [ADR 0045](./docs/adr/0045-provider-and-transport-framework.md) | PID1 owns only the fixed local-root endpoint set. The local-root allocator pre-binds listeners and parent-spawns a distinct controller and broker for each child realm, returning separate pidfds for supervision. Child processes have separate identities, cgroup leaves, state/audit roots, and FD/lease authority; they are not PID1 units. Never collapse them into one realm-tagged broker or add per-workload units. |
| Storage lifecycle / restart / synchronization | Planned generated contracts in `d2b-core::{storage,process_restart,sync}` + Nix emitters, broker storage/sync ops, daemon lifecycle DAG integration, and docs [ADR 0034](./docs/adr/0034-storage-lifecycle-restart-and-synchronization.md) / [`docs/explanation/storage-lifecycle.md`](./docs/explanation/storage-lifecycle.md) | Managed paths, restart adoption, locks, leases, cleanup, and degraded-state reporting are control-plane contracts. Normal daemon restarts are continuation events: do not broad-sweep `/run/d2b`; first re-discover adoptable runners from declared cgroup leaves, open fresh pidfds, verify identity, and quarantine/degrade ambiguity. Pidfds are not persisted. New advisory locks use OFD locks with `O_CLOEXEC`, explicit fd transfer only, and total acquisition order. The broker resolves storage/lock mutations from opaque bundle ids through anchored `openat2`/fd-relative path walking; daemon-owned ledgers are diagnostics, never repair authority. |
| Eval-time assertions                | `nixos-modules/assertions.nix`                                                          | These are the framework's contract with consumers. Loosening one silently turns a previously-rejected misconfig into runtime breakage. New assertions need a matching case in `tests/assertions-eval.sh`. |
| Guest-control exec session table    | `packages/d2bd/src/{exec_session,exec_session_real}.rs`, `run_exec_owner` in `packages/d2bd/src/lib.rs`, `packages/d2b/src/exec_client.rs`, `packages/d2b-contracts/src/public_wire.rs` (`ExecOp`/`ExecOpResponse`) | Arbitrary `d2b vm exec` is **admin-only**; configured `d2b launch` local-VM items may use the same detached guest-control backend with launcher authority because argv is resolved exclusively from the hash-verified private bundle. Both run through `d2bd` plus authenticated guest-control vsock to `guestd`. Attached exec uses the daemon's in-process **session table**: per-session workers own one authenticated guest-control client and proxy typed exec ops. **guestd runs every exec as the VM's workload user (`ssh.user`) inside a real PAM login session (`systemd-run --property=PAMName=login --uid=<user>`) — never as root; the wire `user` field is ignored and the target user is host-fixed, bare `argv[0]` is resolved by the workload user's login `PATH`, and each attached exec runs in a process-unique named transient unit (`d2b-exec-<…>.service`) that teardown stops via `systemctl kill` so a quiet command cannot outlive owner-disconnect, cancel, or the runtime ceiling. Operators elevate with `sudo` inside the session.** Detached non-TTY exec is enabled with `d2b vm exec -d <vm> -- <cmd>` and managed through VM-first verbs (`d2b vm exec <vm> list`, `logs <id>`, `status <id>`, `kill <id>`); command forms always require `--`, so those verb words remain valid VM names. Detached jobs and configured local-VM launches also run as the workload user, never root: the root detached runner only owns trusted slot/log files, re-validates the non-root uid before spawning the workload unit, and fails terminally rather than falling back to direct root execution. Guestd reconciles detached runner/workload units on startup, cleans orphaned workloads, and runs a periodic reaper for terminal records and retained logs; `kill` maps to idempotent two-phase `ExecCancel` (SIGTERM/grace/SIGKILL). There is **no per-VM systemd unit, no new broker op, and no SSH** — the guest owns the PTY; the host only flips termios for attached TTY via an RAII raw-mode guard restored on every exit/error/panic. The admin `SO_PEERCRED` check runs before arbitrary exec session setup; configured launch instead requires local launcher/admin authority and a trusted configured item. Old/non-guest-control generations fail closed (exit `70`) with no proxy and no SSH fallback. Session-table caps (global/per-UID/per-VM), detached slot/log quotas, and rate limits are enforced before connect/auth or create. Attached audit emits one redacted kind=critical session-establishment event (vm/peer_uid/tty); detached create/kill daemon audit carries only vm/peer_uid/action/result/exec_id, while configured-launch audit adds target/item/operation correlation without execution details. Opaque session handles, argv, stdio, env, cwd, and paths never reach any Debug/trace/audit/metric surface. Validate with the `exec_session`/`exec_client` hermetic test matrices. |
| Unsafe-local persistent shells | `packages/d2bd/src/{workload_dispatch,unsafe_local_helper,unsafe_local_terminal,shell_backend}.rs`, shell owner dispatch in `packages/d2bd/src/lib.rs`, `packages/d2b-unsafe-local-helper/src/{shell_runtime,shell_supervisor}.rs`, and `tests/host-integration/unsafe-local-helper.nix` | `d2b shell` remains **admin-only** for every provider. Unsafe-local target identity and `defaultName`/`maxSessions` come only from the hash-verified private bundle; public `ShellOp` keeps protocol v3 and carries no policy, uid, argv, env, cwd, or path. The daemon dispatches helper protocol v2 to the exact `SO_PEERCRED` uid, validates exactly one connected CLOEXEC stream fd, and multiplexes terminal protocol v1 behind a fresh opaque public handle. Disconnect/`CloseAttach` detach but never kill; `Kill` targets only the helper-verified transient user scope. Shells survive CLI, daemon, and helper reconnects while that scope and the non-lingering user manager live. User logout ends them by design. User scopes provide lifecycle ownership, **not containment from other processes with the same host uid**. There is no root unit, broker op, per-VM service, SSH path, host-shell fallback, direct-compositor fallback, or automatic replay after an ambiguous daemon timeout. Never log/audit/label shell names, supervisor ids, public handles, terminal bytes, helper diagnostics, PIDs, unit names, argv, env, cwd, or paths; audit may use configured target/peer uid and fixed digests, while metrics use closed provider/component/operation/outcome/error labels. |
| Lifecycle permission group          | `nixos-modules/host-users.nix`                                                          | Membership in `d2b` + `SO_PEERCRED` at `public.sock` accept time is the **only** lifecycle authorisation surface. There is no polkit allowlist; wiring anything else into the group inverts the threat model. **Exception:** the guarded `ExecStop` shutdown hook runs as uid 0 and receives the narrow `HostShutdown` role, which is permitted only for `vmStop` during host-shutdown teardown (see `packages/d2bd/src/admission.rs`). This exception is scoped strictly: all other admin-only operations (exec, USB attach, key rotation, host prepare, audit export) are denied for this role. The daemon-restart continuation guard is preserved: `Restart=on-failure` restarts never receive `HostShutdown` treatment because the restarting daemon re-adopts runners and the shutdown hook only runs under systemd stop with a live `stopping` system state check. |
| SSH key generation / rotation       | `nixos-modules/host-keys.nix`, `host-activation.nix`                                    | The framework owns `${cfg.site.keysDir}/<vm>_ed25519`. `d2b keys rotate` MUST NOT touch consumer-supplied keys. |
| virtiofsd sandbox model             | `nixos-modules/minijail-profiles.nix` (virtiofsdProfiles), `packages/d2b-priv-broker/src/sys.rs` (`clone3_spawn_runner` user-NS path), `nixos-modules/processes-json.nix` (argv emit) | virtiofsd profiles MUST declare zero host capabilities (`capabilities = []`), `requiresStartRoot = false`, and a `userNamespace` block mapping in-NS UID/GID 0 to the per-share principal. Normal VM shares map to `d2b-<vm>-runner`; the guest-control token share (`d2b-gctl`) maps to the narrower `d2b-<vm>-gctlfs` principal. The broker pre-establishes the user namespace via `clone3(CLONE_NEWUSER)` + `pipe2` sync + `/proc/<pid>/uid_map` writes BEFORE virtiofsd's first instruction runs. virtiofsd argv MUST include `--sandbox=chroot --inode-file-handles=never` and `--readonly` for every `readOnly` share (`ro-store`, `d2b-gctl`). Reintroducing host caps, `requiresStartRoot=true`, or `--sandbox=namespace` violates [ADR 0021](./docs/adr/0021-broker-user-namespace-for-virtiofsd.md). Validate with `tests/minijail-validator-virtiofsd.sh` + `tests/virtiofsd-argv-shape.sh`. |

## Don'ts (security-relevant)

- **Don't remove `lib.mkForce` from the net VM's `10-eth-dhcp`
  neutralizer.** Verify any reshape of `net.nix` against
  `tests/net-vm-network-eval.sh` first.
- **Don't relax the VM-name regex or reserved prefixes.**
  `sys-*` and `launcher` are reserved so the framework can
  declare its own VMs without name collisions and so the CLI
  can route subcommands unambiguously.
- **Don't break the manifest contract silently.** Schema +
  prose + emitter move together, with a `manifestVersion`
  bump and a CHANGELOG entry.
- **Don't paper over a failing assertion by deleting it.** If
  the assertion is wrong, fix its predicate; if the predicate
  is right but the failure mode is misleading, fix the message.
- **Don't add a per-workload systemd unit or bypass the realm process
  model.** Workload lifecycle stays in the owning realm controller's DAG.
  Child realm controllers and brokers are separate parent-spawned,
  pidfd-supervised processes; only the fixed local-root endpoint set is
  PID1-owned. Do not substitute one host-global broker with realm tags or add a
  standalone framework daemon outside the ADR 0045 model.
- **Don't reintroduce a bash CLI fallback or env-knob escape
  hatch.** The Rust CLI is the only operator surface;
  `D2B_LEGACY_BASH_OPT_IN`, `D2B_LEGACY_CLI`, and
  `D2B_NATIVE_ONLY` are no-ops.
- **Don't commit secrets, hostnames, real user identifiers, or
  real network ranges.** Use generic names (`alice`,
  `corp-vm`, `work`, `personal`) and RFC1918 / RFC5737 ranges
  in docs and examples. The repo has no host-identifier
  leaks today; keep it that way.
- **Don't introduce a new linter, formatter, or pre-commit
  hook unless explicitly requested.** `nix flake check`,
  `tests/static.sh`, and `shellcheck` (already wired into
  `static.sh`) are the baseline.
- **Don't add a new `nixpkgs.overlays` entry or change
  `nixpkgs.url` casually.** The overlay surface is part of
  the public ABI and overlay churn rebuilds the world for
  every consumer.
- **Don't leak internal process markers into shipped artifacts.**
  Wave/phase/revision/follow-up/finding tags (`W3`, `W4-fu`, `P6`,
  `D5/P2.3`, `( W1fu3 H20 )`) belong in planning artifacts,
  pre-release `[Unreleased]`, ADRs, this file's process sections,
  and feature-branch commits — never in shipped source comments,
  shipped docs prose, CLI help/error text, or released CHANGELOG
  sections. See [Versioning & changelog](#versioning--changelog).
  The functional `d2b.defaultSwitchReadiness.<wave>` option
  surface is the one deliberate exception.
- **Don't let a host process hold realm credentials, or treat relay
  identity as local auth (ADR 0032).** Realm relay/session/provider
  credentials, remote node registries, and realm audit belong inside
  a per-realm gateway guest VM — never in `d2bd`, the broker, the
  host bundle, host-readable storage, or any host-side activation
  artifact. A relay-authenticated peer is never mapped to local
  `Admin`; `SO_PEERCRED` + `d2b` group membership stays the only
  local lifecycle authz surface. Work and personal realms never share
  a gateway guest or an L2 bridge.
- **Don't add ad-hoc storage, ACL, cleanup, or lock ownership paths.**
  Storage and synchronization changes must fit the ADR 0034 contract:
  broker-resolved opaque ids, anchored path resolution, OFD locks with
  `O_CLOEXEC`, explicit fd transfer only, restart-aware adoption before
  cleanup, and typed degraded-state reporting instead of broad chmod,
  chown, setfacl, or `/run/d2b` sweeps. Every new host-mutable
  path or lock surface must add or reuse a generated `storage.json` /
  `sync.json` row, name a single repair owner, and route repair through
  that owner rather than adding a second activation/broker/daemon fixer.

## cgroup slice naming + ownership-marker conventions

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
  mutation while running as uid 0** (Phase A privileged setup
  — `+controllers` cascade, slice/leaf `mkdir`, `fchown` to
  `d2bd`'s uid/gid — legitimately runs as root per ADR 0011
  Decision item 2; the uid != 0 invariant applies to the
  steady-state cgroup code path after privilege drop). See
  [`docs/reference/cgroup-delegation.md`](./docs/reference/cgroup-delegation.md)
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
[`docs/explanation/host-prepare.md`](./docs/explanation/host-prepare.md)
§ "NetworkManager / systemd-networkd coexistence" and ADR 0013 for
the rationale.

## Realm-local control-plane end state

[ADR 0045](./docs/adr/0045-provider-and-transport-framework.md) supersedes the
ADR 0015 exactly-three-unit invariant for d2b 2.0. Agents MUST treat this as the
contract:

- PID1 owns the fixed local-root `d2bd.socket`, `d2bd.service`,
  `d2b-priv-broker.socket`, and `d2b-priv-broker.service` endpoint set.
- The local-root broker is the only PID1 socket-activated broker and contains
  the host-global allocator.
- Each child host-local realm has a separate controller and broker identity,
  listener, state/audit root, cgroup partition, and process. The allocator
  pre-binds both listeners and parent-spawns both processes; neither is a PID1
  unit or receives `SD_LISTEN_FDS`.
- The local-root controller supervises and adopts child controller/broker
  pidfds. Each realm controller supervises only its workload DAGs.
- There are no per-realm child `.socket`/`.service` units.
- There are no per-workload systemd templates or units. Unit count does not
  scale with realm or workload count.
- Realm-confined privileged effects go through that realm's broker; global host
  effects stay in closed local-root allocator operations. One broker with realm
  tags does not satisfy the boundary.
- The CLI remains Rust-only with no Bash fallback.

Validation must prove the fixed local-root units, absence of child/per-workload
units, separate parent-spawned controller and broker processes, direct cgroup
placement, pidfd supervision/adoption, and child-broker FD/lease confinement.

## References

- [docs/adr/0045-provider-and-transport-framework.md](./docs/adr/0045-provider-and-transport-framework.md)
  — the accepted d2b 2.0 architecture and delivery contract, including the
  realm-local process model and immutable-wave seals.
- [docs/adr/0015-daemon-only-clean-break.md](./docs/adr/0015-daemon-only-clean-break.md)
  — historical daemon-only decision; ADR 0045 supersedes its one-daemon,
  one-broker, exactly-three-unit constraint while retaining no per-workload
  units and broker-mediated mutation.
- [docs/adr/0017-no-bash-fallbacks-invariant.md](./docs/adr/0017-no-bash-fallbacks-invariant.md)
  — the Rust CLI never invokes bash; CI gates enforce no new
  `Command::new("bash")` sites.
- [docs/adr/0018-microvm-nix-removal.md](./docs/adr/0018-microvm-nix-removal.md)
  — d2b owns its per-VM substrate via `vm-options.nix` +
  `vm-evaluator.nix`; the `microvm.nix` flake input is gone.
- [docs/adr/0021-broker-user-namespace-for-virtiofsd.md](./docs/adr/0021-broker-user-namespace-for-virtiofsd.md)
  — broker pre-establishes a single-entry user namespace via
  `clone3(CLONE_NEWUSER)` so virtiofsd runs fake-root inside the
  NS while exposing **zero** host capabilities. Any change to the
  virtiofsd minijail profile or argv shape MUST preserve this
  contract.
- [docs/adr/0031-bare-command-and-detached-exec.md](./docs/adr/0031-bare-command-and-detached-exec.md)
  — bare command-name exec resolution and enabled detached
  workload-user exec with VM-first management verbs.
- [docs/adr/0032-d2b-v2-constellation-control-plane.md](./docs/adr/0032-d2b-v2-constellation-control-plane.md)
  — evolves `d2bd` into a transport-neutral constellation
  daemon. **Load-bearing invariant:** the host daemon/broker hold
  **no** realm relay/provider credentials, remote node registries,
  or realm audit (those live inside a per-realm gateway guest); and
  **relay identity is not local auth** — relay credentials
  authenticate relay/transport access only, are never mapped to a local
  lifecycle role, and `SO_PEERCRED` + `d2b` group membership remains
  the sole local lifecycle authz surface.
- [docs/adr/0034-storage-lifecycle-restart-and-synchronization.md](./docs/adr/0034-storage-lifecycle-restart-and-synchronization.md)
  — selected design for generated storage, restart/adoption, and
  synchronization contracts. **Load-bearing invariant:** normal daemon
  restarts are continuation events; recover/adopt/quarantine before
  cleanup, never persist pidfd authority, and route host storage/lock
  mutation through broker-resolved opaque ids.
- [README.md](./README.md) — consumer-facing intro, install,
  manual integration walkthrough.
- [CHANGELOG.md](./CHANGELOG.md) — Keep-a-Changelog, entries
  accumulate under `## Unreleased` until a tag cuts them.
- [SECURITY.md](./SECURITY.md) — disclosure path + scope.
- [docs/explanation/design.md](./docs/explanation/design.md) —
  threat model, defenses-in-depth list, *Why not X* FAQ.
- [docs/explanation/daemon-lifecycle.md](./docs/explanation/daemon-lifecycle.md)
  — daemon DAG executor, pidfd handoff, supervisor reconciliation.
- [docs/reference/privileges.md](./docs/reference/privileges.md) —
  authoritative broker op catalogue.
- [docs/reference/daemon-api.md](./docs/reference/daemon-api.md) —
  `public.sock` wire surface, audit format, retention.
- [docs/reference/manifest-schema.md](./docs/reference/manifest-schema.md)
  + [docs/reference/manifest-schema.json](./docs/reference/manifest-schema.json)
  — the manifest contract.
- [docs/reference/cli-contract.md](./docs/reference/cli-contract.md) —
  CLI lifecycle FSM, signal semantics, exit codes, JSON vs human
  output.
- [docs/reference/realm-policy.md](./docs/reference/realm-policy.md) —
  host-resident vs gateway-backed realm policy, default-deny
  cross-realm behavior, and `d2b realm list` / `inspect`
  inspection surfaces.
- [docs/reference/constellation-observability.md](./docs/reference/constellation-observability.md)
  — bounded `d2b op inspect`, TraceContext handling, degraded partial
  results, and telemetry redaction/cardinality constraints.
- [docs/how-to/configure-work-gateway.md](./docs/how-to/configure-work-gateway.md)
  — configure a dedicated work/provider realm gateway and verify the
  default-deny boundary.
- [docs/how-to/migrate-d2b-v0-to-v1.md](./docs/how-to/migrate-d2b-v0-to-v1.md)
  — consumer migration guide for v0.x → v1.0.
- [docs/how-to/migrate-d2b-v1-0-to-v1-1.md](./docs/how-to/migrate-d2b-v1-0-to-v1-1.md)
  — consumer migration guide for v1.0 → v1.1.
- [docs/how-to/migrate-d2b-v1-1-to-v1-2.md](./docs/how-to/migrate-d2b-v1-1-to-v1-2.md)
  — consumer migration guide for v1.1 → v1.2, including the
  canonical `d2b` lifecycle group rename.
- [docs/how-to/migrating-from-microvm.md](./docs/how-to/migrating-from-microvm.md)
  — option mapping for users coming from raw microvm.nix
  (scoped to new installs).
- [tests/README.md](./tests/README.md) — full test layering,
  including Layer-2 integration tests.
- [LICENSE](./LICENSE) — Apache-2.0.
