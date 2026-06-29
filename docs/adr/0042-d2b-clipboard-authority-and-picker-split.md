# ADR 0042: d2b clipboard authority and picker split

- Status: Accepted
- Date: 2026-06-28
- Related: ADR 0015 (daemon-only clean break), ADR 0023
  (runner-role lifecycle matrix), ADR 0025 (host-jailed Wayland filter
  proxy role), ADR 0032 (d2b v2 constellation control plane), ADR 0034
  (storage lifecycle, restart adoption, and synchronization)

## Context

D2b graphics VMs already route host-visible Wayland traffic through the
broker-spawned `d2b-wayland-proxy` role. That role is the trusted boundary
between untrusted guest Wayland clients and the host compositor: it owns global
filtering, app-id/title rewriting, and the only per-VM access to the host
compositor socket.

Clipboard handling is more sensitive than ordinary window metadata. A normal
Wayland clipboard transfer carries file descriptors and arbitrary user data,
and privileged clipboard-manager protocols can observe and replace host
selection state. If guest clipboard objects are forwarded directly into the
host compositor namespace, a guest app can become a host clipboard owner or
requester outside d2b policy. If a UI picker owns clipboard state directly, GPL
UI code becomes part of the trusted control plane and couples d2b policy to a
specific frontend implementation.

Cursor Clip is a useful UI starting point but it is a full clipboard manager.
It runs a backend that binds `ext_data_control_manager_v1` when available,
falls back to `zwlr_data_control_manager_v1`, reads offered clipboard MIME
payloads into history, takes ownership of the compositor selection with its own
data-control source, and writes stored bytes when another client requests data.
Its frontend talks to that backend over a newline-delimited JSON Unix socket
with commands such as get history, set clipboard by id, pin/delete/clear, and
toggle persistence. For instant paste it can synthesize Ctrl+V through the
Wayland virtual-keyboard protocol. Those backend behaviors are explicitly not
acceptable as d2b picker authority, but the GTK4/Libadwaita/Layer Shell UI,
pointer-positioning, search, thumbnails, keyboard navigation, and row rendering
are useful.

Niri supports the compositor primitives conventional clipboard managers need:
Layer Shell for overlay UI and data-control for clipboard manager behavior.
Niri also exposes a JSON Unix-socket IPC and event stream for compositor state
such as windows, focus, workspaces, keyboard layout, config reload, screenshots,
and screencasts. The public IPC event list does not currently expose a trusted
"this focused client just received paste input" event. Focused-window metadata
is useful labeling context, not proof that a clipboard data request came from a
real Ctrl+V or context-menu Paste operation.

## Decision

D2b will implement clipboard handling as a split-trust architecture:

1. The d2b repository owns all trusted clipboard authority.
2. A separate `d2b-clip-picker` repository owns only the UI picker.
3. The repositories communicate through a small, documented, versioned,
   newline-delimited JSON Unix-socket protocol.
4. The picker protocol is independently implementable and must not depend on
   d2b internal Rust crates.

### Trusted d2b components

D2b adds `d2b-clipd` as the host-session clipboard daemon. It owns:

- host data-control integration;
- direct Niri IPC integration for focused-window labels;
- in-memory clipboard history and payload storage;
- MIME allowlists and size/TTL/memory limits;
- source provenance;
- policy and transfer decisions;
- timeout and cancellation behavior;
- metadata-only audit;
- VM realm identity and lifecycle cleanup;
- picker launch and supervision;
- writes into the already-open Wayland transfer FD after policy recheck.

`d2b-wayland-proxy` becomes the trusted VM clipboard virtualization endpoint.
It must not forward guest `wl_data_device_manager`, `wl_data_device`,
`wl_data_source`, or `wl_data_offer` directly into the host compositor clipboard
namespace. It also continues to deny privileged guest clipboard-manager globals
such as `ext_data_control_manager_v1` and `zwlr_data_control_manager_v1`.
Guest primary-selection protocols are denied in the initial implementation
rather than virtualized, because primary selection changes on ordinary text
highlight and would create high-frequency eager materialization pressure.

For VM copies, the bridge intercepts the guest standard clipboard protocol,
materializes allowed MIME types through the original guest `wl_data_source`,
stores payload bytes in `d2b-clipd`, and installs d2b's virtual selection for
future paste handling while preserving the guest application's normal
selection/highlight behavior. It must not send guest-facing cancellation solely
because d2b eagerly materialized the data; cancellation is sent only when
ownership legitimately changes from the guest client's perspective or protocol
teardown requires it. For VM pastes, the bridge holds the target's transfer FD
and passes it to `d2b-clipd` over an internal d2b channel. The target app's
native paste completes only when `d2b-clipd` writes selected bytes to that FD.
If `d2b-clipd` is unavailable or rejects a transfer, the bridge cancels or
closes the affected Wayland source/offer and still does not forward the guest
clipboard object upstream.

