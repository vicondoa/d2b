# Display and virtual I/O capabilities

**Diataxis category:** reference.

Display and virtual I/O are independently advertised capabilities. A
provider must opt into each surface it can actually back; display forwarding
does not imply clipboard, audio, USB, HID, GPU acceleration, or video
decode.

## Capability boundaries

| Capability | Meaning | Notes |
| --- | --- | --- |
| `window-forwarding` | Local semantic Wayland/window forwarding. | Used by local graphics VMs with the Cloud Hypervisor runtime and host-side Wayland mediation. |
| `display-streaming` | Provider/relay display byte stream. | Used when display traffic traverses an authorized gateway stream instead of a local host Wayland socket. |
| `clipboard` | Clipboard bridge. | Separate from display; absent unless explicitly advertised. |
| `audio-playback` / `audio-capture` | Speaker and microphone surfaces. | Separate grants; audio is not implied by display. |
| `usb` / `hid` | Device operations. | Separate from display and from each other. |
| `gpu-accel` | Local/runtime GPU acceleration. | Not automatically relay-exportable. |
| `video` sidecar | H264 decode via media sidecar. | Documented by the video component reference; not a generic runtime fallback. |

The shared `DisplayCapabilitySet` has helpers for the two current display
families:

- `local_wayland()` advertises `window-forwarding`, SHM buffers, and dmabuf.
- `provider_streaming(reconnect)` advertises `display-streaming` only and
  leaves SHM/dmabuf disabled.

These helpers intentionally leave adjacent I/O capabilities absent. Callers
must check the specific capability they need and fail closed when it is not
advertised.

## Managed display-session lifecycle

Gateway-managed provider display sessions use a generation-bound session
ledger. The ledger records non-secret session state:

- session id;
- lifecycle state;
- realm and workload target;
- authorizing operation id and principal;
- owning gateway generation.

The listed principal is derived from the daemon's local socket peer
credentials for the opener, not from relay identity or a caller-supplied
display payload.

The gateway list surface returns only these bounded identifiers and state.
It never exposes session secrets, app argv, Wayland socket paths, relay
endpoints, file descriptors, pidfds, cgroup paths, namespace identifiers, or
process output. Closed and failed sessions are removed from active listings.

The current gateway orchestrator already owns open and close sequencing:
it mints a one-shot display credential, arms the listener before the sandbox
sender connects, spawns the provider agent, waits for the verified handshake,
and invokes the configured listener/provider cleanup hooks for tracked
sessions on close, failed open, and daemon-side stale-session collection.

## Related references

- [Provider-managed sandboxes](./provider-managed-sandboxes.md) documents the
  Azure Container Apps adapter and its absent display/I/O capabilities.
- [Graphics](./components-graphics.md) documents local Wayland forwarding.
- [Video](./components-video.md) documents the video sidecar.
- [Audio](./components-audio.md) documents sound sidecar grants.
- [USBIP](./components-usbip.md) documents USB passthrough.
