# ADR 0056: Gas City contributor environment

- Status: Accepted
- Date: 2026-08-11
- Partially supersedes: [ADR 0053](0053-gascity-contributor-infrastructure.md)
  for the implemented Gas City environment. ADR 0053's classification of Gas
  City as optional contributor infrastructure and its measured upstream facts
  remain authoritative. This record supersedes the former unimplemented
  implementation shape and records the deployed package choices. It does not
  change any consumer surface.

## Context

Gas City is useful optional infrastructure for contributors who want a
host-native workflow that can turn a bounded change into a reviewable pull
request. It is not a d2b product capability or a consumer workflow. The
implementation must therefore be useful without importing d2b runtime or
release machinery into Gas City's state or execution graph.

The environment also needs a narrow host integration boundary. Gas City needs
host worktrees, Nix, a local control plane, and persistent workflow state, but
model workers and external integrations must not inherit the operator's home,
ambient credentials, or unrestricted network access.

## Decision

### Optional host-native surface

The repository owns a separate, disabled-by-default NixOS module and package
surface:

- `nixosModules.gasCityContributor`
- `packages.<system>.gascity`
- `packages.<system>.gas-city-contributor`
- `devShells.<system>.gas-city`

The module is imported explicitly as
`services.gasCityContributor`. `nixosModules.default`, the d2b consumer
options, consumer examples, overlays, and the root consumer README remain
separate and unchanged. The module runs one host-native lifecycle service and
its sidecars inside `gascity-contributor.slice`; it does not add a container
or a d2b runtime provider.

Mutable Gas City state is outside the Nix store. The immutable city, sanitized
packs, managed instructions, profile settings, scripts, and executable
closure are built into the package. The service owns its own rig, Dolt and
beads state, branches, worktrees, caches, sockets, and status files. A
repository checkout is not used as the service worktree.

### Current-source package and fixed inputs

Gas City is packaged from the current source revision rather than from an
unrelated release binary. The executable inputs are pinned as follows:

| Input | Revision or version | Use |
| --- | --- | --- |
| `gastownhall/gascity` | `6e0399fb970190a35c3e3d5d272a02becec55ffe` | Supervisor, city configuration, workflow engine, and ACP provider support |
| `gastownhall/gascity-packs` | `f3826035bb7de7c34621c2fdcd8620ab5a18bb08` | Compound Engineering, Discord, base, and publication pack sources |
| `numtide/llm-agents.nix` | `387989ee56d550d86d46d9458ad68a55b9e0ca3b` | Copilot CLI package |
| Copilot CLI | `1.0.79` | ACP agent harness |
| package-only nixpkgs | `f13ff45afd1bb73e640eaa08a7066dbed07e3238` | Go 1.26.5 and Bazel 9.1.1 |
| Dolt | `2.1.7` | Gas City state database |
| beads | `bf97b73749ac3ef2fca2365b54537ac041ad4293` | Durable workflow state and conditional updates |

The package runs its applicable upstream checks. A check that cannot run in
the Nix sandbox may be excluded only narrowly and must have an in-repository
replacement or fixture; the exclusion is not a reason to substitute a stale
source package.

### Native Compound Engineering

The city imports Gas City, Compound Engineering, Discord, and the local
contributor pack as sibling imports. It does not nest one composite pack
inside another. The local `d2b-contributor-build` formula extends the native
Compound build. Native Compound planning, review, synthesis, and bounded
fixes remain the workflow authority.

The final comment-resolution seam is intentionally split:

1. a review role makes the judgment;
2. a native `ce-work` step edits the worktree;
3. a separate review or verification role checks the edit; and
4. the native Compound synthesis stage records the result.

This split prevents a mixed review-and-edit role from being treated as
independent review.

### ACP profiles and fallback

Every model-backed role uses a named ACP provider and a dedicated Copilot
profile. The effective bindings are:

| Role | Model | Context | Effort |
| --- | --- | --- | --- |
| Planning | `gpt-5.6-sol` | `long_context` | `xhigh` |
| Review | `gpt-5.6-sol` | `long_context` | `xhigh` |
| Review fallback | `gpt-5.6-luna` | `long_context` | `max` |
| Coding | `gpt-5.6-luna` | `default` | `max` |

Planning and review use Sol first. The Luna review profile is selected only
when the Sol review probe reports `unsupported` or `unavailable`. An
authentication, network, quota, malformed, or unknown failure blocks
readiness; those failures do not authorize a fallback. Coding uses Luna
directly. Model and context come from immutable profile homes and effort is
supplied by the ACP server settings. Gas City uses ACP subprocesses, not the
Copilot TUI, `--agent`, or tmux execution path.

