# Tracing contract (bounded-cardinality span attributes)

> **Status**: codified and enforced by the static gate
> `tests/tracing-contract-lint.sh`. Tracked on the
> observability roadmap.

The nixling daemon (`nixlingd`) and the privileged broker
(`nixling-priv-broker`) both emit OpenTelemetry spans and structured
`tracing` events. Those spans flow to the native SigNoz backend through
the `OtelHostBridge` broker role and the `sys-obs` collector. To keep
the backend's ClickHouse cardinality budget bounded **and** to prevent
leaking host-layout identifiers into the observability plane, every
`tracing` macro call in workspace Rust source MUST follow this
allowlist.

## TL;DR

* **Bounded scalar attrs only** on `tracing::{info,warn,error,debug,trace}!`
  and `tracing::span!` calls.
* **Per-bundle / per-store-path identifiers are FORBIDDEN** as span
  attributes — the bundle path is high cardinality (one per
  `/nix/store/<hash>-<name>` realisation) and leaks host layout.
* **Filesystem paths in general** are surfaced via the typed
  [error envelope](./error-codes.md) and the
  [broker audit record](./daemon-api.md) (`OpAuditRecord.tracing_span_id`
  links the audit row back to the span). They MUST NOT appear as span
  attrs.
* **Secrets, argv contents, process env values, cwd/current working directory,
  command output, terminal bytes, helper diagnostics, raw shell names,
  terminal session handles, provider endpoints, provider resource ids, and
  provider credentials** never enter traces.

## Allowed scalar attribute names

| Attribute        | Type / shape                                | Bounded by                          |
| ---------------- | ------------------------------------------- | ----------------------------------- |
| `vm`             | kebab-case `&str` (the VM name)             | per-host VM count (small)           |
| `env`            | enum-like `&str` (e.g. `dev`, `obs`)        | per-host env count (small)          |
| `role`           | `RunnerRole` enum discriminant              | fixed (~10 roles)                   |
| `step_id`        | `HostPrepStepKind` enum discriminant        | fixed (host-prep DAG node count)    |
| `operation`      | broker / daemon op kind discriminant        | fixed (closed-set `BrokerRequest`)  |
| `outcome`        | closed-set string (`ok`, `skipped`, `drift`, `refused`, …) | fixed |
| `error_kind`     | typed [`Kind`](./error-codes.md) enum       | fixed (error-code table)            |
| `op_count`       | numeric (`usize` / `u64`)                   | unbounded scalar (no cardinality blowup) |
| `elapsed_ms`     | numeric (`u64`)                             | numeric scalar                      |
| `parent_pid`     | numeric (`i32` PID)                         | numeric scalar                      |
| `exit`           | numeric (`i32` exit code)                   | numeric scalar                      |
| `load_outcome`   | closed-set `&str` (`ok`, `tampered`, `unavailable`) | fixed                       |
| `reason`         | closed-set typed-error `Reason` enum        | fixed                               |
| `drift_kind`     | typed-error `Reason` enum                   | fixed                               |
| `notify_result`  | closed-set `&str` (`sent`, `skipped`)       | fixed                               |
| `nft`/`route`/`sysctl`/`tap`/`bridge` (intent counts) | numeric            | numeric scalar                      |

Per-VM bounded paths (e.g. `path = %spec.path` for the canonical
ownership matrix, `/var/lib/nixling/state/<vm>/<leaf>`) are tolerated
**only** when the path is statically bounded by the
`(vm, canonical-leaf)` cross product — never an arbitrary `/nix/store`
path or operator-supplied path. New tracing sites SHOULD prefer
adding a bounded enum attr (`target_kind = "sshd-host-keys"`) rather
than emitting the path itself.

## Forbidden patterns (gated by `tests/tracing-contract-lint.sh`)

