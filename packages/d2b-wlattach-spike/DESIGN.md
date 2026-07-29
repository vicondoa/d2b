# `d2b-wlattach` design

Condensed from the reviewed plan. This records *why* the shape is what it is;
the full plan (including phase gates and acceptance criteria) lives with the
session that produced it.

## The problem

Wayland object ids are scoped to a `wl_display` connection. When that connection
dies, every object dies with it — surfaces, roles, buffers, callbacks, serials,
registry bindings. Nothing survives. So "reconnect the proxy" is not a thing that
can be done: the persistent side must be a real Wayland **server** owning the
full surface tree, able to materialise it onto a fresh connection on demand.

## Two processes

**Session host** (persistent) — a Smithay `wayland-server`. Owns the
application's connection, the shadow surface tree, the buffer ledger, a synthetic
seat and outputs, and frame pacing. It **never renders**: no EGL, no Vulkan, no
GBM, and no `unsafe`.

**Window frontend** (disposable) — a `wayland-client`. Rebuilt from scratch on
every attach; holds no durable state.

They are *designed* to be connected by an inherited `AF_UNIX`
**`SOCK_SEQPACKET`** socketpair. One datagram carries exactly one frame and its
descriptors, which designs out frame/ancillary desync rather than testing for it,
and it is not a filesystem socket, so no other process can connect and receive
framebuffer descriptors.

> **Status:** the transport is implemented and unit-tested but **not yet the
> live path**. Phase 1 passes shadow state through a file in the mode-0700
> session directory and uses the control socket for close forwarding. The
> seqpacket channel becomes load-bearing when DMA-BUF descriptors need to move.

Because both halves share a machine, DMA-BUF descriptors move by `SCM_RIGHTS`
instead of being serialised — **zero pixel copies on the steady-state path.**

## The buffer ledger

This is the safety-critical core, and it is a pure state machine (`model/`)
holding only opaque ids, reference sets and flags. Adapters translate its
effects into real resource operations. That separation is what makes the
accounting exhaustively testable without a compositor or a GPU.

### Three levels of identity

Conflating any two of these is unsafe:

| Id | What it is |
| --- | --- |
| `BackingId` | the storage (dmabuf planes / shm pool), shareable |
| `AppBufferId` | the application's `wl_buffer` object — **reusable** |
| `BufferUseId` | one attach-to-release **epoch** |

Release is owed **once per epoch**, not once per object. Clients reuse buffers
constantly; owing one release per object stalls any double-buffered client.

An epoch opens on the first attach while the buffer is idle. A further attach
while it is still busy — the same buffer on a second surface, or a reattach
before it drains — **joins** the open epoch. Opening a second epoch would emit a
release while another surface is still reading.

### Four downstream states

| State | Meaning | Awaits |
| --- | --- | --- |
| `Reserved` | sent to the frontend, import outcome unknown | terminal outcome |
| `Imported` | host `wl_buffer` created, **not yet submitted** | commit or abandonment |
| `HostHeld` | frontend declared intent to commit; compositor may read it | `wl_buffer.release` |
| `Quarantined` | orphaned by an unclean exit | nothing |

`Imported` exists because creating a host `wl_buffer` does not mean the
compositor ever read it. Without it, a buffer created and then abandoned leaves
the ledger waiting forever for a release that cannot arrive.

`HostCommitted` is sent *immediately before* `wl_surface.commit`, so the
conservative state is always reached first: a death between the two leaves the
reference already `HostHeld`, and it is quarantined rather than silently lost.

### Quarantine

On an unclean frontend exit the socket queue is drained and applied **first**,
then every unresolved reference — `Reserved` included — becomes `Quarantined`.
`Reserved` is included because "import unconfirmed" never means "import
certainly absent": the frontend may have imported *and* committed before its
report was durably delivered.

**Quarantine is never cleared before session end.** No timer, no heuristic, and
specifically not "a replacement frame was presented" — the compositor may
process the new connection before the old hangup, the replacement may land on a
different output, and the new event refers to an unrelated surface on an
unrelated connection. Elapsed time and unrelated presentations are not evidence.

The honest consequence: graceful `detach` is exact and quarantines nothing. A
*forced kill* is fail-safe but not cost-free — in-flight buffers stay busy for
the rest of the session, and repeated forced kills of an app with a small fixed
pool can stall it. We would rather stall than corrupt.

## Lifecycle

`detach` is not a close. Three distinct ways to leave the attached state:

| Cause | Behaviour |
| --- | --- |
| `detach` | `suspended` + callbacks withheld; graceful drain. **No close sent.** |
| frontend crash | same detached state; quarantine; reported, not hidden |
| compositor close request | `xdg_toplevel.close` forwarded **only** to that toplevel — it is advisory, and the app decides |
| client-drawn X (CSD) | ordinary input; the app destroys its own objects. Not a close event at all. |

The session ends only when the application's connection/process exits.

## While detached

Frame callbacks are withheld and `xdg_toplevel.suspended` is set (version-gated
to clients bound at xdg-shell v6+). The application keeps running — timers,
network and background work all continue — it simply stops drawing, exactly like
a minimised window. `suspended` is advisory, so "zero GPU while detached" is
typical rather than guaranteed.

## Recovery on attach

Surfaces are created parent-first; parents are mapped in topological order and
`set_parent` applied before mapping each child, because `set_parent` against an
unmapped parent is equivalent to null. Effective visible state is then replayed
**descendant-first**, so synchronized child state is latched by the final
ancestor commit.

Each toplevel runs a full configure dance: create, set properties, replay
maximized/fullscreen, commit **with no buffer**, await the compositor's
`configure`, `set_window_geometry`, `ack_configure` with the **new** serial, then
attach the recovery buffer and commit. Old serials are never replayed; the
session host issues its own serial space and never forwards the compositor's.

The retained frame rarely matches the compositor's new configure exactly (a
tiling compositor will size the window differently), so the frontend owns the
surface's single `wp_viewport` — only one may exist per surface — and *composes*
the retained client viewport, buffer scale, transform and the recovery fit,
restoring client state atomically on the app's first normal redraw.

Import uses async `create`, never `create_immed`, whose failure is fatal to the
connection. On failure the surface enters degraded recovery: geometry is
restored, `suspended` cleared, and the window maps on the app's first valid
buffer rather than showing something wrong.
