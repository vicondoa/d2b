# d2b contributor authority

This file is the single operational authority for ordinary contributors and
coding agents working on **`vicondoa/d2b`**. If you consume d2b in a NixOS host
configuration, start with [`README.md`](README.md).

d2b is an opinionated NixOS desktop microVM framework for a single trusted
Wayland host and untrusted, isolated workloads. Its daemon-only control plane
is `d2bd` plus `d2b-priv-broker`; its declarative modules own networking,
per-VM store views, mediated devices, and versioned bundle contracts.
Product direction is in [`STRATEGY.md`](./STRATEGY.md); the threat model is
in [`docs/explanation/design.md`](./docs/explanation/design.md). Binding
decisions include [ADR 0015](./docs/adr/0015-daemon-only-clean-break.md),
[ADR 0018](./docs/adr/0018-microvm-nix-removal.md), [ADR
0021](./docs/adr/0021-broker-user-namespace-for-virtiofsd.md), [ADR
0032](./docs/adr/0032-d2b-v2-constellation-control-plane.md), [ADR
0034](./docs/adr/0034-storage-lifecycle-restart-and-synchronization.md), and
[ADR 0043](./docs/adr/0043-realm-native-control-plane.md), the accepted
realm-native successor.

Existing code is canon. When a plan, specification, README, or reference
document disagrees with committed, passing code, keep the code and document
the drift. If a load-bearing behavior described here changes, update this file
with the same change.

## Start here

Use this index, then open the focused document instead of expanding this file.