### Service identities and credential sidecars

The system module creates fixed service identities in one contributor slice:

| Identity | UID | Responsibility |
| --- | ---: | --- |
| `gascity` | 45100 | Lifecycle supervisor, durable state, and operator request intake |
| `gascity-agent` | 45101 | ACP launcher and worker process ownership |
| `gascity-discord` | 45102 | Outbound Discord gateway and decision state |
| `gascity-publisher` | 45103 | GitHub App publication and pull-request reconciliation |
| `gascity-egress` | 45104 | Allowlisting egress relay |
| `gascity-check` | 45105 | Uncredentialed local check runner |
| `gascity-buildbuddy-proxy` | 45106 | BuildBuddy credential-injecting Envoy proxy |

The check and BuildBuddy identities are created only when their options
require them. Systemd sidecar units require the egress peer, belong to the
main service lifecycle through `PartOf`, are required by the main service,
and use control-group termination.
The main service waits for the private agent, Discord, publisher, and egress
channels before starting the supervisor.

The module accepts only root-owned absolute credential-source paths outside
the Nix store and unsafe host trees. Systemd `LoadCredential` projects the
Copilot token only to `gascity-agent`, the Discord token only to
`gascity-discord`, the GitHub App private key only to `gascity-publisher`, and
the optional BuildBuddy API key only to
`gascity-buildbuddy-proxy`. The main service receives no token. The module
rejects credential-bearing read-only projections and does not copy tokens into
the repository, package, Copilot home, or ordinary logs.

Local operator control is separate from model workers. Named local users are
added to `gascity-operators` and receive only package-provided, passwordless
sudo rules for `gascity-submit`, `gascity-status`, and `gascity-cancel`.
Requests are size-bounded, identifier-checked, credential-field-free, and
written to a root-owned request directory. ACP workers cannot invoke these
operator wrappers.

### Sandbox, egress, and host projections

The ACP launcher runs workers as `gascity-agent` in bubblewrap user, mount,
PID, IPC, UTS, and network namespaces. A worker receives only its assigned
worktree, immutable runtime closure, managed instructions, approved wrappers,
and a scrubbed environment. It has no direct external interface and cannot
read the Discord, publisher, or BuildBuddy channels.

The worker namespace exposes a local relay backed by a close-on-exec file
descriptor. The egress sidecar is the only peer that opens external
connections. Its allowlist accepts the configured Copilot, Discord, GitHub,
and other explicitly configured domains over HTTPS 443 and rejects arbitrary
public, private, link-local, multicast, metadata, and DNS-rebinding
destinations. The main service itself is restricted to host loopback and the
local Dolt port. There is no public Discord interaction endpoint.

Host projections are explicit read-only paths. They must be absolute,
normalized, distinct by projected basename, and outside home directories,
credentials, the `/etc/nixos` directory itself, broad `/var` or `/run` trees,
sockets, and other protected paths. Specific regular files below
`/etc/nixos` may be allowlisted. They are mounted below the service's host
projection directory, not at their original host paths.

### Resources, storage, and local checks

The default resource boundary is a 100 percent CPU quota, 25 percent
`MemoryHigh`, 30 percent `MemoryMax`, zero swap, `TasksMax=512`, at most two
concurrent ACP agents, one active run, one heavy check, one local Nix job,
and two build cores. The aggregate persistent quota is 250 GiB, with 100 GiB for state and
worktrees, 25 GiB for the
cache, 5 GiB for the publisher, 512 MiB for Discord state, and 100 GiB for
the check runner. The component defaults leave 19.5 GiB of unallocated
headroom within the aggregate limit. The host free-space reserve is 20 GiB. The module requires
project-quota support and rejects a configuration whose component quotas
exceed the aggregate quota. Activation sets
`GC_PROJECT_QUOTA_REQUIRED=1` and `GC_MANUAL_CLEANUP_ONLY=1`.

The optional `gascity-check` service is uncredentialed and uses:

```text
NIX_REMOTE=local?root=/var/lib/gascity-check/nix-root
```

It has no host Nix daemon access and cannot mutate the host store. Builders,
outputs, and check cache remain below the check identity's quota. Nix
substituter and fixed-output fetch traffic uses the same allowlisting relay.
Jobs and requested cores are bounded.