The bridge derives VM identity from the authenticated d2b Wayland bridge
session and VM lifecycle mapping, not from the d2b-prefixed app-id used for
Niri layout and visual rules. VM source and destination attribution is exact to
the guest Wayland client connection, authoritative VM/realm, guest app id, and
optional title.

### Host compositor integration

`d2b-clipd` connects directly to `$NIRI_SOCKET` and speaks Niri JSON IPC. It
must not repeatedly shell out to `niri msg`. It maintains a cache from Niri's
event stream and uses focused-window metadata for host labels: app id, title,
workspace, and output when available.
On host clipboard selection changes, it queries Niri's current focused window as
a fresh attribution sample rather than relying only on the event-stream cache; if
that query fails, attribution is marked cache-stale and remains best-effort.

Host attribution is explicitly best-effort. For host-origin copies,
`d2b-clipd` records the Niri-focused window at the instant the host clipboard
selection changes as `focused_window_guess`, using a fresh focused-window query
when possible. It materializes allowed history representations without
immediately replacing the host selection, preserving same-host rich custom MIME
paste between host apps. It asserts a broker-backed host selection only when
exposing VM data to the host or when the user explicitly selects a historical
item through d2b. For host-destination pastes, `d2b-clipd` records the current
focused Niri window as a destination label, also `focused_window_guess`. D2b must
not present these as exact host client identity unless a future compositor
protocol exposes that identity directly.

Host cross-realm picker display requires a trusted paste-intent token. Niri
focus state alone is insufficient, because a background clipboard probe can
request clipboard data while another window remains focused. D2b will use a
no-patch Niri hook or an upstream-equivalent Niri IPC event if one becomes
available. D2b must not carry a Niri source patch or fork for this feature. If
no no-patch hook can prove native host paste intent without swallowing the
application's normal paste operation, host cross-realm picker popups remain
disabled by default or require an explicit d2b-owned paste action. D2b must not
fall back to Cursor Clip-style virtual-keyboard injection.

The explicit fallback, when enabled, is a two-step native-paste workflow. A Niri
keybind such as `Mod+Shift+V` invokes a d2b clipboard command that opens the
picker and arms the selected entry for the current host-focused target. The user
then performs the normal application paste within a short timeout. `d2b-clipd`
writes into that later native transfer FD. The fallback does not synthesize
input and the picker still never writes a clipboard. The UI must explicitly
guide the user after arming, for example with a content-free
`Ready to paste: press Ctrl+V` banner or notification.
If focus changes before the native paste request arrives, the armed fallback
state is cleared, except for the expected picker-to-target focus restoration:
`d2b-clipd` captures the intended target before showing the picker and ignores
the focus event that returns to that target after picker close. This fallback
still has a residual Wayland limitation: without a trusted no-patch Niri
paste-intent hook, the host data-control source cannot prove the exact requesting
client. The timeout stays short and the hook remains the fully probe-resistant
path.
The armed fallback state is also cleared if a new native clipboard selection
change occurs before paste, because the user may have copied something else.

VM lifecycle cleanup is not inferred only from proxy disconnects. `d2b-clipd`
must receive explicit VM lifecycle events from `d2bd` or an equivalent
user-session notification channel so lock, pause, stop, and destruction can
apply retention and quarantine policy predictably.

### Picker repository

`d2b-clip-picker` is GPL-3.0-only and is forked from `Sirulex/cursor-clip` at a
recorded upstream revision unless the fork cannot be safely reduced to UI-only
while preserving the UX. Recreating the UI is permitted only as a fallback.

The picker owns only:

- GTK4/Libadwaita/Layer Shell presentation;
- pointer-adjacent placement with current-output fallback;
- search;
- thumbnail rendering;
- keyboard and mouse navigation;
- Escape/cancel behavior;
- compact provenance labels;
- the d2b picker protocol client.

The picker runs as a supervised per-request UI process. Its GTK/Libadwaita
application is non-unique so rapid respawns use the inherited socketpair instead
of D-Bus remote activation of a previous instance. The picker exits promptly on
EOF or read/write error on the inherited socket.
`d2b-clipd` enforces at most one active picker globally or per seat and
rate-limits UI-triggering paste requests per VM/realm so one peer cannot hold the
picker slot indefinitely. On timeout, cancel, focus-change cancellation, or
request teardown, `d2b-clipd` terminates the supervised picker process so stale UI
does not remain visible. Candidates marked `confirmation_required` require an
explicit second UI action before Select is sent.
The picker must not receive d2b clipboard transfer FDs. If GTK search widgets
would normally accept pasted text via the compositor clipboard, paste is disabled
for those widgets or search is implemented without binding a clipboard data
device.

