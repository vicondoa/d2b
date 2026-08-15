# ADR 0053: Gas City contributor infrastructure

- **Status:** Accepted
- **Date:** 2026-08-02
- **Amended:** 2026-08-11

## Context

Gas City is optional host-native infrastructure for running bounded
contributor automation against a configured repository. It is not a d2b
consumer feature and is not required for the ordinary d2b build, test, or
release workflow.

The host needs a separate rig, worktree ownership, credentials, egress policy,
resource limits, and publication boundary. The design must keep credentials
away from checks and repository worktrees while allowing controlled model
execution, Discord decisions, GitHub publication, and local Nix validation.

## Decision

Export an opt-in `gasCityContributor` NixOS module. The module is disabled by
default and creates no service surface until explicitly enabled. An enabled
host configures a repository slug and base branch, a rig name, operator users,
credential files, GitHub App identifiers, Discord identifiers, resource limits,
storage quotas, and optional check and BuildBuddy services.

Gas City owns its state, worktrees, and lifecycle. It uses separate service
identities and sidecars:

| Identity | Responsibility |
| --- | --- |
| `gascity` | Supervisor and durable state |
| `gascity-agent` | ACP launcher |
| `gascity-discord` | Outbound Discord decisions |
| `gascity-publisher` | GitHub App and pull-request publication |
| `gascity-egress` | Egress relay |
| `gascity-check` | Uncredentialed local checks |
| `gascity-buildbuddy-proxy` | In-memory BuildBuddy API-key boundary |

The module rejects relative, Nix-store, broad, credential-bearing, socket,
home-directory, and protected-kernel projections. Credential files stay
root-owned outside the Nix store. Each sidecar receives only its own
credential. The check runner uses a private local Nix root and never mutates
the host Nix daemon or host store.

The closure pins Gas City source and packs, Copilot CLI, Go, Bazel, and the
package-only nixpkgs input. ACP profiles provide planning and coding roles
with the configured model and effort policy. The packaged
`pack/scripts/copilot-profile.py` adapter remains the runtime entrypoint for
those profiles; it is Gas City infrastructure, not a repository-level d2b
script.

Discord is outbound-only. Only configured users in the configured guild and
channel can submit a decision. The first valid answer makes a durable
transition; duplicate, late, unauthorized, malformed, and unknown answers
are no-ops. Reconciliation after retry or restart is idempotent.

Publication imports an unlinked Git bundle into an isolated publisher-owned
bare clone and pushes the exact managed branch without force. It then finds or
creates the pull request with the exact head and base. Gas City never merges,
auto-merges, or uses a merge queue; a human owns merge approval.

## Security and operational boundaries

- Host projections are narrow and read-only unless a specific module option
  grants a required path.
- Service identities share a cgroup slice for accounting, not for
  adversarial isolation.
- Egress uses an allowlist and bounded jobs, cores, tasks, memory, and storage.
- BuildBuddy injects its API key only in the proxy and listens on loopback.
- Diagnostics expose readiness and bounded error codes but never copy secrets
  from credential or sidecar state.
- Cancellation is durable before a request is advertised and preserves the
  branch and worktree for inspection.

## Consequences

Gas City is useful for maintainers who need host-native automation without
granting repository checks or model processes broad host authority. It adds
NixOS module surface, sidecars, quotas, pinned upstream inputs, and host
operational procedures. Consumers who do not enable the module incur no
runtime or evaluation cost beyond the exported module.

The implementation details and recovery procedures are in
[`docs/contributing/gas-city.md`](../contributing/gas-city.md). The later
environment amendment in [ADR 0056](0056-gas-city-contributor-environment.md)
records the deployed package, profile, sidecar, and publication choices.