| Task | Authority or detail |
| --- | --- |
| Product purpose and current direction | [`STRATEGY.md`](./STRATEGY.md), [`README.md`](./README.md), [`docs/explanation/design.md`](./docs/explanation/design.md) |
| Add, move, or retire tests | [`tests/AGENTS.md`](./tests/AGENTS.md) |
| Worktrees, review, PRs, merge, and disk hygiene | [`docs/contributing/workflow.md`](./docs/contributing/workflow.md), especially the [reviewed-head lifecycle](./docs/contributing/workflow.md#reviewed-head-pr-lifecycle) |
| Changelog or commit grammar | [`docs/contributing/changelog-and-commits.md`](./docs/contributing/changelog-and-commits.md) |
| Gates and build profiles | [`docs/contributing/gates-and-lints.md`](./docs/contributing/gates-and-lints.md) |
| Architecture and per-VM features | [`docs/contributing/architecture.md`](./docs/contributing/architecture.md) and [ADR 0015](./docs/adr/0015-daemon-only-clean-break.md) |
| Critical subsystem invariants | [`docs/contributing/critical-subsystems.md`](./docs/contributing/critical-subsystems.md) |
| Optional Gas City infrastructure | [`docs/contributing/gas-city.md`](./docs/contributing/gas-city.md) |

## Mandatory contributor-agent workflow

Every code change uses Compound Engineering. Scale planning and orchestration
to the task, but never bypass implementation discipline, independent review,
or the PR tail.

1. **Clear bounded change:** use `ce-work` for the smallest sufficient route.
   Apply Ponytail's minimal-safe-implementation discipline and use Caveman only
   for concise transient coordination.
2. **Open-ended bug:** use `ce-debug` for diagnosis, then `ce-work` for the
   bounded fix. Use `ce-plan` if diagnosis reveals product or scope ambiguity.
3. **Larger or product-ambiguous work:** use `ce-brainstorm`, then `ce-plan`,
   then `ce-work`. Parallelize only genuinely disjoint units with isolated
   worktrees; shared contracts, generated files, or uncertain overlap require
   serial work.

Persisted prose remains normal repository prose. Caveman governs transient
communication only and never creates compressed or otherwise special shipped
documentation.

### Skill roles and model defaults

- Compound Engineering (`ce-work`, `ce-code-review`, `ce-resolve-pr-feedback`,
  `ce-commit-push-pr`, `ce-babysit-pr`, and `ce-simplify-code`) routes, reviews,
  delivers, and watches the work.
- Ponytail keeps implementation minimal, safe, and free of unnecessary
  lifecycle or framework machinery.
- Caveman is for transient communication only; it does not govern persisted
  prose.
- Advanced planning, orchestration, and independent review prefer
  `gpt-5.6-sol` with xhigh reasoning and long context (`long_context`).
- Implementation prefers `gpt-5.6-luna` with xhigh reasoning.
- If a preferred profile is unavailable, use the strongest native
  role-equivalent model; record that substitution only in the transient handoff.
  Do not put model, tool, or agent attribution in shipped artifacts.

### Review and PR contract

The exact d2b Compound Engineering profile is:

```text
ce-work
ce-work mode:return-to-caller <plan-path>
ce-code-review mode:agent
ce-commit-push-pr branding:off babysit:off
ce-babysit-pr posture:target
```

Use bare `ce-work` for a clear bounded change. Use caller mode only when an
outer workflow supplies an implementation-ready plan and owns the shipping
tail.

Every code diff receives independent review in a separate clean context.
The repository-owned caller applies actionable fixes, validates them, and
requests fresh review after every fix or other head-changing update.
Missing review evidence fails closed to fresh review; no actionable finding
remains at merge.

`ce-babysit-pr` watches review feedback, required checks, and head currency.
Immediately before merge, refresh the current reviewed head, required checks,
feedback, mergeability, and observed base. Review evidence binds the
repository, PR, observed base ref and OID, head OID, and verdict. A review fix,
CI fix, push, base update, or missing evidence invalidates readiness and
requires synchronization, validation, and fresh review.

Merge only with a normal squash and an expected-head guard.
Never use admin, auto-merge, bypass, or a merge queue. If the result is
ambiguous, reconcile current PR state before retrying. The accepted workflow
refreshes the base on a best-effort basis and accepts the narrow non-atomic
base race under current non-strict branch settings; it does not change GitHub
settings or claim atomic base binding.

### Worktree, validation, and landing rules

- Use a new isolated worktree by default for feature work and parallel scopes.
  Never merge a slice directly to protected `main` or `v3`.
- Commit before authoritative validation so tracked inputs are visible to Nix
  evaluation. This task's orchestrator owns the canonical commit.
- Run the smallest focused validation that covers the changed surface, then
  the required gates when the owning workflow calls for them. Read
  [`tests/AGENTS.md`](./tests/AGENTS.md) before changing test coverage.
- Use the top-level Makefile and existing gates: `make check` is the aggregate,
  `make test-unit` is the Layer-1 development umbrella, and
  `make test-integration` adds the conditional container lane. Do not cite an
  advisory skip as validation evidence.
- Every code change ships a valid changelog entry or a fragment under
  [`changelog.d/`](./changelog.d/).
- Leave `nix/gas-city-contributor/**` and its managed authority unchanged;
  ordinary repo skill policy makes no visibility claim for managed sessions.
- `main` and `v3` are protected and land only through reviewed pull requests.
  Use short imperative area-prefixed commit subjects and no AI, tool, or model
  attribution.
- Never use destructive `git checkout --` or `git restore` on unowned paths,
  package-wide formatters, or `git add -A` while a gate is running. Put
  throwaway files in `.scratch/`, and use the documented disk hygiene rules.

## Critical subsystem index

`make check` invokes the public Bazel suite facade and its nested Layer-1
component and package suites. Bare local runs use the BuildBuddy profile for
eligible actions and automatically fall back to local execution when no
credential is available; CI runs the same suite graph through the local profile
with no BuildBuddy credential. The separate root `buildbuddy.yaml` gives
BuildBuddy Workflows ownership of the protected `v3` `build / check`; its
hosted Ubuntu 22.04 runner executes the remote-compatible fixed Layer-1
selection locally, without nesting the RBE profile or using a GitHub
secret-bearing proxy. The GitHub Actions workflow remains the credential-free
local check. Make aliases are thin facade entry points, while Cargo manifests
and lockfiles remain rules_rs metadata authority rather than contributor
workflow entry points.

The full invariants are in
[`docs/contributing/critical-subsystems.md`](./docs/contributing/critical-subsystems.md).
Read the relevant section before changing any of these:

- networking and firewall neutralization; per-VM closure-only `/nix/store`;
  TPM persistence; USBIP; GPU and video sidecars; audio; UI color contract;
- daemon and broker control plane; manifest and private bundle contracts;
  storage lifecycle, restart adoption, synchronization, and the **single
  repair owner** rule;
- capability/session and zone-bus boundaries; resource mutation seals;
  authoritative subject resolution; controller effects;
- unsafe-local providers and shells; lifecycle group authorization; SSH keys;
  virtiofsd user-namespace sandboxing; eval-time assertions.

## Don'ts (security-relevant)

- Do not remove the net VM's `lib.mkForce` neutralizer for `10-eth-dhcp`;
  validate `net.nix` against `tests/unit/nix/cases/net-vm-network.nix`.
- Do not relax VM-name validation or reserved `sys-*` and `launcher` prefixes.
- Do not silently break the manifest: update schema, prose, emitter,
  `manifestVersion`, and changelog together. Do not hide a failing assertion
  by deleting it; fix its predicate or message.
- Do not reintroduce per-VM systemd units, host-singleton framework services,
  or the retired bash CLI fallback. Lifecycle stays in `d2bd` and privileged
  mutation uses typed broker operations. Retired knobs
  `D2B_LEGACY_BASH_OPT_IN`, `D2B_LEGACY_CLI`, and `D2B_NATIVE_ONLY` are no-ops.
- Do not commit secrets, hostnames, real user identifiers, or real network
  ranges. Use generic names and RFC1918 or RFC5737 examples.
- Do not add a linter, formatter, pre-commit hook, new overlay, or casual
  `nixpkgs.url` change. Use existing gates and tooling.
- Do not leak revision, follow-up, or finding markers into shipped source,
  docs, CLI text, CI names, changelogs, commit messages, or PR bodies.
- Spell dashes with ASCII `-` only. The non-ASCII prohibition includes
  U+2010, U+2011, U+2012, U+2013, U+2014, U+2015, U+2212, U+FE58, and
  U+FF0D. Tests that need one use an escape such as `"\u{2014}"`.
- Do not let the host hold realm credentials, remote node registries, provider
  configuration, or realm audit. Keep them in the per-realm gateway guest;
  relay identity is never local auth, and work and personal realms never share
  a gateway guest or L2 bridge.
- Do not add ad-hoc storage, ACL, cleanup, or lock ownership. Follow ADR 0034:
  broker-resolved opaque ids, anchored paths, `O_CLOEXEC` OFD locks, explicit
  fd transfer, restart adoption before cleanup, typed degraded state, and a
  named **single repair owner** for every host-mutable path or lock surface.
  Never add broad chmod, chown, setfacl, or `/run/d2b` sweeps.
- Do not mutate host state outside its ownership marker or continue past a
  foreign marker. Preserve foreign nftables byte for byte; use the
  `d2b managed: <ownership-id>` comment and the
  `# d2b-managed begin` / `# d2b-managed end` delimiters for hosts and
  NetworkManager. systemd-networkd is detection-only. Foreign markers fail
  closed and never authorize overwrite.
- Do not mutate d2b cgroups outside delegation: use
  `/sys/fs/cgroup/d2b.slice/<vm>/<role>/` leaves, no threaded groups,
  partition roots, `cpuset.cpus.partition`, parent `cgroup.kill`, or root
  chown; only the broker owns delegated root mutation after privilege drop.
- Do not commit or attach unredacted screenshots. Remove secrets, credentials,
  tokens, PII, host paths, internal node names, and realm principals; describe
  the result in text if redaction would destroy its meaning.

## Daemon-only end-state (P6 onward)

d2b declares exactly three root-visible units: `d2bd.service`,
`d2b-priv-broker.socket`, and `d2b-priv-broker.service`. The binding decision
is [ADR 0015](./docs/adr/0015-daemon-only-clean-break.md).

- `d2bd` supervises every per-VM DAG. The broker dispatches audited host
  mutations and launches runners through `SpawnRunner`, returning pidfds.
- No framework-declared per-VM systemd units or host-singleton framework
  services exist. The Rust `d2b` binary is the only CLI surface.
- The retired bash fallback and legacy environment knobs are removed or
  no-ops. Lifecycle authorization is `d2b` group membership plus
  `SO_PEERCRED` at `public.sock` accept time.
- Repository-wide policy is limited to source hygiene, workspace and lock
  integrity, supply chain, and changelog policy; security-critical behavior
  remains owner-local or structural.

## References

- Consumer entry point: [`README.md`](README.md)
- Product direction: [`STRATEGY.md`](./STRATEGY.md)
- Security disclosure: [`SECURITY.md`](./SECURITY.md)
- Contributor detail: [`docs/contributing/`](./docs/contributing/)
- Test model: [`tests/AGENTS.md`](./tests/AGENTS.md)
- Manifest and daemon contracts:
  [`docs/reference/manifest-schema.md`](./docs/reference/manifest-schema.md),
  [`docs/reference/daemon-api.md`](./docs/reference/daemon-api.md),
  [`docs/reference/privileges.md`](./docs/reference/privileges.md)
- Lifecycle explanation:
  [`docs/explanation/daemon-lifecycle.md`](./docs/explanation/daemon-lifecycle.md)
- Naming glossary:
  [`docs/reference/naming-conventions.md`](./docs/reference/naming-conventions.md)
- License: [`LICENSE`](./LICENSE)