The picker must not use `ext-data-control-v1`,
`zwlr-data-control-manager-v1`, `wl-copy`, `wl-paste`, virtual-keyboard
injection, ydotool, clipboard persistence, direct Wayland clipboard ownership,
or any privileged clipboard control. It must never receive clipboard transfer
FDs and must never write a selected item to any clipboard. Selecting an item
sends only `Select { request_id, entry_id }`; closing or pressing Escape sends
only `Cancel { request_id }`.
The picker also does not receive `NIRI_SOCKET`; `d2b-clipd` owns Niri IPC and
passes any Niri-derived labels or placement hints in `OpenRequest` metadata.

D2b does not include the GPL picker as a default flake input or dependency.
Operators install the separate picker flake in their host configuration and
pass its package or binary path to the d2b Nix module.

### Picker protocol

The picker protocol uses newline-delimited JSON over a per-request anonymous
AF_UNIX `socketpair()` inherited by the spawned picker. This intentionally
replaces the earlier named `$XDG_RUNTIME_DIR/d2b/clipd.sock` idea: because
`d2b-clipd` is the picker parent, socketpair authenticity is stronger than a
same-UID pathname socket. No picker launch token is needed on the socketpair
path. If a future side-channel nonce is introduced,
it must use a CSPRNG with at least 256 bits of entropy and must never be passed
through argv or environment because same-UID processes can inspect those through
`/proc`. The picker receives the inherited socket FD number via a non-secret argv
flag such as `--ipc-fd=3`. The parent never clears close-on-exec on the
socketpair FD; inheritance is arranged only in the child between fork and exec,
using `CommandExt::pre_exec`, `command-fds`, or an equivalent safe spawn wrapper.
All unrelated transfer FDs are close-on-exec before the picker is spawned. Any
future stable user-session control socket under
`$XDG_RUNTIME_DIR/d2b/` is separate from the picker protocol and cannot receive
transfer FDs or authorize picker selections.

The internal VM bridge channel is separate from the picker socket. It lives in a
d2b-managed per-user, per-VM runtime path such as
`/run/d2b/clipd/<uid>/bridge/<vm>/clip.sock`, or an equivalent storage-contract
path, with exact ACLs for the host session user and the matching
`d2b-<vm>-wlproxy` principal. Static tmpfiles may create only the stable parent.
Dynamic per-VM directories and default ACLs for `d2b-<vm>-wlproxy` principals are
applied by d2bd/broker lifecycle work before `d2b-clipd` binds the socket, and
are revoked or cleaned during teardown. Parent directories grant required
execute/traverse permission. Recreated sockets inherit the correct ACLs after a
`d2b-clipd` restart. The user service is not expected to create arbitrary
`/run/d2b` parents itself. The per-user component avoids multi-user collisions.
Before binding, `d2b-clipd` unlinks a stale socket path only after verifying it
is a socket inside the declared d2b-owned per-VM bridge directory; `ENOENT` is
success. Path generation must fit Linux `sockaddr_un.sun_path`, using a
hash-shortened VM component when necessary.

The bridge uses local-only Unix-domain sockets with peer-credential validation.
Prefer `SOCK_SEQPACKET` so SCM_RIGHTS descriptors and typed messages preserve
boundaries. Because Tokio does not provide a native `UnixSeqpacket` wrapper, the
implementation creates non-blocking seqpacket sockets with a safe low-level API
such as `rustix` and drives `sendmsg`/`recvmsg` through `AsyncFd` or an
equivalent wrapper. It uses `SCM_RIGHTS` only between d2b components. Received
transfer FDs are wrapped as owned FDs received atomically with close-on-exec,
using `MSG_CMSG_CLOEXEC` or an equivalent memory-safe `recvmsg` path. The
implementation must detect control-message truncation (`MSG_CTRUNC`) and fail
that frame closed after closing any FDs that were partially received in the
returned control buffer. Setting close-on-exec after receipt is not sufficient
because a concurrent picker spawn could inherit the descriptor in the race
window. Transfer FDs are never inherited by the picker.
After a successful SCM_RIGHTS handoff, `d2b-wayland-proxy` closes its local copy
of the transfer FD immediately. On handoff failure it also closes the local FD
while cancelling or closing the relevant Wayland source or offer.

Version 1 messages include:

