# How to install nixling on Fedora

This is the current Fedora Tier-1-later / best-effort manual install path.
Like the Ubuntu walkthrough, it is a scaffold-oriented guide for the current
host-install flow rather than a polished GA installer.

## 1. Install Fedora prerequisites

```bash
sudo dnf install -y \
  curl git jq xz \
  openssh-clients openssh-server \
  iproute nftables NetworkManager \
  kmod util-linux usbutils
```

If you need USBIP tooling for YubiKey validation, also install the Fedora
package that ships `usbip` on your image (commonly `kernel-tools`).

## 2. Install Nix (multi-user)

```bash
curl -fsSL https://install.determinate.systems/nix | sh -s -- install
exec "$SHELL" -l
```

Verify:

```bash
nix --version
```

## 3. Install the nixling CLI

```bash
nix profile install github:vicondoa/nixling#nixling
nixling --help >/dev/null
```

## 4. Run the host-install scaffold

```bash
sudo nixling host install --dry-run --enable --start
sudo nixling host install --apply --enable --start
```

Inspect the written artifacts under `/etc/systemd/system/`, `/etc/nixling/`,
and `/var/lib/nixling/runtime/` before assuming you have a production-ready
service install.

## 5. Validate host prerequisites

```bash
nixling host check --strict
nixling host prepare --dry-run --json
```

Fedora uses the same daemon-facing first-boot model as Ubuntu: the practical
operator path is usually `vm start --apply` after a clean dry-run, while the
standalone public `host prepare --apply` surface is still a separately staged
command.

## 6. Land the trusted bundle

Place the full trusted bundle at the daemon's configured bundle root. On the
current host-install scaffold, that means:

```text
/var/lib/nixling/current-bundle/manifest.json
```

Use [`../reference/manifest-bundle.md`](../reference/manifest-bundle.md) as the
source of truth for the bundle artifact set.

## 7. Plan and start the first VM

```bash
nixling vm start work-vm --dry-run --json
sudo nixling vm start work-vm --apply
```

If the guest comes up, finish the SSH trust handshake:

```bash
nixling trust work-vm
nixling status work-vm
```

## 8. Fedora-specific notes

- group names for the important device nodes match the support matrix defaults
  (`kvm`, `fuse`);
- some Fedora hosts need an explicit `vhost_net` load on first boot — let
  `host check --strict` be the deciding signal;
- keep the `launcherUsers` / `adminUsers` sets small; on the current public
  socket, that boundary is what matters most.

## See also

- [`install-ubuntu-2404.md`](./install-ubuntu-2404.md)
- [`../reference/support-matrix.md`](../reference/support-matrix.md)
- [`../reference/manifest-bundle.md`](../reference/manifest-bundle.md)
