# nixling — host scaffold

You just ran:

```
nix flake init -t github:vicondoa/nixling
```

This directory now contains a minimal, two-file [nixling] host
configuration:

- `flake.nix` — pins `nixpkgs` and `nixling`, declares one
  `nixosConfigurations.<host>`.
- `configuration.nix` — the host config: one Wayland user, one
  isolated env, one workload VM.

`configuration.nix` is pre-filled with **sentinel placeholder
values** for the hard-to-default fields (`TODO-set-user`,
`TODO-set-hostname`, an empty `userAuthorizedKeys` list) and
**plausible real defaults** for the network CIDRs (RFC 5737
documentation ranges + a `192.168.1.0/24` host LAN). A matching
`assertions = [ … ]` block at the bottom of the file enforces the
sentinels.

`nix flake check` on the un-edited scaffold will fail with
actionable messages for **TODOs 2 and 3, including the SSH-key
sub-step** (hostname; the Wayland user identity; the at-least-one
SSH key under `nixling.site.userAuthorizedKeys`, which is the tail
half of TODO 3). The remaining TODOs (1 hardware, 4 SSH-user
echo, 5–7 network CIDRs) ship with values that PASS
`nix flake check` — they are gated by **your judgement**, not by
eval. Treat the assertion-passing scaffold as a starting point that
still requires a manual review of TODOs 5–7 before activation.

Why no eval-gate for the CIDR TODOs? The framework's per-env CIDR
validator in `nixos-modules/network.nix` does pure-Nix IPv4 prefix
arithmetic; non-numeric sentinel strings (`"TODO/REPLACE/CIDR"`)
would crash eval before any TODO assertion could fire with an
actionable message. Sentinels that pass format validation (RFC 5737
ranges, `192.168.x.0/24`) are indistinguishable from real LANs, so
they are flagged in comments rather than in `assertions = [ … ]`.

The only assertion the scaffold trips that isn't a nixling-side
TODO is NixOS's own `fileSystems."/"` check (TODO 1 — drop in a
real `hardware-configuration.nix`).

[nixling]: https://github.com/vicondoa/nixling

## What to edit

The placeholders are numbered `TODO 1` through `TODO 7` in
`configuration.nix`. They are, in order (the **Gate** column tells
you whether `nix flake check` will catch a missed edit):

| # | File | What | Gate |
|---|---|---|---|
| 1 | `configuration.nix` | Bootloader, filesystems, hardware. Drop in a real `hardware-configuration.nix` from `nixos-generate-config`. | NixOS's own `fileSystems."/"` check |
| 2 | `configuration.nix` | `networking.hostName` (sentinel: `TODO-set-hostname`). | assertion |
| 3 | `configuration.nix` | Rename the `let user = "TODO-set-user"` binding at the top of the file. It threads through `users.users.<user>`, `nixling.site.{waylandUser,launcherUsers}`, and `nixling.vms.corp-vm.ssh.user`. Also add at least one public key to `nixling.site.userAuthorizedKeys`. | assertion (× 2) |
| 4 | `configuration.nix` | `nixling.site.waylandUser` — keep at `user` for a graphical host, or set to `null` if you're going fully headless. | reviewed in TODO 3 |
| 5 | `configuration.nix` | `nixling.hostLanCidrs` — your host's primary LAN CIDR(s). `ip route` will tell you. Default `192.168.1.0/24` is a plausible home LAN. | judgement only |
| 6 | `configuration.nix` | `nixling.envs.<env>.lanSubnet` — the /24 your workload VMs sit on. Must not overlap TODO 5. Default `10.20.0.0/24` is a reasonable starting choice. | judgement + framework's CIDR-overlap check |
| 7 | `configuration.nix` | `nixling.envs.<env>.uplinkSubnet` — point-to-point /30 between host and the env's auto-declared net VM. Default `192.0.2.0/30` is an RFC 5737 doc range. | judgement + framework's /30-shape check |

`flake.nix` also contains two **optional renames** (the host
attribute and the flake description). They are not numbered in the
`TODO N:` scheme because they aren't required for a working
deployment — but you probably want to rename them anyway:

- The flake's `description` (currently `"TODO: short description of this host"`)
- The flake's `nixosConfigurations.desktop` attribute name
- The env (currently `work` in `configuration.nix`)
- The workload VM (currently `corp-vm` in `configuration.nix`)

…to names that fit your host.

## After editing

```bash
# 1. Confirm the eval graph is well-formed. Sentinel assertions
#    for TODOs 2-3 must pass; TODOs 5-7 carry plausible CIDR
#    defaults that pass eval but still need your review.
nix flake check

# 2. Build the host closure (no activation — useful for catching
#    eval errors and pulling the closure into the local store).
sudo nixos-rebuild build --flake .#desktop

# 3. Activate.
sudo nixos-rebuild switch --flake .#desktop
```

