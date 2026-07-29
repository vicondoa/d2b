XX# `d2b-wlattach` — reconnectable Wayland application forwarding (prototype)

*tmux for GUI apps.*

A persistent **session host** owns a real application's Wayland connection and
all of its surface state. A disposable **window frontend** — the process
actually connected to the compositor — can be detached and re-attached at will.
The application keeps running throughout and never reconnects or restarts.

```
detach   window disappears, application keeps running
attach   window returns with the content you left on screen
```

Closing the window closes the application the way that application chooses to;
`detach` is not a close.

## Status

**Prototype / spike.** This crate is deliberately *not* a member of the d2b
workspace and changes no shipping d2b component. It exists to validate an
architecture before any of it is proposed for production.

Phase 0 (foundations) is in progress. See `DESIGN.md` for the architecture and
`docs/p0b-vm-decision.md` for the VM-boundary decision.

## Why not just reconnect the proxy?

Wayland object ids are scoped to a `wl_display` connection. When the compositor
connection dies, **every** object dies with it: surfaces, roles, buffers,
callbacks, serials, registry bindings. Nothing is recoverable from the
connection itself.

So the persistent side cannot be a pass-through proxy pairing client objects to
upstream objects — it has to be a real Wayland **server** that owns the full
surface tree and can materialise it onto a brand-new connection on demand. That
is the whole design.

## Architecture in one picture

```
application
    │  WAYLAND_DISPLAY -> <session dir>/wayland-0
    ▼
session host  (persistent)     Smithay wayland-server. Shadow surface tree,
    │                          buffer ledger, synthetic seat/outputs, frame
    │                          pacing. Never renders. No GPU code. One audited
    │                          unsafe expression, for reading wl_shm (see DESIGN).
    │  Phase 1: shadow state via a file in the mode-0700 session dir.
    │  Planned: inherited SOCK_SEQPACKET socketpair + SCM_RIGHTS (built, tested,
    │  not yet live -- it carries DMA-BUF descriptors when those land).
    ▼
window frontend (disposable)   wayland-client. Rebuilt from scratch every
    │                          generation. Holds no durable state.
    ▼
compositor (niri)
```

Because both halves are on one machine, DMA-BUF descriptors are designed to move
by `SCM_RIGHTS` rather than being serialised, so the session host never imports a
buffer into a GPU context: **zero pixel copies, no EGL/Vulkan/GBM**.

## What is actually implemented today

This is a Phase-0/1 milestone. Implemented and demonstrated:

* the session host, the disposable frontend, and `attach`/`detach`/`ls`/`status`;
* **SHM** content, copied at commit and the application.s buffer released
  immediately, so its pool keeps turning over while detached;
* window reconstruction on a brand-new compositor connection.

**Not yet wired into the live path:** the buffer ledger and the
`SOCK_SEQPACKET` transport are implemented and unit-tested, but the running
system currently passes shadow state through a file in the mode-0700 session
directory and uses the control socket for close forwarding. Both become
load-bearing when DMA-BUF lands. Also absent: input forwarding,
`xdg_toplevel.suspended` and frame pacing, DMA-BUF, subsurfaces and popups.

**On `unsafe`:** the crate is `unsafe_code = "deny"` with exactly one audited
module, `src/serve/sys.rs`. Smithay exposes `wl_shm` contents only as a raw
pointer, so reading them needs one `unsafe` expression. It copies with volatile
reads and never forms a Rust reference over client-writable memory.

## What this prototype does not do

- No clipboard or drag-and-drop (d2b has `d2b-clipd` for that).
- No screen capture, layer-shell, virtual input or foreign-toplevel protocols.
- Exact window *placement* is not restored by the protocol; niri places a
  re-attached window as a new window. Best-effort restore via niri IPC is opt-in.
- A *forced kill* of the frontend is safe but not free — see `DESIGN.md`
  § "Quarantine".

## Layout

| Path | Role |
| --- | --- |
| `src/model/` | Pure state machines. No descriptors, no Wayland types. |
| `src/model/ledger.rs` | The buffer ledger — the safety-critical core. |
| `src/wire/` | Frontend protocol: DTOs and the seqpacket/SCM_RIGHTS transport. |
| `src/probe/` | Phase-0 de-risking probes. |
| `demo/` | Presentation material only. Tests live in `tests/`. |

## Building

The crate needs `libxkbcommon` at link time, and `libgbm`/`pkg-config` for the
probes:

```bash
nix-shell            # provides the above
cargo test --locked
```

**This crate is intentionally outside CI.** It is not a member of
`packages/Cargo.toml`, and it is not wired into `tests/test-rust.sh` or
`tests/test-proofs.sh` — a spike should not consume a shipping gate before it
has proven itself. The existing `packages/d2b-wlproxy-spike` is unwired for the
same reason.

Instead, the following are **mandatory phase-gate evidence**, run locally and
recorded in each phase's review:

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --locked
cargo deny check
cargo audit
```

A phase does not close without them. If this prototype graduates to production,
CI wiring is part of that graduation.
