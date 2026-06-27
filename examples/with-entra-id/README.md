# `with-entra-id` — composing d2b with `entrablau`

This example is the **integration showcase** for using
[`vicondoa/d2b`][d2b] together with the framework-agnostic
[`vicondoa/entrablau.nix`][entrablau] flake to spin up an
Entra-ID-joined work microVM on a NixOS host.

The two flakes are deliberately decoupled — neither imports the
other. This directory is the **consumer-side composition** that
wires them together in your top-level `flake.nix`.

## What you get

A single `nixosConfigurations.demo` that declares one isolated
work environment with one Entra-joined VM:

| Component                | Owner            | Role                                                 |
|--------------------------|------------------|------------------------------------------------------|
| Bridges / NAT / firewall | d2b          | Per-env isolated /24 LAN, /30 uplink, NAT egress     |
| Net VM (`sys-work-net`)  | d2b          | Auto-declared dnsmasq + nftables NAT router          |
| swtpm 2.0                | d2b          | Software TPM exposed to guest at `/dev/tpmrm0`       |
| Wayland / audio / GPU    | d2b          | (Off in this minimal example — TPM-only headless VM) |
| YubiKey USBIP backend    | d2b          | (Available; toggle `usbip.yubikey = true` per VM)    |
| Himmelblau daemon + PAM  | entrablau        | Linux-native Entra ID client (TPM-enabled rebuild)   |
| Intune compliance shims  | entrablau        | `dmiOverride` / `osReleaseOverride`, `FileDescriptorStoreMax` for PRT survival |
| Firefox SSO + pinentry   | entrablau        | Browser-broker plumbing for the interactive MFA flow |

## File layout

```
examples/with-entra-id/
├── flake.nix           inputs: nixpkgs, d2b, entrablau
│                       outputs: nixosConfigurations.demo
├── configuration.nix   host-side: user, d2b.site, d2b.envs.work
├── work-entra.nix         guest-side: hostname, security.tpm2, entrablau.*
└── README.md           you are here
```

## Two-flake composition: who owns what

The split is intentional. **`d2b` owns VM lifecycle and host
plumbing**; **`entrablau` owns Entra protocol + Intune
compatibility**. Putting Entra inside d2b would have coupled the
framework to Microsoft-specific machinery that the average single-
user NixOS desktop will never want.

### Options that live in `d2b.*` (from this flake)

These are set on the **host** in `configuration.nix`, or on the VM
attrset in `flake.nix`. They have nothing to do with Entra; they
configure the VM itself.

| Option                                | Set in            | Purpose                                           |
|---------------------------------------|-------------------|---------------------------------------------------|
| `d2b.site.waylandUser`            | `configuration.nix` | Host's Plasma / Wayland user                    |
| `d2b.site.launcherUsers`          | `configuration.nix` | Adds users to the `d2b` lifecycle group for daemon socket access |
| `d2b.site.yubikey.enable`         | `configuration.nix` | Host-side YubiKey udev rules; `usbip-host` loads on per-VM opt-in |
| `d2b.envs.<env>.lanSubnet`        | `configuration.nix` | Per-env workload `/24`                          |
| `d2b.envs.<env>.uplinkSubnet`     | `configuration.nix` | Per-env host↔net-VM `/30`                       |
| `d2b.vms.<vm>.tpm.enable`         | `flake.nix`         | swtpm for this VM                               |
| `d2b.vms.<vm>.graphics.enable`    | `flake.nix`         | virtio-gpu + Wayland forward (off in this example) |
| `d2b.vms.<vm>.usbip.yubikey`      | `flake.nix`         | YubiKey USBIP passthrough (off in this example) |
| `d2b.vms.<vm>.env`, `index`       | `flake.nix`         | Bind VM into the env's LAN; derive MAC + IP     |
| `d2b.vms.<vm>.ssh.user`           | `flake.nix`         | SSH user the CLI uses to talk into the VM       |
| `d2b.vms.<vm>.config.imports`     | `flake.nix`         | **The composition seam** — guest NixOS modules  |

### Options that live in `entrablau.*` (from the other flake)

These are set **inside the VM** in `work-entra.nix`. They configure
Himmelblau and the Intune compliance shim. The full schema is in
the [`entrablau` README][entrablau-readme].