After `switch`, the `nixling` CLI is on `$PATH`, the env's bridges
(`br-work-up` / `br-work-lan`) and net VM (`sys-work-net`) are
materialised, and a framework-managed Ed25519 key has been generated
at `/var/lib/nixling/keys/corp-vm_ed25519`.

```bash
nixling list       # corp-vm + sys-work-net
# NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS
# corp-vm            work      false     false false   10.20.0.10      stopped
# sys-work-net       work      false     false false   192.0.2.2       running (net-vm)

nixling status     # same table + a "=== Bridge health ===" footer
# NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS
# corp-vm            work      false     false false   10.20.0.10      stopped
# sys-work-net       work      false     false false   192.0.2.2       running (net-vm)
#
# === Bridge health ===
# BRIDGE               STATE      ADMIN   EXPECTED     RESULT
# br-work-up           UP         up      UP           ok
# br-work-lan          NO-CARRIER up      NO-CARRIER   no-carrier (no workloads up)

# STATUS values: `running` = supervised by nixlingd with a live runner;
# `running (net-vm)` marks the auto-declared per-env net VM; `stopped`
# = no live runner.
nixling vm start corp-vm --apply   # boot it
ssh -i /var/lib/nixling/keys/corp-vm_ed25519 alice@10.20.0.10 hostname
nixling vm stop corp-vm --apply
```

`sys-work-net` (and every per-env net VM) is `autostart = true` by
construction in `nixos-modules/network.nix` — it has to come up
before any workload VM can use the LAN. Workload VMs are NOT
autostarted unless you flip `nixling.vms.<vm>.autostart = true`.

### After every subsequent rebuild

`nixos-rebuild switch` updates the declared nixling bundle and may
restart `nixlingd`, but daemon restarts are continuation events:
running VM runners are re-adopted rather than cycled. After rebuilding,
check whether any VM has pending changes:

```bash
nixling list
# ... STATUS column shows `running [pending restart]` for VMs whose
# `current` closure differs from `booted` while they're running.

nixling vm restart <vm> --apply    # apply the new declared VM closure
# or
nixling switch <vm> --apply        # full per-VM closure rebuild + live activation
```

`nixling status <vm>` (per-VM view) reports `pending-restart: yes/no`
with both store paths and the exact remediation command. See
[`docs/reference/cli-contract.md`](../../docs/reference/cli-contract.md#pending-restart-signal-v015)
for the full semantics.

## Going further

- **More VMs**: copy the `nixling.vms.corp-vm` block, give it a new
  name and a new `index` (`10`–`250`, unique within an env).
- **More envs**: copy the `nixling.envs.work` block, give it a new
  name + non-overlapping `lanSubnet`/`uplinkSubnet`. VMs in
  different envs cannot talk to each other.
- **Graphics / audio**: flip `graphics.enable = true` (and/or
  `audio.enable = true`) on a VM. Requires `nixling.site.waylandUser`
  set. See [`examples/graphics-workstation`][gfx-example].
- **Microsoft Entra ID**: compose [`vicondoa/entrablau.nix`][entrablau]
  per-VM via `nixling.vms.<vm>.config.imports`. See
  [`examples/with-entra-id`][entra-example].
- **Two-env isolation**: see [`examples/multi-env`][multi-env-example].

[gfx-example]: https://github.com/vicondoa/nixling/tree/main/examples/graphics-workstation
[entra-example]: https://github.com/vicondoa/nixling/tree/main/examples/with-entra-id
[multi-env-example]: https://github.com/vicondoa/nixling/tree/main/examples/multi-env
[entrablau]: https://github.com/vicondoa/entrablau.nix

## Common gotchas

- `/var/lib/nixling` MUST live on the same filesystem as
  `/nix/store` (the per-VM `/nix/store` is a hardlink farm).
- CIDR overlap is an eval error, by design.
- A graphics VM with `nixling.site.waylandUser = null` is an eval
  error — there is no X11 fallback path.
- The sentinel assertions only fire if you leave a TODO at its
  default value; replacing one sentinel without replacing the
  others still fails until they're all gone.

## See also

- [`examples/minimal`](https://github.com/vicondoa/nixling/tree/main/examples/minimal) — read-and-copy headless starter
- [`examples/graphics-workstation`](https://github.com/vicondoa/nixling/tree/main/examples/graphics-workstation) — desktop VM with Wayland + audio + USBIP
- [`examples/multi-env`](https://github.com/vicondoa/nixling/tree/main/examples/multi-env) — two isolated envs (work + personal)
- [`examples/with-entra-id`](https://github.com/vicondoa/nixling/tree/main/examples/with-entra-id) — Entra-ID composition via the sibling flake

See the upstream [README][readme] for the full quick-start, the
threat model, and the design rationale.

[readme]: https://github.com/vicondoa/nixling#readme
