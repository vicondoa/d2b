# Gas City contributor environment

> **Optional contributor infrastructure only.** Gas City is not a d2b
> consumer feature and is not the ordinary contributor workflow. Gas City
> runs do **not** use the d2b panel, selected-roster signoff, wave delivery,
> wave sealing, attestation, receipt, merge-eligibility, or bespoke
> evidence-pinning path. Standalone d2b contributors remain governed by
> [`panel-review.md`](./panel-review.md), the wave process, and the other
> d2b contributor contracts.

This guide describes the implemented host-native slice exported by this
repository. It assumes a supported NixOS host and a dedicated repository
configuration. Gas City owns its own rig and worktrees; do not point it at
the maintainer's checkout.

## Import the module

Import the named module from the d2b flake. Do not replace
`nixosModules.default` with it and do not add it to a consumer host merely to
use d2b.

```nix
{
  inputs.d2b.url = "github:vicondoa/d2b";

  outputs = { self, nixpkgs, d2b, ... }: {
    nixosConfigurations.gas-city-host = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        d2b.nixosModules.gasCityContributor
        ({ ... }: {
          services.gasCityContributor = {
            enable = true;
            repository.githubSlug = "OWNER/REPOSITORY";
            repository.baseBranch = "v3";
            repository.rigName = "d2b";
            operators.users = [ "operator" ];
            credentials = {
              copilotTokenFile = "/var/lib/gascity-secrets/copilot-token";
              githubPrivateKeyFile = "/var/lib/gascity-secrets/github-app-key";
              discordBotTokenFile = "/var/lib/gascity-secrets/discord-token";
              buildBuddyApiKeyFile = "/var/lib/gascity-secrets/buildbuddy-key";
            };
            github = {
              appId = "123456";
              installationId = "12345678";
            };
            discord = {
              applicationId = "123456789012345678";
              guildId = "123456789012345678";
              channelId = "123456789012345678";
              operatorUserIds = [ "123456789012345678" ];
            };
            check.enable = true;
            buildBuddy.enable = true;
          };
        })
      ];
    };
  };
}
```

The example uses placeholders. Replace every repository, identifier, and
credential path with host-specific values. `enable = false` is the default
and evaluates without creating the service surface.

## Provision credentials

Create credential files outside the Nix store. Keep them root-owned and
unreadable by ordinary users; systemd loads each file into only its owning
sidecar. Never put a token, private key, or API key in `configuration.nix`,
the flake, a derivation, a Git tree, or a changelog.

```sh
sudo install -d -o root -g root -m 0700 /var/lib/gascity-secrets
sudo install -o root -g root -m 0600 /dev/null \
  /var/lib/gascity-secrets/copilot-token
sudo install -o root -g root -m 0600 /dev/null \
  /var/lib/gascity-secrets/github-app-key
sudo install -o root -g root -m 0600 /dev/null \
  /var/lib/gascity-secrets/discord-token
sudo install -o root -g root -m 0600 /dev/null \
  /var/lib/gascity-secrets/buildbuddy-key
```

Populate the files through a root-only secret-management procedure:

- The Copilot token is read only by `gascity-agent`.
- The GitHub App key is read only by `gascity-publisher`; scope the
  installation to the configured repository and pull-request operations.
- The Discord bot token is read only by `gascity-discord`; restrict the bot to
  the configured guild and channel.
- The BuildBuddy key is optional and is read only by
  `gascity-buildbuddy-proxy`.

The module rejects relative paths, Nix-store paths, unsafe host projections,
and credential-bearing projections. BuildBuddy is disabled unless its key
file is configured.

## Host-specific options

At minimum, an enabled host must set the repository slug and base branch,
one operator user, the three required credential paths, GitHub App IDs,
Discord identifiers and an authorized Discord user. The relevant options
are:

| Option group | Purpose |
| --- | --- |
| `repository.*` | Fixed GitHub `owner/repository`, pull-request base, and Gas City rig |
| `operators.users` | Local users placed in `gascity-operators` |
| `credentials.*` | Root-owned source files projected by systemd |
| `github.*` | GitHub App and installation identifiers |
| `discord.*` | Application, guild, channel, and allowed decision users |
| `hostReadOnlyPaths` | Narrow, read-only host projections |
| `network.allowedDomains` | Exact or left-label wildcard egress destinations |
| `resources.*` | CPU, memory, task, agent, run, check, job, and core bounds |
| `storage.*` | Persistent quotas and the free-space reserve |
| `check.enable` | Uncredentialed local Nix check runner |
| `buildBuddy.enable` | Envoy BuildBuddy boundary; requires its API-key file |

