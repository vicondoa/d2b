# `with-entra-id` — composing nixling with `nixos-entra-id`

This example is the **integration showcase** for using
[`vicondoa/nixling`][nixling] together with the framework-agnostic
[`vicondoa/nixos-entra-id`][nixos-entra-id] flake to spin up an
Entra-ID-joined work microVM on a NixOS host.

The two flakes are deliberately decoupled — neither imports the
other. This directory is the **consumer-side composition** that
wires them together in your top-level `flake.nix`. Read the same
pattern from the `nixos-entra-id` side at
[`examples/inside-nixling-vm/`][entra-from-the-other-side] in that
repo.

## What you get

A single `nixosConfigurations.demo` that declares one isolated
work environment with one Entra-joined VM:

| Component                | Owner            | Role                                                 |
|--------------------------|------------------|------------------------------------------------------|
| Bridges / NAT / firewall | nixling          | Per-env isolated /24 LAN, /30 uplink, NAT egress     |
| Net VM (`sys-work-net`)  | nixling          | Auto-declared dnsmasq + nftables NAT router          |
| swtpm 2.0                | nixling          | Software TPM exposed to guest at `/dev/tpmrm0`       |
| Wayland / audio / GPU    | nixling          | (Off in this minimal example — TPM-only headless VM) |
| YubiKey USBIP backend    | nixling          | (Available; toggle `usbip.yubikey = true` per VM)    |
| Himmelblau daemon + PAM  | nixos-entra-id   | Linux-native Entra ID client (TPM-enabled rebuild)   |
| Intune compliance shims  | nixos-entra-id   | Fake DMI / `/etc/os-release`, `FileDescriptorStoreMax` for PRT survival |
| Firefox SSO + pinentry   | nixos-entra-id   | Browser-broker plumbing for the interactive MFA flow |

## File layout

```
examples/with-entra-id/
├── flake.nix           inputs: nixpkgs, nixling, nixos-entra-id
│                       outputs: nixosConfigurations.demo
├── configuration.nix   host-side: user, nixling.site, nixling.envs.work
├── work-vm.nix         guest-side: hostname, security.tpm2, nixosEntraId.*
└── README.md           you are here
```

## Two-flake composition: who owns what

The split is intentional. **`nixling` owns VM lifecycle and host
plumbing**; **`nixos-entra-id` owns Entra protocol + Intune
compatibility**. Putting Entra inside nixling would have coupled the
framework to Microsoft-specific machinery that the average single-
user NixOS desktop will never want.

### Options that live in `nixling.*` (from this flake)

These are set on the **host** in `configuration.nix`, or on the VM
attrset in `flake.nix`. They have nothing to do with Entra; they
configure the VM itself.

| Option                                | Set in            | Purpose                                           |
|---------------------------------------|-------------------|---------------------------------------------------|
| `nixling.site.waylandUser`            | `configuration.nix` | Host's Plasma / Wayland user                    |
| `nixling.site.launcherUsers`          | `configuration.nix` | Polkit grant for `nixling up/down`              |
| `nixling.site.yubikey.enable`         | `configuration.nix` | Host-side YubiKey udev rules + `usbip-host`     |
| `nixling.envs.<env>.lanSubnet`        | `configuration.nix` | Per-env workload `/24`                          |
| `nixling.envs.<env>.uplinkSubnet`     | `configuration.nix` | Per-env host↔net-VM `/30`                       |
| `nixling.vms.<vm>.tpm.enable`         | `flake.nix`         | swtpm for this VM                               |
| `nixling.vms.<vm>.graphics.enable`    | `flake.nix`         | virtio-gpu + Wayland forward (off in this example) |
| `nixling.vms.<vm>.usbip.yubikey`      | `flake.nix`         | YubiKey USBIP passthrough (off in this example) |
| `nixling.vms.<vm>.env`, `index`       | `flake.nix`         | Bind VM into the env's LAN; derive MAC + IP     |
| `nixling.vms.<vm>.ssh.user`           | `flake.nix`         | SSH user the CLI uses to talk into the VM       |
| `nixling.vms.<vm>.config.imports`     | `flake.nix`         | **The composition seam** — guest NixOS modules  |

### Options that live in `nixosEntraId.*` (from the other flake)

These are set **inside the VM** in `work-vm.nix`. They configure
Himmelblau and the Intune compliance shim. The full schema is in
the [`nixos-entra-id` README][nixos-entra-id-readme].

