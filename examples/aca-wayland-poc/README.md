# ACA + Wayland forwarding POC (ADR 0032)

> **Status: proof-of-concept.** This directory is the hard vertical slice
> behind ADR 0032 — it proves a Wayland-native app can run inside an
> **Azure Container Apps sandbox** and be rendered on the operator's
> compositor with the display carried over an **Azure Relay** hybrid
> connection. The production realm-gateway design (jailed broker
> `SpawnRunner` display runners, gateway-minted per-session credentials,
> authenticated constellation handshake, stream mux + audit) is described in
> the plan and ADR 0032; this POC uses upstream Waypipe and a minimal relay
> bridge to demonstrate feasibility end to end.

## What it demonstrates

A `foot` terminal (Wayland-native, no XWayland) running in an ACA sandbox,
visible live on the host niri compositor, every display byte tunneled over
Azure Relay:

```
foot (ACA sandbox)
  → waypipe --no-gpu server (SHM-only)
  → /run/nixling/wp.sock
  ← nixling-relay-bridge send (unix-listen target; trusts the ACA
      egress-proxy CA; outbound, SAS-authed WebSocket)
  → Azure Relay hybrid connection (hc-nixling-display)
  → nixling-relay-bridge listen (host)
  → /run/user/<uid>/wpc.sock
  → waypipe --no-gpu client (host)
  → niri
```

## Layout

| Path | What |
| --- | --- |
| `azure/` | Bicep (CAF-aligned): ACR, managed identity, **sandbox group** (`Microsoft.App/sandboxGroups`), Relay namespace + hybrid connection + scoped Listen/Send SAS rules. |
| `container/` | `image.nix` (Nix-built OCI image: waypipe + foot + the relay bridge + the agent), `bridge/nixling-sandbox-agent.sh` (in-sandbox entrypoint), `build-and-push.sh`. |
| `relay-bridge/` | `nixling-relay-bridge` — a small Rust tunnel that bridges a local socket to an Azure Relay hybrid connection (sender role in the sandbox, listener role on the host). Trusts the sandbox egress-proxy CA via `--ca-file`. |
| `host/` | `run-host-display.sh` — brings up the host `waypipe client` + relay listener (POC stand-in for the gateway's host-side display runner). |
| `tests/` | `live-demo-checklist.md` — the manual, Layer-2 reproduce-and-verify procedure. |

## Run it

See [`tests/live-demo-checklist.md`](./tests/live-demo-checklist.md) for the
full, copy-pasteable procedure. In short: deploy the Bicep, build+push the
image, register a sandbox disk + create a sandbox, start the host receivers
(`host/run-host-display.sh up`), then run `nixling-sandbox-agent` in the
sandbox via `aca sandbox exec`. **Delete the sandbox when done.**

## Trust-boundary notes (ADR 0032)

- The container holds **no** long-lived Azure credential — only a
  short-lived, least-privilege Relay **Send** SAS token. The host holds the
  **Listen** key; neither side is the authenticated constellation principal
  (the relay only grants reachability).
- The display path is **SHM-only** (`waypipe --no-gpu`, no DMABUF) — ACA
  sandboxes have no host GPU.
- ACA sandboxes terminate egress TLS with a transparent proxy; the relay
  bridge trusts that proxy CA explicitly rather than disabling verification.

## Known POC limitations (hardened in later ADR 0032 waves)

- SAS keys are passed through env/CLI for the demo; the production design
  delivers gateway-minted, expiring, per-session tokens via a sealed
  enrollment path.
- No authenticated constellation handshake / per-operation authz / stream
  mux / gateway audit yet — those are later ADR 0032 work, not this
  feasibility bridge.
- The relay bridge is a raw byte tunnel; it does not implement the
  constellation `display` stream framing.