Host projections must be deliberate. The module rejects home directories,
the `/etc/nixos` directory itself, broad `/var` and `/run` trees, sockets,
credentials, and protected kernel paths. Specific regular files below
`/etc/nixos` may be allowlisted. The host filesystem must support project quotas. The defaults are one
active run, two ACP agents, one heavy check, one Nix job, two build cores, a
20 GiB free-space reserve, and an aggregate persistent quota of about 250 GiB.

## Fixed runtime and routing

The closure uses:

- Gas City source `6e0399fb970190a35c3e3d5d272a02becec55ffe`;
- Gas City packs `f3826035bb7de7c34621c2fdcd8620ab5a18bb08`;
- Copilot CLI 1.0.79 from llm-agents.nix;
- Go 1.26.5; and
- Bazel 9.1.1 from the package-only nixpkgs input.

Gas City imports the Gas City, Compound Engineering, Discord, and local
contributor packs as siblings. The local `d2b-contributor-build` formula
keeps native Compound planning, review, synthesis, and bounded fixes. Its
comment-resolution seam separates judgment, native `ce-work` editing,
verification, and synthesis; this is native Compound behavior, not the d2b
panel.

Model-backed roles use ACP profiles:

| Profile | Model | Context | Effort | Use |
| --- | --- | --- | --- | --- |
| `planning-sol` | `gpt-5.6-sol` | `long_context` | `xhigh` | Planning |
| `review-sol` | `gpt-5.6-sol` | `long_context` | `xhigh` | Review |
| `review-luna` | `gpt-5.6-luna` | `long_context` | `max` | Review fallback |
| `code-luna` | `gpt-5.6-luna` | `default` | `max` | Coding |

Luna is the review fallback only for Sol `unsupported` or `unavailable`.
Authentication, network, quota, malformed, and unknown failures block
readiness. Coding uses Luna directly. ACP is headless subprocess execution;
the TUI, `--agent`, and tmux are not part of this environment.

## Identities, sidecars, and boundaries

The service identities are fixed:

| Identity | UID | Credential or responsibility |
| --- | ---: | --- |
| `gascity` | 45100 | Supervisor and durable state |
| `gascity-agent` | 45101 | Copilot ACP launcher |
| `gascity-discord` | 45102 | Discord token and outbound decisions |
| `gascity-publisher` | 45103 | GitHub App key and PR publication |
| `gascity-egress` | 45104 | Egress relay |
| `gascity-check` | 45105 | Local checks without credentials |
| `gascity-buildbuddy-proxy` | 45106 | BuildBuddy API-key proxy |

The main service and sidecars share `gascity-contributor.slice`, but a
same-slice service identity is not adversarial isolation between agents.
ACP workers run as `gascity-agent` in bubblewrap user, mount, PID, IPC, UTS,
and network namespaces. A close-on-exec FD relay is the only path from a
worker to the allowlisting egress sidecar. Workers cannot read Discord,
GitHub, or BuildBuddy credentials and cannot invoke operator wrappers.

The main service requires its sidecars, and each sidecar carries `PartOf` so
stopping the main service drains the complete slice. The main service is
loopback-only except for its private channels. Discord
and GitHub have no public listener. Sidecars use `PartOf` and control-group
termination so a main-service stop drains the related processes.

## Lifecycle and readiness

The lifecycle owner is `gas-city-contributor.service`. It starts the agent,
Discord, publisher, egress, free-space monitor, and optional check and
BuildBuddy services, then waits for their private sockets and readiness probe.
Useful units are:

```text
gas-city-contributor.service
gascity-agent.service
gascity-discord.service
gascity-publisher.service
gascity-egress.service
gascity-check.service
gascity-buildbuddy-proxy.service
gascity-free-space-monitor.service
gascity-contributor.slice
```