| Option                                       | Purpose                                                            |
|----------------------------------------------|--------------------------------------------------------------------|
| `nixosEntraId.enable`                        | Activate the Himmelblau workspace + module                         |
| `nixosEntraId.domain`                        | Tenant domain(s) (`listOf str`)                                    |
| `nixosEntraId.userMap.<local> = <UPN>`       | Local-user → Entra UPN mapping (`/etc/himmelblau/user-map`)        |
| `nixosEntraId.joinType`                      | `"join"` (Intune-enrolled) or `"register"` (BYOD)                  |
| `nixosEntraId.localUser`                     | Diagnostic — name of the local user that maps to the UPN           |
| `nixosEntraId.intuneCompliance.enable`       | Turn on/off the Intune compliance shim                             |
| `nixosEntraId.intuneCompliance.fakeDmi`      | SMBIOS values bind-mounted into himmelblau's mount ns              |

### The seam

The two trees meet at exactly one place — `flake.nix`:

```nix
nixling.vms.work-vm = {
  tpm.enable = true;             # nixling option
  config = {
    imports = [
      nixos-entra-id.nixosModules.default   # bring in the other flake
      ./work-vm.nix                         # bring in our own VM config
    ];
  };
};
```

`config` is a regular NixOS module. It gets merged into the VM's
internal NixOS configuration by nixling's `host.nix` translation
into `microvm.vms.<name>`. From the imported modules' perspective,
they're being evaluated as a normal NixOS system — they neither
know nor care that they're inside a microVM.

## Why TPM 2.0 is mandatory

Microsoft Entra Conditional Access on most enterprise tenants
requires a **hardware-rooted device identity**. Without it,
Himmelblau can still authenticate, but the PRT (Primary Refresh
Token) cookie is bound to a software key, and a Conditional
Access policy of the form "require compliant device" will refuse
to issue further tokens.

Specifically, the device-registration flow generates a
**device-bound certificate** with a CSR signed by a TPM-resident
key. The receiving endpoint validates that the certificate is
backed by an attestable TPM. A software-only Himmelblau build will
either skip the TPM portion (failing CA) or emit a CSR the
endpoint refuses with
`400 Bad Request: Value must be a valid PEM-encoded PKCS#10 CSR`.

`nixos-entra-id` ships a TPM-enabled rebuild of the Himmelblau
workspace specifically because the upstream build has the `tpm`
cargo feature off by default and requires two vendored crate
patches to make the Intune CSR validation pass. See
[`pkgs/himmelblau-tpm/MAINTAINING.md`][himmelblau-tpm-maintaining]
in `nixos-entra-id` for the patch rationale.

**swtpm satisfies this requirement**, because the kernel inside
the VM sees a real TPM CRB at `/dev/tpmrm0` and exposes it through
the standard tpm2 stack. The keys it generates persist on the host
under `/var/lib/nixling/vms/<vm>/swtpm/` (for this example:
`/var/lib/nixling/vms/work-vm/swtpm/`).

> **DO NOT wipe this directory.** It holds the Intune device-bound
> TPM credentials. Wiping it forces a fresh device-registration on
> next boot — Entra/Intune may register that as a tamper signal in
> your IT's compliance feed, and you'll need to clean up the old
> stale device entry from the tenant. Back it up the same way you
> back up the rest of nixling state (each `microvm.stateDir` /
> `swtpm-state` pair is a unit; see
> `nixos-modules/host-sidecars.nix` for the exact paths).

## Intune compliance disclaimer

This is **compatibility, not bypass**.

The `intuneCompliance` shim makes a Linux/microVM guest *look* to
Intune the way a supported corporate device would — same fake DMI
strings, same `/etc/os-release` shape, same `FileDescriptorStoreMax`
behaviour the Windows / macOS clients rely on for PRT survival
across restarts. That gets you past the device-compliance gate so
the actual user-facing Conditional Access policies (MFA prompt,
location check, risk-based step-up) can run their normal flow.

It does **not** hide that you're running Linux + nixling +
Himmelblau:

- The Himmelblau client identifies itself in protocol-level
  fingerprints (User-Agent, signed-JWT issuer fields, certificate
  attributes from its CSR). Tenant admins running detailed sign-in
  logs see the exact client.
- The TPM AIK / EK certificates the device emits are signed by
  swtpm's own CA chain, not by a Windows-vendor TPM CA. Anything
  validating attestation provenance (Microsoft Defender for
  Identity, conditional access policies inspecting TPM EK
  thumbprints) will flag this device as "Linux with software TPM."
- The vendored fake DMI values are static text — they will not
  satisfy any compliance check that cross-references a
  Manufacturer-Vendor-ID against a hardware database.

