# d2b CLI contract

**Diataxis category:** reference.

The Rust `d2b` binary is the only operator surface. It addresses the local
Zone resource plane through `d2bd`; privileged host effects are reached only
through the typed broker path selected by the daemon.

## Model

The CLI uses typed ResourceRefs:

```text
Zone/work
Guest/work-app
Process/work-app-worker
EphemeralProcess/exec-1
Provider/runtime-cloud-hypervisor
```

`--zone <name>` selects the Zone route for a request. It is a route assertion,
not a permission grant or an alternate store. The daemon authenticates the
peer, checks the Zone session, validates Resource UID/generation/revision and
Provider capability, and then dispatches the operation.

Nix authors a Guest's semantic spec and selected immutable artifacts. The
Guest controller derives and observes its child Resources. The CLI never
reconstructs a child graph from a manifest, starts a process directly, or
falls back to SSH or a legacy process table.

## Common conventions

- `--json` emits one newline-terminated JSON document on stdout.
- `--human` forces terminal-oriented output.
- Lifecycle mutations require `--dry-run` or `--apply` where the command
  supports both.
- `--deadline` bounds daemon requests and streams.
- Invalid ResourceRefs, unknown resource kinds, stale generations, missing
  capabilities, and unauthorized callers fail closed with typed errors.
- Retries use the operation identity and are safe to repeat.
- Public output never contains credentials, raw host paths, argv, pidfds,
  cgroup paths, namespace identifiers, or private runtime locators.

## Resource commands

The generic verbs operate on one ResourceType:

```text
d2b get <TYPE>/<name> [--zone <zone>]
d2b list <TYPE> [--zone <zone>]
d2b watch <TYPE> [--zone <zone>]
d2b create <TYPE> --spec-file <path> [--zone <zone>]
d2b update-spec <TYPE>/<name> --spec-file <path> [--zone <zone>]
d2b delete <TYPE>/<name> [--zone <zone>] --apply
d2b status <TYPE>/<name> [--zone <zone>]
d2b upgrade <TYPE>/<name> [--zone <zone>] --apply
d2b reconcile <TYPE>/<name> [--zone <zone>] --apply
```

Typed nouns are convenience forms over the same Resource API:

```text
d2b zone get|list|status ...
d2b guest get|list|status|start|stop|restart|create|update-spec|delete ...
d2b process get|list|status|start|stop|create|update-spec|delete ...
d2b provider get|list|status|inspect ...
d2b host get|list|status|check|doctor|prepare|destroy|reconcile ...
d2b volume ...
d2b network ...
d2b device ...
d2b endpoint ...
d2b export ...
d2b import ...
d2b user ...
d2b credential ...
d2b quota ...
d2b emergency-policy ...
```

For `guest` and `process` lifecycle verbs, `<name>` is resolved within the
selected Zone as `Guest/<name>` or `Process/<name>`. A missing `--zone` uses
the nearest local runtime when one is available.

## Guest lifecycle

```text
d2b guest start <name> --zone <zone> --dry-run
d2b guest start <name> --zone <zone> --apply
d2b guest stop <name> --zone <zone> --apply
d2b guest restart <name> --zone <zone> --apply
d2b guest status <name> --zone <zone>
```

Start and restart wait for the current Guest generation and its required
children unless `--no-wait-ready` is supplied. Stop is drain-oriented and
does not clear the Guest finalizer until the controller proves that owned
descendants and the authenticated Guest session are gone. `--force` remains a
typed escalation request; it does not bypass identity or ownership checks.

Guest readiness is status-first. Pending dependencies, stale Provider
assignments, session loss, uncertain broker responses, and blocked finalization
remain typed states rather than being hidden by a second lifecycle authority.

## Guest execution

```text
d2b exec run Guest/<name> -- /bin/sh
d2b exec run Host/<name> -- /usr/bin/id
d2b exec attach EphemeralProcess/<name>
d2b exec wait EphemeralProcess/<name>
d2b exec status EphemeralProcess/<name> --watch
d2b exec list Guest/<name>
d2b exec logs EphemeralProcess/<name> --stdout-offset 0 --max-len 4096
d2b exec kill EphemeralProcess/<name>
```

