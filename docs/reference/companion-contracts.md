# Desktop companion inventory

**Diataxis category:** reference.

**Contract version:** d2b 3.0 (v3).

**Document revision:** 2.

**Publication status:** The inventory is published. Compatibility verification
is still pending.

This is the complete inventory of desktop companions whose compatibility is a
release condition for d2b 3.0. A companion is included when it consumes a d2b
public operator contract, a public presentation artifact, or a packaged
desktop integration supplied by d2b. The four rows below are the current
release-blocking set, not an example list.

## Verification status

Publishing this page is not a compatibility sign-off. No live-host exercise
against a d2b 3.0 release candidate is claimed here. Source inspection,
matching version numbers, and reading this document are not verification
evidence.

Each row remains pending until the companion is exercised on the daily-driver
host against the release candidate. An incompatible or unverified companion
holds the d2b 3.0 release.

## Inventory

| Companion | Repository | Pinned commit | Maintainer of record | Discovery source | Exact consumed surface | Verification status |
| --- | --- | --- | --- | --- | --- | --- |
| `d2b-toolkit` | `github:vicondoa/d2b-toolkit` | **Unresolved:** no pinned companion commit is recorded; no network lookup was run. | **Unresolved:** no maintainer of record is stated in repository or contract records; no network lookup was run. | `docs/contributing/architecture.md`; `docs/how-to/configure-desktop-terminal-integration.md` | Client DTOs for the public daemon API; public-socket framing; Wayland color parsing; Waybar helpers. See [the v3 replacement contract](./zone-cli-contract.md) and [the UI color contract](./ui-colors.md). | **Pending live-host verification.** No release-candidate exercise is recorded by this publication. |
| `d2b-wlterm` | `github:vicondoa/d2b-wlterm` | **Unresolved:** no pinned companion commit is recorded; no network lookup was run. | **Unresolved:** no maintainer of record is stated in repository or contract records; no network lookup was run. | `docs/contributing/architecture.md`; `docs/how-to/configure-desktop-terminal-integration.md` | Qualified ShellSession Resource lifecycle, ProcessAttachClient named streams, provider-neutral launcher metadata, canonical Host/Guest execution references, and the `d2b-wayland-proxy` package. The retired public-socket `ShellOp` family is not supported. See [the persistent-shell reference](./guest-control-persistent-shell.md), [the v3 replacement contract](./zone-cli-contract.md), and [the desktop integration guide](../how-to/configure-desktop-terminal-integration.md). | **Pending live-host verification.** No release-candidate exercise is recorded by this publication. |
| `d2b-wlcontrol` | **Unresolved:** no repository URL is recorded in repository or contract records. | **Unresolved:** no pinned companion commit is recorded; no network lookup was run. | **Unresolved:** no maintainer of record is stated in repository or contract records; no network lookup was run. | `docs/adr/0040-graceful-vm-shutdown.md`; `docs/adr/0041-console-and-audio-controls.md`; `docs/how-to/use-console-and-audio.md` | The public daemon socket; `/etc/d2b/ui-colors.json`; `/etc/d2b/ui-colors.css`; `d2b audio status --json`; security-key state and action DTOs (`WlcontrolSkStatus`, `WlcontrolCeremonyRow`, and `WlcontrolAction`); the `d2b device security-key status`, `sessions`, and `cancel` operations; and graceful-stop semantics, including the distinction between normal stop and `--force`. See [the UI color contract](./ui-colors.md), [the CLI contract](./cli-contract.md), and [the v3 replacement contract](./zone-cli-contract.md). | **Pending live-host verification.** No release-candidate exercise is recorded by this publication. |
| `d2b-clip-picker` | `github:vicondoa/d2b-clip-picker` | **Unresolved:** no pinned companion commit is recorded; no network lookup was run. | **Unresolved:** no maintainer of record is stated in repository or contract records; no network lookup was run. | `docs/adr/0042-d2b-clipboard-authority-and-picker-split.md`; `docs/how-to/configure-clipboard-picker.md` | The versioned newline-delimited JSON picker protocol over an inherited anonymous Unix `socketpair()` file descriptor; `ClientHello`, `OpenRequest`, `Select`, and `Cancel`; canonical realm/workload target names; and resolved accent colors. See [the picker protocol](./clipboard-picker-protocol.md) and [the realm target contract](./realm-access-resolver.md). | **Pending live-host verification.** No release-candidate exercise is recorded by this publication. |

## Excluded candidate: `weezterm`

The repository records identify `weezterm` as the terminal binary selected by
the `d2b-wlterm` launcher, but explicitly record that it consumes no d2b
socket, schema, presentation artifact, or other surface in the FR-064 limb-2
list. It is therefore not a member of this inventory. This is a negative
surface-consumption determination recorded on 2026-08-05, not a compatibility
verification.

The repository URL `github:vicondoa/weezterm` is recorded in
`docs/how-to/configure-desktop-terminal-integration.md`. No pinned
`weezterm` commit or maintainer of record is present in repository or contract
records, and no network lookup was run. The external revision-bound metadata
for this excluded candidate remains unresolved; the exclusion is limited to
the recorded negative determination that it consumes no d2b surface.

## Companion boundaries

The following sibling-shaped projects are deliberately not companions in this
inventory:

- `entrablau.nix` is an identity sibling composed per guest, not a desktop
  companion.
- `wl-proxy` is an upstream crate dependency, not a sibling desktop
  repository.

They therefore do not expand the release-blocking companion set.

## Verification record required before release

For every row, the release record must identify:

1. the exact d2b release candidate exercised;
2. the companion revision and the host integration used;
3. a live-host exercise of every surface named in the row; and
4. the result, including any capability refusal or degraded behavior.

The release record must not substitute a source diff, a package version, or a
successful documentation check for the live-host exercise. Until all four
records are complete and compatible, the release gate remains closed.