**Use this only on tenants where you, the user, are entitled to
sign in from a Linux workstation.** Your IT admin sees the
device. If your acceptable-use policy says "only corporate
Windows," using this is a policy violation regardless of whether
the technical compliance check passes.

`nixos-entra-id` was written to support migrating a corporate
workstation off Windows onto NixOS, with the IT department's
explicit awareness. That's the supported use case. If you need to
actually circumvent enterprise controls, this is the wrong tool
— and the wrong approach.

## Quick start

The example wires `inputs.nixling.url = "path:../.."` so it builds
against the in-tree nixling sources during the refactor. Real
consumers should swap that for a tagged GitHub ref
(`github:vicondoa/nixling/v0.1.0` once tags exist, or
`github:vicondoa/nixling` to track `main`); see the comments in
`flake.nix`.

### 1. Evaluate

From the repo root, eval that the example builds without surprises:

```bash
nix flake check --no-build --all-systems ./examples/with-entra-id

# Force the toplevel derivation path to resolve (proves the full
# nixosSystem module graph evaluates without errors):
nix eval ./examples/with-entra-id#nixosConfigurations.demo.config.system.build.toplevel.drvPath
```

### 2. Build

Once you've cloned this into your own host config (replacing the
filesystem + bootloader stubs in `configuration.nix` with your
real `hardware-configuration.nix`):

```bash
sudo -A nixos-rebuild build  --flake .#demo
```

The first build pulls in `nixos-entra-id`'s TPM-enabled Himmelblau
rebuild — ~10 minutes of Rust compile on a cold cache, cached
afterwards. **Do not skip the `build` step**: a broken Entra config
that switches live is harder to recover from than one that fails
loudly at build time.

### 3. Activate

```bash
sudo -A nixos-rebuild switch --flake .#demo
```

This creates `/var/lib/nixling/keys/work-vm_ed25519`, spawns
`sys-work-net` (the per-env net VM), materialises the
`br-work-up` + `br-work-lan` bridges, and installs the `nixling`
CLI on `$PATH`. The work VM itself is **not** started — graphics
VMs and Entra VMs both expect interactive launch.

After activation, `nixling list` / `nixling status` should look
like this (the work VM has `tpm.enable = true`, the rest is
default):

```text
nixling list
# NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS
# sys-work-net       work      false     false false   192.0.2.2       systemd (net-vm)
# work-vm            work      false     true  false   10.20.0.10      stopped

nixling status
# NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS
# sys-work-net       work      false     false false   192.0.2.2       systemd (net-vm)
# work-vm            work      false     true  false   10.20.0.10      stopped
#
# === Bridge health ===
# BRIDGE               STATE      ADMIN   EXPECTED     RESULT
# br-work-up           UP         up      UP           ok
# br-work-lan          NO-CARRIER up      NO-CARRIER   no-carrier (no workloads up)
```

`work-vm` shows `STATUS=stopped` until you `nixling up work-vm`;
after that it transitions to `interactive` (because Entra VMs are
expected to be launched ad-hoc from a Plasma terminal, never as a
systemd unit). The `TPM=true` column reflects the swtpm sidecar
wired up by `nixling.vms.work-vm.tpm.enable = true`.

### 4. Bring the VM up