When enabled, BuildBuddy is a separate boundary. Bazel 9.1.1 talks to an
Envoy listener on loopback; Envoy injects `x-buildbuddy-api-key` in memory and
uses HTTP/2 TLS to `remote.buildbuddy.io:443` with fixed SNI and SAN
verification. The uncredentialed check runner and proxy share a private
network namespace, so no other unit can use the proxy listener or observe the
key. The Gas City environment supplies the boundary and fixture checks; it
does not claim that the d2b Rust build has migrated to Bazel.

### Lifecycle, readiness, and restart

`gas-city-contributor.service` is the only lifecycle owner. Systemd restarts
processes; Gas City beads and state own continuation, retry, and progress.
The readiness document records the active city generation, state schema,
effective profiles, and a bounded error code. Readiness fails closed if a
required profile, credential, path, quota, free-space, sidecar, or provider
probe is invalid.

An ACP loss starts a fresh process with the open bead, durable summary,
current branch and commit, worktree state, review state, retry counters, and
next action. Active work remains bound to its compatible city generation and
state schema. Immutable package, city, pack, profile, and instruction
inputs remain Nix GC-rooted for active runs and are removed only by terminal
cleanup. A restart with an incompatible generation or schema blocks with an
actionable migration condition rather than reinterpreting old state.

The service is manual-cleanup-only. It refuses new work before the configured
free-space reserve is consumed. Stopping the service preserves branches,
worktrees, beads, pull-request state, and open pull requests. An operator
must stop the service and identify terminal runs before removing stale cache,
check output, or terminal run roots. Active-run roots and open-run state are
never removed by age-based cleanup.

### Discord decisions

Discord uses an outbound gateway sidecar with a dedicated bot token and a
private decision socket. It is limited to the configured guild, channel, and
operator user IDs. There is no inbound public HTTPS interaction service.

The gate bead contains the decision identity and allowed choices. The first
valid answer wins through a durable conditional transition; duplicate,
late, unauthorized, malformed, or unknown answers do not change the gate.
Acknowledgement and reconciliation are safe to repeat after a provider retry
or service restart. The sidecar does not create a d2b approval ledger,
signature, receipt, seal, or evidence record.

### Pull-request-only publication

Publication uses the separate `gascity-publisher` identity. The main service
passes an unlinked Git bundle file descriptor and bounded metadata over the
private publisher channel. The publisher imports the bundle into its own
bare clone with isolated Git configuration and hooks disabled, pushes the
exact managed branch to the fixed HTTPS repository without force, and then
finds or creates the exact pull request for the fixed base and head.

Push and pull-request mutations are bounded to three attempts and reconcile
ambiguous results before retrying. Repeating publication converges on one
pull request. The publisher can notify Discord after it has the URL, but it
never merges, auto-merges, or enters a merge queue.

### Verification split

Verification has three deliberately different tiers:

1. **Hermetic pull-request checks and pre-PR host integration.** Nix unit,
   package, policy, flake, and changelog checks run in the pull-request
   pipeline. Host integration runs locally before the pull request. Both use
   fake services and no real external
   credential. They check the service contracts, ACP profiles, credential
   boundaries, sandbox, local Nix store, BuildBuddy proxy protocol, and
   forbidden d2b surfaces.
2. **Pre-merge live smoke.** After the implementation pull request opens, an
   operator deploys it to a disposable acceptance repository with temporary
   scoped credentials. The smoke runs one bounded real Copilot ACP request,
   one Discord decision, a restart, a non-force push, pull-request creation
   and repetition, and a pinned Bazel 9.1.1 BuildBuddy cache and
   remote-execution round trip. Real credentials are never prerequisites for
   PR creation and are never committed.
3. **Post-merge rollout acceptance.** The operator confirms readiness and all
   profile routes, then takes three representative projects to reviewable
   pull requests. The operator repeats publication, verifies restart
   reconstruction and no merge, and measures the seeded and clean-state
   BuildBuddy workload for unchanged remote-execution avoidance, bounded CAS
   upload, wall-time improvement, and free-tier headroom.

No live secret, prompt, response, review output, or proof is committed as
verification evidence.

## Explicitly excluded

Gas City does not import, invoke, or wait for repository-only d2b contributor
state. Ordinary Nix and dependency locks remain normal reproducibility inputs.
Automatic pull-request merging is also excluded.

## Consequences

Contributors get a reproducible, optional host-native Gas City environment
with native Compound review, narrow credential sidecars, durable restart
recovery, and a human-owned pull-request merge boundary. The cost is a
NixOS-specific operational surface, explicit credential provisioning, and
manual live and post-merge acceptance. The environment is not a supported
d2b consumer feature and carries no promise that an ordinary d2b run will use
it.