`exec run` creates an `EphemeralProcess` Resource. `--user` and `--provider`
accept typed ResourceRefs; `--env` and `--cwd` are bounded and validated.
The selected Process Provider owns command execution, retention, cancellation,
and restart adoption. Detached output is a bounded Resource projection.

Interactive attach uses `--interactive` and `--tty`; it is human-only and
restores host terminal state on every exit path. No command form accepts a
caller-supplied host process, shell fallback, or private locator.

## Shells

```text
d2b shell open Guest/<name> --name build
d2b shell attach ShellSession/build
d2b shell list --zone <zone>
d2b shell status ShellSession/build
d2b shell detach ShellSession/build --apply
d2b shell kill ShellSession/build --apply
```

Shell sessions are `ShellSession` resources with bounded names and lifecycle
state. Guest shells use the authenticated ComponentSession; Host shells use the
approved unsafe-local Provider path. Neither path creates a framework-owned
per-Guest service.

## Activation

Activation is an operation on a typed Guest target:

```text
d2b activation build Guest/<name>
d2b activation switch Guest/<name> --apply
d2b activation test Guest/<name> --apply
d2b activation rollback Guest/<name> --apply
d2b activation generations Guest/<name>
d2b activation gc --apply
d2b activation migrate --apply
d2b activation keys list
d2b activation keys show Guest/<name>
d2b activation keys rotate Guest/<name> --apply
d2b activation trust <name> --apply
```

The activation Provider prepares and publishes the selected Guest system
artifact. Live activation occurs only through the authenticated Guest session;
offline staging changes the next-start selection without executing a host-side
Guest command.

## Host and observability commands

```text
d2b host check
d2b host doctor --read-only
d2b host prepare --apply
d2b host destroy --apply
d2b host reconcile --apply
d2b audit --json
d2b op inspect --json
d2b auth status --json
```

Host commands report broker, daemon, ownership-marker, and Provider status.
They do not mutate foreign state. Audit and operation inspection expose typed,
redacted observations only.

## Provider projections

The `audio`, `clipboard`, and `display` namespaces are Provider projections.
They resolve the owning Zone and ResourceRefs through the daemon; they do not
carry host socket paths or direct effect arguments. A Provider that is not
installed, ready, or authorized returns a typed failure.

## Stable JSON schemas

Generated schemas live in [`cli-output/`](./cli-output/):

| Surface | Schema |
| --- | --- |
| `list` | [`list.schema.json`](./cli-output/list.schema.json) |
| `status` | [`status.schema.json`](./cli-output/status.schema.json) |
| `usb probe` | [`usb-probe.schema.json`](./cli-output/usb-probe.schema.json) |
| `audit` | [`audit.schema.json`](./cli-output/audit.schema.json) |
| `host check` | [`host-check.schema.json`](./cli-output/host-check.schema.json) |
| `host doctor` | [`host-doctor.schema.json`](./cli-output/host-doctor.schema.json) |
| `auth status` | [`auth-status.schema.json`](./cli-output/auth-status.schema.json) |
| `op inspect` | [`op-inspect.schema.json`](./cli-output/op-inspect.schema.json) |
| `store verify` | [`store-verify.schema.json`](./cli-output/store-verify.schema.json) |

The schema generator is `bazel run //packages/xtask:xtask -- gen-cli-schemas`.
The drift target is `//packages/xtask:gen_cli_schemas_drift`.

## Errors

Typed errors include the owning command, stable kind, exit code, redacted
message, and remediation when one is safe to disclose. Common classes are:

| Exit | Meaning |
| --- | --- |
| `0` | Operation completed or an idempotent result was returned. |
| `2` | Invalid command, argument, or ResourceRef. |
| `31` / `75` | Authorization, capacity, or temporary operation conflict. |
| `69` | Zone, Provider, or transport prerequisite unavailable. |
| `70` | Capability or version mismatch. |
| `76` | Protocol, revision, or retained-output conflict. |
| `77` | Session, identity, or authorization evidence rejected. |
| `78` | A typed native handler or broker operation failed closed. |

See [`error-codes.md`](./error-codes.md) for the generated complete table.

## Historical material

Older Realm, environment, VM-first, Gateway-daemon, and bash-fallback command
forms are retained only in historical ADRs and migration notes. They are not
part of the current parser or lifecycle contract. When historical documents
conflict with the current Zone resource model, the current code and the
references linked from the repository README are authoritative.