From a Plasma / Wayland terminal (not over SSH — see
[nixling's README, "Common gotchas"][nixling-readme] for why):

```bash
nixling up work-vm
```

### 5. Trigger enrolment

SSH into the VM, then drive the auth flow. `aad-tool auth-test` is
the canonical way to surface enrolment errors without going through
a graphical login:

```bash
ssh -i /var/lib/nixling/keys/work-vm_ed25519 alice@10.20.0.10

# Inside the VM:
sudo aad-tool auth-test --name alice@contoso.com
```

A `pinentry-qt` window pops on the host (forwarded via the VM's
Wayland session if graphics is enabled, or the regular console
flow if headless). Answer the password + Hello / MFA prompt; on
success the device receives a client cert from
`Microsoft Intune Beta MDM Device CA` and the sealed PRT is
written to `/var/lib/himmelblaud/`.

### 6. Verify

```bash
# Still inside the VM:
aad-tool tpm                                     # 'Hardware TPM supported: true'
aad-tool status                                  # himmelblaud reachable
getent passwd alice@contoso.com                  # NSS sees the Entra user
systemctl status himmelblaud himmelblaud-tasks   # daemons healthy
```

## Customising

- **Other tenants** — swap `contoso.com` for your domain and
  update `userMap` + `localUser`. Read the
  [`nixos-entra-id` README quick start][nixos-entra-id-readme]
  for tenant prerequisites (admin role, Conditional Access caveats,
  `dmidecode` for realistic `fakeDmi` values).
- **Add graphics** — set `nixling.vms.work-vm.graphics.enable =
  true` in `flake.nix` and the VM gains a virtio-gpu + Wayland
  forward to the host compositor (a `foot` terminal auto-launches
  inside the guest on boot). Requires `nixling.site.waylandUser`
  to be non-null on the host — already set in this example.
- **Add YubiKey passthrough** — set
  `nixling.vms.work-vm.usbip.yubikey = true` and run
  `nixling usb work-vm` to redirect a plugged YubiKey from the
  host's USB controller to the VM via USBIP. Useful for the MFA
  prompt during `aad-tool auth-test` and any downstream FIDO2
  flow.
- **BYOD / no Intune** — set `nixosEntraId.joinType = "register"`
  and `nixosEntraId.intuneCompliance.enable = false`. The TPM is
  still useful (PRT survival), but the compliance shim drops out
  of the picture.

## Platform support

This example targets **`x86_64-linux`**. The flake declares
`system = "x86_64-linux"` explicitly in
`nixosConfigurations.demo`.

The framework itself is multi-arch (headless VMs eval on
`aarch64-linux`); nixling's platform gate fires on `graphics.enable`
+ `audio.enable` only — **not on `tpm.enable`** — so a future
`aarch64`-clean variant of this example would be possible if
upstream Himmelblau gained an `aarch64` cargo build. Today,
`nixos-entra-id`'s TPM-enabled Himmelblau package is wired for
`x86_64-linux` only via its `himmelblauSystems` allowlist
(see the `nixos-entra-id` flake.nix), so the practical answer
remains `x86_64-linux` for the foreseeable future.

## Where the two flakes' docs disagree

If something in this example contradicts the option descriptions in
either upstream flake's README, **the option descriptions win**.
File an issue against this example's README and we'll bring it
back into sync.

## Common gotchas

- **TPM state backup**: do **not** wipe
  `/var/lib/nixling/vms/work-vm/swtpm/`. It holds the per-VM TPM
  2.0 NVRAM + EK seed that Entra/Intune treats as the device's
  hardware identity. Zeroing it forces re-enrolment and looks
  like device tampering to the IdP.
- **First Himmelblau enrollment can take 30–60 seconds.** The
  initial AAD device-code dance + Intune policy pull is
  network-bound; subsequent logins are fast.
- **x86_64-only.** Both the graphics component (cloud-hypervisor +
  crosvm GPU sidecar) and TPM emulation paths are platform-gated
  to `x86_64-linux`. aarch64 hosts will fail eval with an
  actionable message.
- **`nixling up work-vm` before SSH/enrollment.** The Himmelblau
  service inside the VM doesn't start until the VM is up;
  attempting to enrol against a stopped VM hangs at the first
  device-code prompt.
- **Intune policy visibility depends on tenant configuration.**
  Whether you see Compliance / Conditional Access results in the
  Entra portal is a function of the tenant's MDM scope; the
  framework can't enforce policy visibility, only the
  authentication primitives.

## See also

- [`examples/minimal`](../minimal/) — read-and-copy headless starter
- [`examples/graphics-workstation`](../graphics-workstation/) — desktop VM with Wayland + audio + USBIP
- [`examples/multi-env`](../multi-env/) — two isolated envs (work + personal)
- [`templates/default`](../../templates/default/) — scaffold via `nix flake init`
- [`vicondoa/nixos-entra-id`][nixos-entra-id] — the Entra/Himmelblau
  module bundle. Read its README for tenant prerequisites,
  detailed enrolment troubleshooting, and the full `nixosEntraId.*`
  schema.
- [`vicondoa/nixos-entra-id/examples/inside-nixling-vm/`][entra-from-the-other-side]
  — the same composition pattern documented from the Entra flake's
  side. Slightly different emphasis (Entra-first); cross-link
  rather than duplicate.
- [`vicondoa/nixling` README][nixling-readme] — quick start, common
  gotchas, full option index.

[nixling]: https://github.com/vicondoa/nixling
[nixling-readme]: ../../README.md
[nixos-entra-id]: https://github.com/vicondoa/nixos-entra-id
[nixos-entra-id-readme]: https://github.com/vicondoa/nixos-entra-id#readme
[entra-from-the-other-side]: https://github.com/vicondoa/nixos-entra-id/tree/main/examples/inside-nixling-vm
[himmelblau-tpm-maintaining]: https://github.com/vicondoa/nixos-entra-id/blob/main/pkgs/himmelblau-tpm/MAINTAINING.md