| Option                                       | Purpose                                                            |
|----------------------------------------------|--------------------------------------------------------------------|
| `entrablau.enable`                        | Activate the Himmelblau workspace + module                         |
| `entrablau.domain`                        | Tenant domain(s) (`listOf str`)                                    |
| `entrablau.userMap.<local> = <UPN>`       | Local-user → Entra UPN mapping (`/etc/himmelblau/user-map`)        |
| `entrablau.joinType`                      | `"join"` (Intune-enrolled) or `"register"` (BYOD)                  |
| `entrablau.localUser`                     | Diagnostic — name of the local user that maps to the UPN           |
| `entrablau.intuneCompliance.enable`       | Turn on/off the Intune compliance shim                             |
| `entrablau.intuneCompliance.dmiOverride`      | SMBIOS values bind-mounted into himmelblau's mount ns              |

### The seam

The two trees meet at exactly one place — `flake.nix`:

```nix
d2b.vms.work-entra = {
  tpm.enable = true;             # d2b option
  config = {
    imports = [
      entrablau.nixosModules.default   # bring in the other flake
      ./work-entra.nix                         # bring in our own VM config
    ];
  };
};
```

`config` is a regular NixOS module. It gets merged into the VM's
internal NixOS configuration by d2b's `host.nix` translation
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

`entrablau` ships a TPM-enabled rebuild of the Himmelblau
workspace specifically because the upstream build has the `tpm`
cargo feature off by default and requires two vendored crate
patches to make the Intune CSR validation pass. See
[`pkgs/himmelblau-tpm/MAINTAINING.md`][himmelblau-tpm-maintaining]
in `entrablau` for the patch rationale.

**swtpm satisfies this requirement**, because the kernel inside
the VM sees a real TPM CRB at `/dev/tpmrm0` and exposes it through
the standard tpm2 stack. The keys it generates persist on the host
under `/var/lib/d2b/vms/<vm>/swtpm/` (for this example:
`/var/lib/d2b/vms/work-entra/swtpm/`).

TPM persistence is necessary but not sufficient for stable Entra device
identity. Himmelblau also stores identity-bearing OS state under the
guest's `/var` tree, including the systemd credential host key,
Himmelblau's encrypted HSM PIN credential, and its cache DB. Declare a
persistent `/var` volume for Entra-joined VMs, for example:

```nix
config.microvm.volumes = [{
  image = "var.img";
  mountPoint = "/var";
  size = 8192;
  fsType = "ext4";
}];
```

D2b mounts declared `microvm.volumes` by stable virtio serial inside
the guest. Without this persistent `/var`, the root tmpfs regenerates
`/etc/machine-id`, `/var/lib/systemd/credential.secret`, and Himmelblau
state after each restart, which can look like a new device enrollment
even when the swtpm directory is intact.

Existing Entra-joined VMs that already booted without a persistent `/var`
may need one final enrollment when you add the volume, because the prior
identity state may only have existed on tmpfs. Once `/var` is mounted from
the persistent volume and enrollment state is written there, subsequent VM
restarts should keep the same device identity.

> **DO NOT wipe this directory.** It holds the Intune device-bound
> TPM credentials. Wiping it forces a fresh device-registration on
> next boot — Entra/Intune may register that as a tamper signal in
> your IT's compliance feed, and you'll need to clean up the old
> stale device entry from the tenant. Back it up the same way you
> back up the rest of d2b state (each `microvm.stateDir` /
> `swtpm-state` pair is a unit; see
> `nixos-modules/host-sidecars.nix` for the exact paths).

## Intune compliance disclaimer

This is **compatibility, not bypass**.

The `intuneCompliance` shim makes a Linux/microVM guest *look* to
Intune the way a supported corporate device would — same DMI override
strings, same `/etc/os-release` shape, same `FileDescriptorStoreMax`
behaviour the Windows / macOS clients rely on for PRT survival
across restarts. That gets you past the device-compliance gate so
the actual user-facing Conditional Access policies (MFA prompt,
location check, risk-based step-up) can run their normal flow.

It does **not** hide that you're running Linux + d2b +
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
- The vendored DMI override values are static text — they will not
  satisfy any compliance check that cross-references a
  Manufacturer-Vendor-ID against a hardware database.

