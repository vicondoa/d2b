# Clipboard architecture

**Diataxis category:** explanation.

D2b clipboard support is a split-trust design. The trusted d2b control plane
owns clipboard authority; the picker is only a UI client.

## Components

| Component | Authority |
| --- | --- |
| `d2b-clipd` | Host-session daemon that owns data-control access, Niri IPC, in-memory payloads, policy, picker supervision, metadata audit, metrics, and writes into Wayland transfer FDs. |
| `d2b-wayland-proxy` | Per-VM Wayland clipboard virtualization endpoint. It derives VM identity from the authenticated bridge session and lifecycle mapping, not from host-visible app-id labels. |
| `d2b-clip-picker` | Separate GPL UI-only picker. It receives display metadata over an inherited socketpair and sends only select/cancel decisions. |

The picker is not a clipboard manager. It must not bind data-control globals,
monitor selections, persist history, receive transfer FDs, write clipboards, or
synthesize input. It also does not receive `NIRI_SOCKET`; `d2b-clipd` owns Niri
IPC and sends only bounded labels and placement hints.

## Host session service

`d2b.site.clipboard.enable` declares a `systemd.user.services.d2b-clipd` unit
for the host Wayland user. The unit is tied to `graphical-session.target`, uses
`AssertEnvironment=WAYLAND_DISPLAY NIRI_SOCKET`, restarts on failure, and owns
only a namespaced user runtime directory (`d2b-clipd`). It does not create
`/run/d2b` parents.

`d2b-clipd` is supplied by package or executable path:

```nix
d2b.site.clipboard = {
  enable = true;
  niri.external = true; # if niri is not declared through programs.niri
  clipd.executablePath = "/run/current-system/sw/bin/d2b-clipd";
  # Or set clipd.package once the d2b-clipd package is wired.
};
```

## Internal bridge sockets

The VM bridge path is a d2b-managed runtime contract:

```text
/run/d2b/clipd/<uid>/bridge/<vm>/clip.sock
```

The stable root and socket name are configurable through
`d2b.site.clipboard.runtime.*`, but the parent directories and ACLs are
provisioned dynamically by `d2bd` and the broker as VMs start, stop, and restart.
Static tmpfiles may create only stable parents in future wiring; they are not
sufficient for per-VM `d2b-<vm>-wlproxy` ACLs or lifecycle cleanup.

The bridge is local-only, validates peer credentials, and may use `SCM_RIGHTS`
between d2b components. Transfer FDs never go to the picker.

## Guest Wayland clipboard virtualization

`d2b-wayland-proxy` always exposes a synthetic `wl_data_device_manager` to
guest clients. It does this even when the host compositor omits the standard
clipboard global, because the guest clipboard namespace is implemented by d2b
and is not inherited from the host compositor. If the host does advertise its
own `wl_data_device_manager`, the proxy hides that host global and keeps guest
`wl_data_*` objects local to the virtual clipboard path.

## Niri and paste intent

`d2b-clipd` connects directly to `$NIRI_SOCKET` and speaks Niri JSON IPC. It
does not shell out to `niri msg`. Focused-window data is labeling context only:
host attribution is recorded as best-effort `focused_window_guess`. Native
clipboard events and explicit operator actions such as `d2b clipboard arm` use
the maintained Niri event-stream cache so the daemon's Wayland event loop does
not block on synchronous compositor IPC.

Host cross-realm native paste requires a trusted no-patch Niri hook or future
upstream-equivalent IPC event. Focus alone is not paste intent. When that hook is
unavailable, operators can enable the explicit fallback: a d2b-owned keybind
opens the picker when one is configured, or directly arms the current
d2b-owned selection when no picker is configured, then the user performs a
normal paste within a short timeout. D2b never compensates by using
virtual-keyboard injection.

## Diagnostics

Clipboard diagnostics are bounded metadata only; raw clipboard contents,
previews, URLs, image bytes, and unbounded titles are never logged. Guest-driven
proxy denials and bridge failures are rate-limited by VM, event, and reason.
Relevant reasons include `connect-failed` and `handoff-failed` for the internal
clipboard bridge, plus picker exits before selection completion. These warnings
are operational signals only; clipboard transfer decisions and byte counts remain
in the structured audit/metrics paths.

`d2b-clip-debug` provides local Wayland probe commands for development and
manual validation:

```bash
d2b-clip-debug wl-copy "hello from this Wayland session"
d2b-clip-debug wl-paste text/plain
```

The probes use only the standard unprivileged Wayland clipboard protocol of the
session they run inside. They do not talk to the picker protocol, do not receive
privileged data-control globals, and do not bypass `d2b-clipd` for VM boundary
transfers.

## Initial limitations

- Primary selection is denied in v1.
- Wayland drag-and-drop is denied in v1.
- File transfer through clipboard MIME types is denied; use a separate file
  transfer feature.
- No remote or relay clipboard transport is declared by this architecture.

## Related references

- [Clipboard picker protocol](./clipboard-picker-protocol.md)
- [Clipboard policy](./clipboard-policy.md)
- [Configure a clipboard picker](../how-to/configure-clipboard-picker.md)
- [Display and virtual I/O capabilities](./display-io-capabilities.md)
