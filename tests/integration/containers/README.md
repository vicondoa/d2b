# Container integration tests (podman)

A small, deliberately narrow tier of integration tests that run a Nix-built
OCI image under **podman**. Each test is a standalone `tests/integration/containers/*.sh`
script that sources `lib.sh`, builds its image
(`containerImages.<system>.<name>`, auto-discovered from
`tests/integration/containers/images/*.nix`), loads it into podman, runs it, and asserts.

Run them with `make test-integration` on the development host before opening
an agent-owned PR. The host must have podman available, or `lib.sh`
bootstraps it via `nix shell nixpkgs#podman`.

## What this tier is for

**Only** scenarios that genuinely need a *foreign, non-Nix userland* — things
that cannot be proven by a native Rust test or a nix eval. Today that is one
case:

- `ubuntu-host-check` — proves a statically-linked nixling binary
  (`nixling-guestd-static`) runs on a stock `ubuntu:24.04` userland, i.e. the
  guest-side binary is portable to the distros nixling targets.

## What this tier is deliberately NOT for

**Booting systemd to exercise daemon / socket activation.** That coverage
already exists, far more cheaply and without containers:

| What | Where | Cost |
| --- | --- | --- |
| Broker adopts the socket-activated fd (`LISTEN_FDS` fd-3 handoff) + serves a Hello round-trip | `packages/nixling-priv-broker/tests/socket_activation.rs` | ~0.4 s, unprivileged |
| Daemon binds `public.sock`, serves Hello/vmStart, `SO_PEERCRED` authz, writes the version file | `packages/nixlingd/tests/daemon_*.rs` | native, hermetic |
| Unit shape + `Wants=`/ordering, broker capability set, tmpfiles, evidence-record shape | `tests/unit/nix/cases/{broker-socket-activation,nixlingd-startup-smoke}.nix` | nix eval, fail-closed |

A faithful systemd-boot container was built and **measured**: even forcing
`pkgs.systemdMinimal`, the image stays **~1.4 G** to ship. The bulk is not
systemd — it is the nixling bundle under test, whose process descriptors
transitively reference the full per-VM runtime substrate (swtpm → tpm2-tss,
the hypervisor, virtiofsd, store-overlay → cryptsetup, full systemd). On an
Ubuntu CI runner with no `/nix/store`, that 1.4 G must be built-or-fetched
every run, and the boot needs privileged podman + `sudo` — all for **zero**
marginal coverage over the native tests above.

So: do not add a systemd-boot / privileged-podman container here. If you think
you need one, the thing you actually want is almost certainly a native
integration test against `CARGO_BIN_EXE_*` (see `socket_activation.rs` for the
fd-passing pattern) or a nix-unit case.