**Use this only on tenants where you, the user, are entitled to
sign in from a Linux workstation.** Your IT admin sees the
device. If your acceptable-use policy says "only corporate
Windows," using this is a policy violation regardless of whether
the technical compliance check passes.

`entrablau` was written to support migrating a corporate
workstation off Windows onto NixOS, with the IT department's
explicit awareness. That's the supported use case. If you need to
actually circumvent enterprise controls, this is the wrong tool
— and the wrong approach.

## Quick start

The example wires `inputs.d2b.url = "path:../.."` so it builds
against the in-tree d2b sources during the refactor. Real
consumers should swap that for a tagged GitHub ref
(`github:vicondoa/d2b/v0.1.0` once tags exist, or
`github:vicondoa/d2b` to track `main`); see the comments in
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

The first build pulls in `entrablau`'s TPM-enabled Himmelblau
rebuild — ~10 minutes of Rust compile on a cold cache, cached
afterwards. **Do not skip the `build` step**: a broken Entra config
that switches live is harder to recover from than one that fails
loudly at build time.

### 3. Activate

```bash
sudo -A nixos-rebuild switch --flake .#demo
```

This creates `/var/lib/d2b/keys/work-entra_ed25519`, spawns
`sys-work-net` (the per-env net VM), materialises the
`br-work-up` + `br-work-lan` bridges, and installs the `d2b`
CLI on `$PATH`. The work VM itself is **not** started — graphics
VMs and Entra VMs both expect interactive launch.

After activation, `d2b list` / `d2b status` should look
like this (the work VM has `tpm.enable = true`, the rest is
default):

```text
d2b list
# NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS
# sys-work-net       work      false     false false   192.0.2.2       running (net-vm)
# work-entra            work      false     true  false   10.20.0.10      stopped

d2b status
# NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS
# sys-work-net       work      false     false false   192.0.2.2       running (net-vm)
# work-entra            work      false     true  false   10.20.0.10      stopped
#
# === Bridge health ===
# BRIDGE               STATE      ADMIN   EXPECTED     RESULT
# br-work-up           UP         up      UP           ok
# br-work-lan          NO-CARRIER up      NO-CARRIER   no-carrier (no workloads up)
```

`work-entra` shows `STATUS=stopped` until you `d2b vm start
work-entra --apply`; after that it transitions to `running` under
`d2bd` supervision. The `TPM=true` column reflects the swtpm
sidecar wired up by `d2b.vms.work-entra.tpm.enable = true`.

### 4. Bring the VM up