- `ClientHello` with protocol range and picker version only;
- `OpenRequest` with protocol version, request id, destination metadata,
  requested MIME type, expiry, and filtered candidates;
- `Select` with request id and entry id;
- `Cancel` with request id;
- bounded close/error frames.

Candidates include entry id, source realm, source realm kind, source app,
source app id, source attribution quality, bounded preview text, content type,
timestamp, optional capped PNG thumbnail, and whether confirmation is required.
The picker does not evaluate policy.
The newline-delimited JSON decoder is bounded with an explicit maximum line
length derived from the configured maximum candidate count, metadata size, and
thumbnail-size caps; overlong frames are rejected before deserialization.
Caps are asymmetric: `d2b-clipd` accepts only tiny picker-to-daemon frames such
as Hello, Select, and Cancel, while the picker receive path accepts the larger
bounded `OpenRequest`.
If a large history would make the request frame too large or slow, use a
lightweight `OpenRequest` plus paginated/streamed candidate frames or lazy preview
retrieval.
`d2b-clipd` must not decode, resize, or parse untrusted image payloads to create
thumbnails. The picker must not receive guest-originated raw image previews in
the initial implementation because it runs in the host session and image decoder
bugs would become a VM-to-host escape path. Guest-origin image rows show a
generic image indicator plus safe metadata such as byte count. Future
guest-origin thumbnails are out of scope for the initial implementation. Text
and HTML previews are rendered strictly as plain text: no rich HTML, no Pango
markup, and no remote resource loading. ANSI escape sequences and non-printable
control characters are stripped or escaped before display. Text preview
generation preserves JSON-valid UTF-8 by validating or lossy-converting bytes and
trimming at valid character boundaries. If host-origin image
thumbnails are later enabled, the picker enforces decode dimension and
pixel-count limits to prevent decompression-bomb crashes.

### Payload, policy, and audit

Initial MIME support is limited to `text/plain;charset=utf-8`, `text/plain`,
`text/html`, and `image/png`. Custom MIME types, `text/uri-list`, copied-file
formats, file-manager formats, and `application/octet-stream` are denied. File
transfer remains a separate explicit d2b feature.
Standard secret/password manager hints such as `x-kde-passwordManagerHint` are
recognized from offered MIME names. When present, d2b either bypasses history
storage or stores a non-previewable/masked entry according to policy; it never
shows the secret as preview text.

Clipboard payloads stay in memory by default. D2b provides configurable
per-item limits, MIME-specific limits, total memory caps, TTLs, pending-request
timeouts, and cleanup behavior tied to VM lock, stop, pause, and destruction.
It also caps concurrently held paste transfer FDs globally, per UID, and per VM
to avoid fd-exhaustion attacks, and rate-limits eager copy materialization per
VM/realm and host selection source so a guest or host app cannot continuously
spam selection changes to burn CPU.

Reads from source materialization FDs and writes to target Wayland transfer FDs
are non-blocking and deadline-bound. `d2b-clipd` validates fd type with
`fstat()` before async I/O, accepting pipes/FIFOs and sockets for both source
reads and target writes. Regular files are accepted only when `fstatfs()` or
`statfs()` proves they are memory-backed Wayland transfers such as memfd/tmpfs
or ramfs; disk regular files, block devices, and other fd types are rejected
because `O_NONBLOCK` does not prevent disk-file I/O from blocking executor
threads. Pipes and sockets are explicitly set to non-blocking mode before
registration with the async executor; d2b does not trust peers to have set file
status flags correctly. Memory-backed regular files are handled in short-lived
helper processes that can be killed on deadline, not with unkillable
`spawn_blocking` threads and not by registering regular files with epoll/Tokio
`AsyncFd`. `d2b-clipd` ignores or handles
`SIGPIPE`, maps `EPIPE` to a bounded
closed-FD reason, and closes the target FD immediately on successful write
completion, cancel, timeout, or policy denial so the target observes EOF. A
stalled or malicious peer must not block the daemon event loop.

When held-FD caps are reached, `d2b-clipd` continues draining bridge messages
and immediately drops excess received FDs so descriptors do not remain pinned in
the socket receive queue. The caps are coordinated with `RLIMIT_NOFILE` so they
fire before the process is starved of Niri IPC or bridge sockets.
Internal application-level FD caps stay below `RLIMIT_NOFILE` by a reserved
margin large enough for base daemon sockets plus the maximum FDs one `recvmsg`
can deliver, so the bridge receive loop can always drain and close excess FDs
instead of hitting `EMFILE`.