| Pattern                                                | Why forbidden                                                | Gated since |
| ------------------------------------------------------ | ------------------------------------------------------------ | ----------- |
| `bundle = %X.display()` / `bundle = ?X.display()`      | High cardinality (`/nix/store/<hash>-bundle`); leaks host store layout. | `b6f4ac9` |
| `bundle_path = %X` / `bundle_path = ?X`                | Same as `bundle =`; alias must also be refused.              | `b6f4ac9` |
| `keys_dir = %X.display()`                              | Per-VM sshd state dir — encode via `vm` + bounded `outcome`. | `58aaac8` |
| `path = %X.display()` inside `ssh_host_key_preflight`  | Same path-leak class at `debug!` level.                      | `cbd2169` |
| `/nix/store/...` **string literal** inside a `tracing!` arg | Pins the host store hash into the trace backend.        | `b6f4ac9`  |
| `argv = …`, `cmdline = …`, `command_line = …`          | Argv may contain operator-supplied content / secrets.        | this gate |
| `process_env = …`, `environment = …`, `cwd = …`, `current_working_directory = …` | Process env and working directories may contain secrets or host/user layout. | this gate |
| `secret`, `password`, `token`, `private_key`           | Credential leak.                                             | this gate |
| `provider_endpoint`, `provider_resource_id`, `provider_credential` | Provider endpoints, resource ids, and credentials can identify or authenticate external services. | this gate |
| `stdout = …`, `stderr = …` carrying child-process bytes | Command output not bounded; flows through typed envelope.   | this gate |

## How the contract is enforced

1. **`tests/tracing-contract-lint.sh`** — the static gate. It greps
   workspace Rust source for the forbidden patterns above and fails
   closed if any match. Runs in `tests/static-fast.sh` order alongside
   other drift gates.
2. **Audit-record fallback** — every operator-recoverable detail
   (paths, drift reasons, child stderr) lives in
   `OpAuditRecord.{typed_envelope,tracing_span_id}` (see
   [`daemon-api.md`](./daemon-api.md)), so the operator can pivot from
   a bounded span to the full envelope via `tracing_span_id`.
3. **Code review** — new `tracing!` call sites in PR review are scored
   against this allowlist. Reviewers should reject any per-bundle or
   per-store-path attr and point the author at the bounded-attr +
   audit-record pattern established by the historical closures.

## Worked examples (from the historical closures)

### Broker bundle-load span (canonical reference)

Before (`packages/nixling-priv-broker/src/runtime.rs`, pre-`b6f4ac9`):

```rust
tracing::info!(
    bundle = %bundle_path.display(),     // FORBIDDEN: per-bundle store path
    nft = resolver.nft_intent_ids().count(),
    "Bundle resolver loaded"
);
tracing::error!(
    bundle = %bundle_path.display(),     // FORBIDDEN
    path = %path.display(),              // FORBIDDEN: leaks tampered artifact path
    reason = %reason,
    "Bundle tamper-resistance check failed"
);
```

After (current, post-`b6f4ac9`):

```rust
tracing::info!(
    load_outcome = "ok",                 // bounded closed-set
    nft = resolver.nft_intent_ids().count(),
    "Bundle resolver loaded"
);
tracing::error!(
    load_outcome = "tampered",           // bounded
    reason = %reason,                    // typed-error Reason enum
    "Bundle tamper-resistance check failed"
);
```

The bundle path itself remains operator-recoverable via the typed
`BundleTampered` envelope (exit 60) and the broker audit log.

### ssh-host-key-preflight

The same shape closed the observability findings:
`path = %keys_dir.display()` and `path = %path.display()` inside
`ssh_host_key_preflight.rs` were replaced with
`outcome = "skipped-keys-dir-absent"` / `outcome = "key-entry-ok"`
plus bounded `uid` / `gid` / `mode` numerics. The path remains in the
typed `SshdHostKeyDrift` envelope.

### P2fu1 — bounded `drift_kind`

`observability-r1-1` (P2fu1, `48f4838`) introduced `drift_kind = ?reason`
for the per-VM key drift event. The `drift_kind` is a typed
`SshHostKeyDriftReason` enum — bounded — not an open-ended string.

## Adding new tracing sites

1. Pick attrs from the **Allowed scalar attribute names** table.
2. If you need to surface a path, a bundle, argv, process env, cwd/current
   working directory, provider endpoint, provider credential, provider
   resource id, or child output:
   route it through the typed error envelope
   (`packages/nixling-core/src/error.rs`) and the broker audit log,
   not the span.
3. Run `bash tests/tracing-contract-lint.sh` locally before pushing.
4. If you genuinely need a new bounded attr name, add a row to the
   table above and a corresponding allow in the lint script.
