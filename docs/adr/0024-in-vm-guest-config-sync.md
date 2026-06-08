# ADR 0024: In-VM guest config editing, sync, and containment

- Status: Accepted (Unreleased)
- Date: 2026-06-07
- Related: ADR 0015 (daemon-only clean break), ADR 0017 (no bash
  fallbacks), ADR 0018 (microvm.nix removal — nixling owns the per-VM
  evaluator)

## Context

A VM's NixOS config is host-owned: the operator declares it in
`nixling.vms.<vm>.config`, the host builds it
(`system.build.toplevel`), and the guest boots a read-only closure.
The guest is **untrusted** in the threat model
([`docs/explanation/design.md`](../explanation/design.md) §2); the host
owns the runner substrate (mounts, devices, hypervisor args, kernel,
vsock) end-to-end.

Operators nonetheless want to iterate on what is *installed inside* a
VM from inside the VM, and persist that change on the host. Doing so
naively inverts the trust model: it would let guest-authored bytes flow
into the host's trusted evaluation. We want the ergonomics without the
inversion.

## Decision

1. **Two authorship surfaces, one closure.** The host-owned
   `nixling.vms.<vm>.config` keeps full power (mounts, `microvm.*`,
   `nixling.*`, env, components) and current eval behaviour. A new
   `nixling.vms.<vm>.guestConfigFile` holds the **guest-editable** OS
   layer (packages, services, in-guest users, files). Both merge into
   the one per-VM closure the guest boots.

2. **Containment is a host-eval allowlist, fail-closed.** The guest
   file is evaluated through a hard assertion (`assertions.nix` +
   `lib.nix`'s `guestConfigForbiddenDefs`) that attributes every option
   definition back to its source file via the module system's
   `definitionsWithLocations`. If the guest file defines ANY option
   under `microvm.*` or `nixling.*`, the host rebuild fails, naming the
   offending option(s). A guest can change its own OS, never the host's
   control of it. Only VMs that set a `guestConfigFile` pay this cost.

3. **Canonical config stays in its current host-side location.** No new
   source-of-truth repo. History/rollback come from the operator's
   existing version control of their host config.

4. **Transport is a host-initiated SSH copy — no new attack surface.**
   `nixling config sync <vm>` pulls the guest's edited file over the
   **existing** framework-managed per-VM SSH key + manifest
   `static_ip`/`ssh_user` into a **user-local** staging file. There is
   no virtiofs config share, no new daemon/broker socket, and no
   writable host-backed mount. The guest never initiates a connection
   into the host control plane (the host reaches into the guest). The
   pulled bytes are treated as untrusted data and are never evaluated
   until approved. The guest editable baseline is seeded into the VM
   via the normal read-only closure (`environment.etc` +
   systemd-tmpfiles), not a share.

5. **Review/approve is host-operator-only and never auto-touches the
   config tree.** `config diff` reviews staging vs an operator-named
   live file. `config approve --to <target>` atomically publishes the
   staged copy onto the operator-named target only — the CLI never
   auto-locates or writes the operator's config tree. The authoritative
   containment + eval gate is the `guestConfigFile` assertion that runs
   on the subsequent `nixling switch`; `approve` itself performs only
   light byte validation (non-empty, valid UTF-8).

6. **Guest-built store paths are never trusted.** Durable persistence
   always flows config → sync → approve → host build. An in-guest
   `nixos-rebuild` (guest-build mode) would be fast local iteration
   only; the host never hardlinks/ingests guest-built `/nix/store`
   paths. Guest-build remains a separate future spike.

## Consequences

- The trust direction is preserved: untrusted guest input is contained
  by (a) the host-eval allowlist, (b) operator review before the host
  ever evaluates it, and (c) the host-operator-only approve step.
- No net-new privileged surface is added for the workflow; it reuses
  the per-VM SSH key and the static manifest.
- The fast `assertions-eval` gate is unaffected for VMs without a
  `guestConfigFile` (the containment assertion forces per-VM evaluator
  output only when one is set).
- A malicious or buggy guest edit cannot reach the host's trusted
  evaluation until an operator approves it, and even then is rejected
  fail-closed if it reaches for a host-owned option.

## Alternatives considered

- **Writable virtiofs config share / host `git fetch` of a guest
  repo** — rejected: expands the host's attack surface (writable mount;
  host git parsing attacker-controlled repo data). Replaced by an
  on-demand host-initiated SSH copy of a plain file.
- **A new per-VM git repo under `/var/lib` as source of truth** —
  rejected: the canonical config stays in its current host-side
  location; history lives in the operator's existing VCS.
- **Applying the allowlist to the whole `config` surface** — rejected:
  breaks existing consumers that legitimately set `microvm.*` there.
  The allowlist applies only to the dedicated `guestConfigFile`.
- **Auto-landing approve into the operator's config tree** — rejected
  as a default: the CLI only writes the operator-named `--to` target,
  keeping the tool from silently editing host config.
