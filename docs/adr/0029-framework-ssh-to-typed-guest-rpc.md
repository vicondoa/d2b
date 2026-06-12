# ADR 0029: Migrate framework SSH operations to typed guest-control RPCs

- Status: Accepted (Unreleased)
- Date: 2026-06-09
- Related: ADR 0015 (daemon-only clean break), ADR 0024 (in-VM guest
  config editing, sync, and containment), ADR 0028 (guest-control plane
  over vsock)

## Context

Two framework operations historically reached into a running VM over
SSH:

1. **Readiness.** The per-VM DAG gated framework readiness on a raw
   TCP-22 probe (`guest-ssh-readiness`): a VM was "ready" once its sshd
   accepted a connection on the manifest `static_ip`.
2. **`config sync`.** The host pulled the in-guest edited
   `guestConfigFile` by running `ssh <user>@<ip> cat <path>` and
   capturing stdout (ADR 0024).

Both predate the authenticated guest-control plane (ADR 0028). SSH as a
framework transport has three problems in this threat model:

- **It authenticates the wrong thing.** A TCP-22 connect proves a
  listener exists, not that the trusted guest-control endpoint is
  healthy. `accept-new` host-key pinning is first-use-trust, and a
  same-env peer can answer on the static IP before the real guest does.
- **It requires SSH metadata to exist.** `ssh.user` + `static_ip` are
  load-bearing for an operation (config sync) that is conceptually about
  a host-owned file, not about a login session.
- **It is unaudited and free-form.** The host spawns `ssh`/`scp`
  subprocesses outside the broker's typed, audited op surface.

The guest-control plane already provides a mutually-authenticated,
token-bound, versioned ttRPC channel over vsock (ADR 0028) with a
host-side Health probe (the W11 machinery). It is the right substrate
for framework operations that need to reach into a running guest.

## Decision

Framework operations that reach into a running VM migrate from SSH to
typed guest-control RPCs over the authenticated vsock channel. SSH is
retained only as an explicit, clearly-delimited operator convenience
during a compatibility window.

1. **Readiness.** Framework readiness on a guest-control-capable VM is
   the authenticated guest-control Health probe, modelled as
   `ProcessRole::GuestControlHealth` +
   `ReadinessPredicate::GuestControlHealth { vm }`. Unlike a
   `ComponentSpecific` predicate (which reports ready unconditionally
   and would fail OPEN), this predicate **fails CLOSED**: a node is
   ready only when the daemon completes a full Hello + token
   challenge-response + Health and the guest reports `Healthy` or
   `Degraded`. An old-generation / unreachable / auth-failed /
   timed-out / protocol-violating guest is never ready. The raw TCP-22
   `guest-ssh-readiness` DAG node is removed.

2. **Reading guest files.** A new single-shot, bounded, enum-keyed
   `ReadGuestFile` guest RPC (initially keyed to `GuestConfig` only)
   lets the host read a small trusted in-guest file safely. The guest
   resolves the path via `openat` from a trusted directory fd with
   `O_RDONLY | O_CLOEXEC | O_NOFOLLOW`, rejects symlinks / `..` /
   non-regular files, and enforces a size cap **before** allocating. The
   response is bounded below both the ttRPC frame and the `public.sock`
   frame (accounting for base64/JSON overhead). The capability is
   negotiated as `GuestCapability::ReadGuestFile`; an authenticated
   guest that does not advertise it **fails closed** rather than being
   probed. File-specific typed errors (`FileNotFound` / `FileTooLarge` /
   `PathUnsafe` / `ReadDenied`) map to operator-actionable remediations,
   never a blind retry.

3. **`config sync`.** `config sync` reads the in-guest `guestConfigFile`
   via the daemon's authenticated `ReadGuestFile` path instead of SSH.
   The host computes size + SHA-256 from the **received** bytes (the
   guest's self-reported metadata is untrusted) and keeps the existing
   diff / approve / atomic-publish staging model. On a non-guest-control
   (old-generation) VM, `config sync` **fails closed** with a typed
   `guest-control-unavailable-old-generation` error — there is no SSH
   fallback. SSH argv builders are retained only behind a clearly
   delimited compatibility module.

4. **`vm konsole`.** `vm konsole` is **deprecated but functional**. It
   remains the operator SSH convenience until a typed guest-control
   session command lands; nothing is removed yet.

5. **No new framework SSH.** Outside the delimited compatibility module,
   the deprecated `vm konsole` convenience, and the `usb` connect
   `--apply` convenience, the framework does not spawn `ssh`/`scp`. A
   fail-closed source allowlist plus a runtime spawn-instrumented test
   enforce that `config sync` and readiness never spawn SSH on a
   guest-control VM.

## Consequences

- Readiness now reflects a healthy *trusted* guest endpoint, not merely
  an open TCP port, and no longer depends on SSH metadata.
- `config sync` works on VMs with no `ssh.user` / `static_ip`, and a
  hostile or buggy guest cannot influence the host's integrity
  accounting (size / hash are computed from received bytes).
- The SSH attack surface shrinks to explicit, audited-by-comment
  operator conveniences during the compatibility window. A later wave
  removes them once a typed `nixling vm exec` session command lands.
- Old-generation VMs (no guestd) lose framework readiness and
  `config sync` until they are rebuilt onto a guest-control-capable
  generation; this is intentional fail-closed behaviour, surfaced with a
  typed, actionable error.
