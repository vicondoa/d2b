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

2. **Containment is a host-eval namespace policy lint, fail-closed.**
   The guest file is evaluated through a hard assertion
   (`assertions.nix` + `lib.nix`'s `guestConfigForbiddenNamespaces`)
   that evaluates the guest file (and its full import closure) with
   `lib.evalModules` over the **real nixpkgs NixOS module set** — so a
   guest module that READS a standard option (e.g.
   `config.networking.hostName` in a `mkIf` guard) resolves instead of
   crashing the host eval — with `microvm` and `nixling` redeclared as
   detector options that nothing else defines. A namespace is reported
   iff `options.<ns>.isDefined`, i.e. by **definition-existence**, not
   by trusting the module system's reported source file. So a guest's
   `imports`, a `builtins.toFile`-generated module, and `_file`
   spoofing are all caught. If the guest file defines ANY option under
   `microvm.*` or `nixling.*`, the host rebuild fails, naming the
   offending option(s). A guest can change its own OS, never the host's
   substrate/framework control of it. Only VMs that set a
   `guestConfigFile` pay this cost.

   **Scope / non-goal (eval-time purity).** This check is a *namespace*
   policy boundary, not an eval-time security sandbox. `lib.evalModules`
   cannot prevent an approved guest file from reading host paths at eval
   time (e.g. `builtins.readFile "/etc/…"`) — and a sound structural
   purity boundary is not achievable for a single module merged into the
   otherwise-impure per-VM `nixosSystem` eval without a larger redesign
   (a restricted/pure evaluator whose normalized output is the only
   thing the host consumes — see Future work). The eval-time exposure is
   therefore bounded by the **operator-review-and-approve trust gate**:
   the host only ever evaluates a guest file the operator has reviewed
   (`config diff`) and explicitly approved, at which point it is trusted,
   operator-reviewed host Nix — no more privileged than config the
   operator writes by hand. The namespace lint guarantees the operator
   cannot *accidentally* let a guest escape into `microvm.*`/`nixling.*`;
   it does not claim to sandbox a malicious approved file's eval-time
   filesystem access.

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
  by (a) the host-eval namespace policy lint (no `microvm.*`/`nixling.*`
  escape), (b) operator review before the host ever evaluates it, and
  (c) the host-operator-only approve step.
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

## Future work

- **Structurally sound eval-time purity.** The namespace lint does not
  constrain a guest file's eval-time filesystem access (see Decision 2,
  scope note). A fully-sound boundary would evaluate the per-VM system
  for `guestConfigFile` VMs under a restricted/pure evaluator and have
  the host consume only that evaluator's normalized output (toplevel
  drv + runner attrs + namespace result), never re-importing the guest
  file impurely. This is a larger change than the namespace lint and is
  deferred; until then, operator review/approve is the eval-purity
  trust boundary.
- **Guest-build mode.** In-guest `nixos-rebuild` for fast local
  iteration (store DB, build users, inputs, closure reconciliation) is a
  separate spike. Durable persistence always flows config → sync →
  approve → host build; the host never ingests guest-built store paths.