Audit records contain only metadata: source realm, destination realm, MIME type,
byte count, policy decision, attribution quality, timestamp, request id, and
bounded reason codes. Raw clipboard contents, previews, URLs, HTML, image bytes,
and transfer paths are never logged.
Reason codes are closed and low-cardinality, for example `mime_rejected`,
`policy_denied`, `background_probe`, `intent_missing`, `picker_not_configured`,
`picker_timeout`, `request_expired`, `fd_write_timeout`, `fd_closed`,
`bridge_unavailable`, `source_materialize_timeout`, `memory_cap_exceeded`, and
`loop_suppressed`. Metrics may count decisions using bounded enum labels, but
must not use request ids, window titles, arbitrary app ids, previews, URLs, or
raw payload-derived strings as labels.
Formal security audit events are fail-closed: if the event cannot be queued or
delivered to d2bd, the associated clipboard transfer is denied with
`audit_failure` and the transfer FD is closed without writing. Metrics and
low-priority diagnostics may be dropped with counters.
Fail-closed audit queues are scoped per VM/realm, and high-frequency local
rate-limit hits are coalesced into bounded diagnostics so one malicious VM cannot
starve audit delivery for other realms.

`d2b-clipd` forwards metadata-only audit and low-cardinality metrics to `d2bd`
over a dedicated unprivileged Unix-domain ingestion socket, or an equivalently
isolated public-socket verb class, with peer-credential validation; it does not
own a separate durable audit-log format by default. The forwarding queue is
non-blocking and bounded; overflow of droppable metrics/diagnostics follows an
explicit drop policy and increments a dropped-event counter instead of blocking
clipboard transfers indefinitely.
`d2bd` owns append, rotation, retention, export, and integration with the
existing d2b observability pipeline. Operational logs follow the same redaction
rules as audit. User-visible failures such as a missing picker, picker crash,
timeout, or policy denial may emit bounded desktop notifications through the
standard D-Bus notification service, but those notifications must not include
clipboard content, previews, URLs, raw titles, or transfer paths. They may include
generic realm labels and bounded app-id labels.
Metrics include bounded counters/gauges plus latency histograms for source
materialization, picker interaction, and async FD writes, along with Niri IPC and
bridge connection status/disconnect counters.

Picker runtime responses remain limited to Select and Cancel. Manual deletion of
sensitive entries is provided through d2b-owned management surfaces such as a
future `d2b clipboard history delete <entry-id>` command or trusted management
UI, not through picker history ownership.

Purely same-VM clipboard traffic preserves rich guest desktop semantics where
safe: the proxy can route the original guest source directly to another client in
the same VM without host exposure. D2b MIME filtering, history materialization,
and picker policy apply when data is exposed to host, cross-realm, or stored
history. When another realm legitimately takes ownership, `d2b-clipd` sends a
selection-replaced bridge message so the proxy can deliver guest-facing
`wl_data_source.cancelled` at the correct semantic point.

Drag-and-drop shares the same Wayland data-device interfaces. Initial DND policy
is explicit clean denial for same-VM, cross-realm, and host DND in the clipboard
v1 implementation. Safe DND routing requires compositor pointer/surface state and
policy outside this clipboard ADR. Clipboard virtualization must not silently
forward DND upstream; restoring DND requires a separate ADR and implementation.

Loop suppression is required. When `d2b-clipd` replaces a host or VM selection
with a d2b-owned broker-backed source, it must not ingest that replacement as a
new visible history entry. Use a broker generation id plus MIME set and content
hash or opaque source token to recognize d2b-originated writes.

## Consequences

### Positive

- Trusted clipboard state stays in the Apache-2.0 d2b control plane.
- The GPL picker can evolve independently without becoming a policy engine or
  privileged clipboard manager.
- VM clipboard attribution remains exact and tied to d2b lifecycle identity.
- Host attribution is accurately labeled as best-effort rather than overstated.
- Native paste completion can write into the original Wayland transfer FD rather
  than synthesizing input or writing a second clipboard.

### Negative / trade-offs

- The Wayland proxy must grow real clipboard virtualization instead of simple
  global filtering.
- Host cross-realm native paste depends on a no-patch Niri hook or future
  upstream-equivalent IPC event; without that signal, the secure default is to
  deny picker popups for host background probes.
- A separate picker repository adds release/version coordination.
- Cursor Clip's backend cannot be reused as-is; it must be removed or replaced.

## Non-decisions

- This ADR does not define remote/constellation clipboard transport.
- This ADR does not add file transfer through the clipboard.
- This ADR does not select a persistent encrypted clipboard store; memory-only is
  the default.
- This ADR does not make d2b carry a Niri source patch or fork.
- This ADR does not make `d2b-clip-picker` a default d2b flake input.