Readiness is written to `/run/gascity-contributor/readiness.json`. It records
the city generation, state schema, effective profiles, and a bounded error
code, not credentials or prompts. Readiness blocks on any profile failure
other than the permitted Sol review fallback, on missing credentials or
sidecars, on invalid quotas or paths, and on insufficient free space.

Gas City beads and durable state own continuation and retry. Systemd only
restarts processes. If an ACP process exits, a new process receives the open
bead, summary, branch and commit, worktree state, review state, retry
counters, and next action. Compatible generation and state schema continue;
an incompatible generation or schema blocks rather than silently migrating
old state. Active runs retain their immutable Nix inputs until terminal
cleanup. A service restart preserves branches, worktrees, decisions, and
publication state.

## Submit, status, and cancel

Authorized local users use only the package-provided wrappers. Resolve the
absolute package path used by the installed system before invoking the scoped
sudo rule:

```sh
GC_SUBMIT="$(readlink -f "$(command -v gascity-submit)")"
GC_STATUS="$(readlink -f "$(command -v gascity-status)")"
GC_CANCEL="$(readlink -f "$(command -v gascity-cancel)")"
```

Submit a bounded request with no credential fields:

```sh
printf '%s\n' \
  '{"run_id":"demo-001","bead_id":"demo-001","summary":"Bounded contributor smoke","repository":"OWNER/REPOSITORY","base_branch":"v3"}' |
  sudo -n "$GC_SUBMIT"
```

Inspect the lifecycle service:

```sh
sudo -n "$GC_STATUS"
```

Cancel a run before publication:

```sh
printf '%s\n' '{"run_id":"demo-001","reason":"operator requested cancellation"}' |
  sudo -n "$GC_CANCEL"
```

Identifiers and request size are bounded. A cancellation marker is durable
before the request is advertised, so publication checks it before external
mutations. Cancellation preserves the branch and worktree for inspection.

## Discord decisions

Discord is outbound-only through `gascity-discord.service`. The sidecar
connects to the configured gateway and uses a private socket to deliver
decisions to the supervisor. There is no public HTTPS interaction endpoint.

Only configured users in the configured guild and channel can answer. The
first valid answer wins the durable gate transition. Duplicate, late,
unauthorized, malformed, or unknown answers are no-ops. Reconciliation after
retry or restart is safe to repeat. This is a product-decision channel, not a
d2b panel signoff or an approval receipt.

## Pull-request publication

The publisher receives an unlinked Git bundle file descriptor, imports it
into an isolated publisher-owned bare clone, and pushes the exact managed
branch to the fixed HTTPS repository without force. It then finds or creates
the pull request with the exact head and base. Push and PR operations retry
at most three times and reconcile ambiguous results before retrying.

Publication repetition converges on the same pull request. Gas City may
notify Discord after the URL is known, but it never merges, auto-merges, or
uses a merge queue. A human owns merge approval.

## Local Nix store and BuildBuddy

When `check.enable` is true, check execution runs as the uncredentialed
`gascity-check` identity with:

```text
NIX_REMOTE=local?root=/var/lib/gascity-check/nix-root
```

The host Nix daemon and host `/nix/store` are not used or mutated. Builders,
outputs, and cache stay below the check quota. Nix substituter and fixed-output
fetches go through the allowlisting relay with bounded jobs and cores.

With `buildBuddy.enable = true`, Envoy listens only on loopback at
`127.0.0.1:19801`. It injects `x-buildbuddy-api-key` in memory and speaks
HTTP/2 TLS to `remote.buildbuddy.io:443` with fixed SNI and SAN verification.
The uncredentialed check runner joins the proxy's private namespace. No other
service gets the BuildBuddy key or can use the proxy listener.

The pinned proving workload uses Bazel 9.1.1. It is a boundary and
verification workload, not a claim that the d2b Rust build has moved to
Bazel. No real credential is present in a pull-request gate.

## Diagnostics and restart

Start with the service and readiness state:

```sh
sudo systemctl status gas-city-contributor.service
sudo journalctl -u gas-city-contributor.service -u gascity-agent.service \
  -u gascity-discord.service -u gascity-publisher.service --since today
sudo cat /run/gascity-contributor/readiness.json
sudo -n "$GC_STATUS"
```