From a Plasma / Wayland terminal (not over SSH — see
[d2b's README, "Common gotchas"][d2b-readme] for why):

```bash
d2b vm start work-entra --apply
```

### 5. Trigger enrolment

SSH into the VM, then drive the auth flow. `aad-tool auth-test` is
the canonical way to surface enrolment errors without going through
a graphical login:

```bash
ssh -i /var/lib/d2b/keys/work-entra_ed25519 alice@10.20.0.10

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

Also verify the persistence layer before trusting a device enrollment:

```bash
# Still inside the VM:
findmnt /var
# expected: /var is ext4 from /dev/disk/by-id/virtio-var (or /dev/vda)

cat /etc/machine-id
sudo sha256sum \
  /var/lib/systemd/credential.secret \
  /var/lib/private/himmelblaud/hsm-pin.enc
sudo tpm2_readpublic -c 0x81000001 | grep -E '^(name:|qualified name:)'
```

After `d2b vm stop work-entra --apply` followed by
`d2b vm start work-entra --apply`, those values should remain stable
except for normal Himmelblau cache DB churn. If the VM asks to re-enroll
after every restart, check `findmnt /var` first: TPM loss and `/var`
persistence loss look similar from Entra's point of view, but the fix is
different. Mount failures surface as `var.mount` / `local-fs.target`
errors in the guest journal:

```bash
journalctl -b -u var.mount -u local-fs.target --no-pager
```

## Customising

- **Other tenants** — swap `contoso.com` for your domain and
  update `userMap` + `localUser`. Read the
  [`entrablau` README quick start][entrablau-readme]
  for tenant prerequisites (admin role, Conditional Access caveats,
  `dmidecode` for realistic `dmiOverride` values).
- **Add graphics** — set `d2b.vms.work-entra.graphics.enable =
  true` in `flake.nix` and the VM gains a virtio-gpu + Wayland
  forward to the host compositor (a `foot` terminal auto-launches
  inside the guest on boot). Requires `d2b.site.waylandUser`
  to be non-null on the host — already set in this example.
- **Add YubiKey passthrough** — set
  `d2b.vms.work-entra.usbip.yubikey = true` and run
  `d2b usb attach work-entra <busid> --apply` to redirect a plugged YubiKey from the
  host's USB controller to the VM via USBIP. Useful for the MFA
  prompt during `aad-tool auth-test` and any downstream FIDO2
  flow.
- **BYOD / no Intune** — set `entrablau.joinType = "register"`
  and `entrablau.intuneCompliance.enable = false`. The TPM is
  still useful (PRT survival), but the compliance shim drops out
  of the picture.

## Platform support

This example targets **`x86_64-linux`**. The flake declares
`system = "x86_64-linux"` explicitly in
`nixosConfigurations.demo`.

The framework itself is multi-arch (headless VMs eval on
`aarch64-linux`); d2b's platform gate fires on `graphics.enable`
+ `audio.enable` only — **not on `tpm.enable`** — so a future
`aarch64`-clean variant of this example would be possible if
upstream Himmelblau gained an `aarch64` cargo build. Today,
`entrablau`'s TPM-enabled Himmelblau package is wired for
`x86_64-linux` only via its `himmelblauSystems` allowlist
(see the `entrablau` flake.nix), so the practical answer
remains `x86_64-linux` for the foreseeable future.

## Where the two flakes' docs disagree

If something in this example contradicts the option descriptions in
either upstream flake's README, **the option descriptions win**.
File an issue against this example's README and we'll bring it
back into sync.

## Common gotchas

- **TPM state backup**: do **not** wipe
  `/var/lib/d2b/vms/work-entra/swtpm/`. It holds the per-VM TPM
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
- **`d2b vm start work-entra --apply` before SSH/enrollment.** The Himmelblau
  service inside the VM doesn't start until the VM is up;
  attempting to enrol against a stopped VM hangs at the first
  device-code prompt.
- **Intune policy visibility depends on tenant configuration.**
  Whether you see Compliance / Conditional Access results in the
  Entra portal is a function of the tenant's MDM scope; the
  framework can't enforce policy visibility, only the
  authentication primitives.

## After subsequent rebuilds

`nixos-rebuild switch` updates the declared d2b bundle and may
restart `d2bd`, but daemon restarts are continuation events:
running VM runners are re-adopted rather than cycled. After rebuilding,
`d2b list` flags any VM whose declared closure has drifted from the
running one as `[pending restart]`; apply with `d2b vm restart
<vm> --apply`. See
[`templates/default/README.md` — After every subsequent rebuild](../../templates/default/README.md#after-every-subsequent-rebuild)
for the recommended workflow and
[`docs/reference/cli-contract.md`](../../docs/reference/cli-contract.md#pending-restart-signal-v015)
for the exact predicate.

## See also

- [`examples/minimal`](../minimal/) — read-and-copy headless starter
- [`examples/graphics-workstation`](../graphics-workstation/) — desktop VM with Wayland + audio + USBIP
- [`examples/multi-env`](../multi-env/) — two isolated envs (work + personal)
- [`templates/default`](../../templates/default/) — scaffold via `nix flake init`
- [`vicondoa/entrablau.nix`][entrablau] — the Entra/Himmelblau
  module bundle. Read its README for tenant prerequisites,
  detailed enrolment troubleshooting, and the full `entrablau.*`
  schema.
- [`vicondoa/d2b` README][d2b-readme] — quick start, common
  gotchas, full option index.

[d2b]: https://github.com/vicondoa/d2b
[d2b-readme]: ../../README.md
[entrablau]: https://github.com/vicondoa/entrablau.nix
[entrablau-readme]: https://github.com/vicondoa/entrablau.nix#readme
[himmelblau-tpm-maintaining]: https://github.com/vicondoa/entrablau.nix/blob/main/pkgs/himmelblau-tpm/MAINTAINING.md
