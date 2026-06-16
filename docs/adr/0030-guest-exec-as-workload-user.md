# ADR 0030: Guest exec runs as the workload user in a PAM login session

- Status: Accepted (Unreleased)
- Date: 2026-06-13
- Related: ADR 0028 (guest-control plane over vsock), ADR 0029 (migrate
  framework SSH operations to typed guest-control RPCs), ADR 0015
  (daemon-only clean break)

## Context

The guest-control plane (ADR 0028) shipped guest exec as **root-only**:
`nixling vm exec` ran the requested command in the guest as `root` with a
cleared environment (`env_clear()` + the client-supplied `--env` pairs only)
and **no PAM session and no login shell**. Non-root exec was deferred to a
future wave behind a `nixling-userd` stub crate and a reserved
`guest.exec.users` option.

That model is wrong for the actual operator workflow, which is to reproduce
what the retired SSH `vm konsole` did: land an interactive or non-interactive
command **as the per-VM workload user** (`nixling.vms.<vm>.ssh.user`, e.g.
`john`) with a real login session. Empirically (verified live on a graphics
VM):

| Mechanism | `XDG_RUNTIME_DIR` | `WAYLAND_DISPLAY` | user |
| --- | --- | --- | --- |
| `su -l john` | empty ✗ | wayland-1 | john |
| `systemd-run --uid=1000 -p PAMName=login -- bash -lc …` | `/run/user/1000` ✓ | wayland-1 ✓ | john |
| `machinectl shell john@.host` | `/run/user/1000` ✓ | wayland-1 ✓ | john |

`su` skips `pam_systemd`, so it never gets an `XDG_RUNTIME_DIR`; a graphical
app then fails with "Missing XDG_RUNTIME_DIR". Only a **real PAM login
session** (the `login` PAM stack includes `pam_systemd.so`) creates
`/run/user/<uid>` and sets `XDG_RUNTIME_DIR`, and `WAYLAND_DISPLAY` is only
exported by the **login shell** sourcing `/etc/set-environment`. So both are
required — exactly the two things SSH used to provide.

Running guest commands as root also broke the privilege model the operator
expects: a guest-control client should never be able to choose the target
user, and the framework should never silently execute attacker-influenced
argv as root.

## Decision

1. **Target user is host-fixed and never root.** `ExecPolicy` carries
   `exec_user: Option<String>` (resolved to uid/gid in the guest), threaded
   from `guest-control.nix` via guestd's `--exec-user <name>` flag, which is
   derived from the per-VM `ssh.user`. Guestd **ignores the wire `user`
   field entirely** — a client cannot escalate to root or target any other
   user. A missing configured workload user fails closed. Eval-time
   assertions reject `guest.exec.enable` without a valid non-root `ssh.user`.

2. **Non-interactive exec runs through a PAM login session.** Guestd spawns
   `systemd-run --uid=<uid> --pipe --wait --quiet --collect
   --expand-environment=no --property=PAMName=login -- <login-shell> -l -c
   'exec "$@"' nl-exec <program> <args…>`. The user argv is passed as shell
   **positional parameters** (never string-joined into `-c`), so an argv
   element that looks like `$VAR`/`$(...)` stays inert; `--expand-environment=no`
   additionally stops systemd expanding variables at unit-load time. The
   login shell sources the user profile (`/etc/set-environment`) before
   `exec`-ing the workload, giving `WAYLAND_DISPLAY`; the PAM login session
   gives `XDG_RUNTIME_DIR`.

3. **Interactive `-it` runs the requested command on guestd's PTY under the
   workload login session.** Like every exec, `-it` requires an explicit
   command after `--` (the CLI rejects an empty command); it runs that
   command on a guestd-allocated PTY inside the workload login session. The
   `vm konsole` replacement is therefore `nixling vm exec -it <vm> -- <shell>`
   (e.g. `-- bash`), which gives an interactive login shell with
   `XDG_RUNTIME_DIR`/`WAYLAND_DISPLAY`/the login profile. (`-i` without `-t`
   is rejected, since guestd forwards stdin only in PTY mode.)

4. **Guestd stays the supervisor; teardown SIGKILLs the named unit's
   cgroup.** The workload runs in a PID 1-owned transient unit, **not** in the
   `systemd-run` wrapper's process group, so killing the wrapper PGID/session
   alone leaves quiet non-TTY commands (e.g. `sleep 3600`) running after a
   host disconnect, `ExecCancel`, or runtime ceiling. Teardown therefore (a)
   issues the local wrapper-PGID / `/proc`-session kill first (always fires,
   even if PID 1 is wedged, and stops a further `StartTransientUnit`), then
   (b) `systemctl --system --kill-whom=all --signal=SIGKILL kill <unit>` on the
   **named** transient unit to reap the whole workload cgroup. The unit name is
   minted per exec (`unique_exec_unit_name`) and passed via `--unit=`; the kill
   is bounded (a wedged PID 1 cannot hang teardown) and retried once to close
   the spawn/teardown registration race.

5. **Removals.** `guest.exec.allowRoot` and `guest.exec.users` are removed
   (hidden tombstone stubs + friendly migration assertions remain so legacy
   assignments fail eval with guidance), along with the `nixling-userd` stub
   crate, the `UserDirectory`/`GuestUserIdentity` scaffolding, and the
   `nixling vm konsole` subcommand (superseded by `vm exec -it`).

## Consequences

- `nixling vm exec` reproduces the old SSH `vm konsole` behaviour without
  SSH: workload user, login profile, `XDG_RUNTIME_DIR`, and `WAYLAND_DISPLAY`,
  so graphical apps launch. Elevation is the user's own `sudo` inside the
  session.
- The capability set is re-gated from `enabled && allow_root` to `enabled &&
  exec_user resolved && required helpers present`, and the host always
  negotiates the output (`EXEC_LOGS`) capability alongside attached exec — a
  session that cannot stream output is never reachable.
- No manifest/bundle schema change: this changes host-owned guestd flags, not
  the per-VM manifest contract.
- The named-unit `systemctl kill` teardown is the load-bearing guarantee that
  a disconnect or cancel cannot strand a workload; it is covered by hermetic
  unit tests (argv exactness + retry-once) and by the live-host validation
  (disconnect a `sleep 3600` and confirm it is gone).