Inspect the sidecars and slice when a dependency is not ready:

```sh
sudo systemctl --no-pager --full status \
  gascity-egress.service gascity-check.service \
  gascity-buildbuddy-proxy.service gascity-contributor.slice
sudo systemctl show gas-city-contributor.service \
  -p CPUQuota -p MemoryHigh -p MemoryMax -p MemorySwapMax -p TasksMax
```

The readiness `error_code` distinguishes profile, credential, network,
quota, malformed, and provider failures. Do not copy secrets from
`/run/credentials`, service homes, or sidecar state into a diagnostic.

For a controlled restart:

```sh
# Run this as a root host operator. The gascity-operators group is limited
# to the three package-provided wrappers and is not a general systemctl grant.
sudo systemctl restart gas-city-contributor.service
```

After the restart, check readiness and the run status. The supervisor
reconstructs from beads and durable state; it does not resume an ACP
conversation. If the generation or state schema is incompatible, stop and
follow the reported migration condition instead of deleting state.

## Manual cleanup

The environment sets `GC_MANUAL_CLEANUP_ONLY=1`. It does not delete old
state by age. A free-space refusal is an operator stop signal, not permission
to remove a live run.

A root host operator performs the stop and cleanup; membership in
`gascity-operators` grants only the three bounded wrappers, not general
systemd or filesystem administration.

1. Stop `gas-city-contributor.service` and confirm all sidecars have stopped.
2. Inspect active beads, branches, worktrees, cancellation markers, and open
   pull requests before removing anything.
3. Preserve `/var/lib/gascity-contributor/state`, active worktrees,
   `/var/lib/gascity-publisher`, and any state needed to reconcile an open PR.
4. Remove only an operator-confirmed stale cache or check output below
   `/var/cache/gascity-contributor` or `/var/lib/gascity-check/output`.
5. Remove an active-run Nix root only through the terminal cleanup path after
   the corresponding run is terminal. Never remove roots by age or unlink
   another run's root.
6. Restart the service and confirm readiness before submitting new work.

Do not use `rm -rf` on `/var/lib/gascity-contributor`, `/var/lib/gascity-*`,
or `/run/gascity-contributor` as a cleanup shortcut. Open pull requests and
their branches are not disposable cache.

## Verification and acceptance

### Pull-request and pre-PR gates

Repository gates use fake services and no real external credentials. They
cover module evaluation, package metadata, policy boundaries, native
Compound composition, ACP profile routing, identity and sidecar ownership,
sandbox and egress behavior, local Nix store placement, BuildBuddy proxy
protocol, and the absence of excluded d2b delivery surfaces.

Host integration is also fake-credential and fixture based, but it runs
locally before the pull request because the PR pipeline does not provide the
required NixOS/KVM host. It is evidence of the boundary, not evidence that a
live provider accepted a real mutation.

### Pre-merge live smoke

After the implementation pull request opens, deploy its revision to a
disposable acceptance repository with temporary scoped credentials. Before
human merge, run:

1. one bounded real Copilot ACP request;
2. one Discord decision;
3. a service restart during active work;
4. one non-force push and pull-request creation;
5. publication repetition, confirming the same pull request;
6. the Bazel 9.1.1 fixture through one authenticated BuildBuddy cache and
   remote-execution round trip through the credential proxy and uncredentialed
   check runner.

The smoke credentials are temporary and scoped. No secret, prompt, response,
review output, or proof is committed. This smoke gates human merge; it is not
a prerequisite for opening the implementation pull request.

### Post-merge three-project acceptance

After merge, the operator deploys the named module and confirms:

- readiness selects Sol review, Luna review fallback, and Luna coding with the
  required context and effort;
- one bounded representative change reaches a reviewable pull request;
- one Discord decision resumes the same run;
- a restart preserves beads, branch, commits, worktree, and publication
  reconciliation;
- repeated publication returns the same pull request and never merges; and
- three representative projects independently reach reviewable pull requests.

For the BuildBuddy workload, seed the cache, clear local output state, and
run the same clean commit under the same resource limits. Confirm zero
unchanged cacheable remote executions, bounded unchanged CAS upload excluding
BES metadata, measurable wall-time improvement, and free-tier headroom at
the expected monthly cadence.
