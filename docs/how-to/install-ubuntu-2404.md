# How to install nixling on Ubuntu 24.04

This is the current Ubuntu 24.04 Tier-1 **manual/scaffold** path.

It gets you to the point where:

- the distro prerequisites are present;
- Nix is installed in multi-user mode;
- the `nixling` CLI is installed;
- the host-install scaffold has laid down its expected artifact paths; and
- you can dry-run the first VM lifecycle through the daemon-facing CLI.

The current host-install writer is still a staging path, so treat this guide as
an operator checklist, not a GA one-command installer.

## 1. Install Ubuntu prerequisites

```bash
sudo apt update
sudo apt install -y \
  curl git jq xz-utils \
  openssh-client openssh-server \
  iproute2 nftables network-manager \
  kmod util-linux usbutils
```

If you plan to validate USBIP/YubiKey flows, also install the Ubuntu package
that provides `usbip` on your image (`linux-tools-generic` on the common 24.04
images).

## 2. Install Nix (multi-user)

The existing how-to and smoke harnesses use the Determinate Systems installer:

```bash
curl -fsSL https://install.determinate.systems/nix | sh -s -- install
exec "$SHELL" -l
```

Confirm the daemon install succeeded:

```bash
nix --version
```

## 3. Install the nixling CLI

```bash
nix profile install github:vicondoa/nixling#nixling
nixling --help >/dev/null
```

## 4. Lay down the host-install scaffold

Plan first, then apply:

```bash
sudo nixling host install --dry-run --enable --start
sudo nixling host install --apply --enable --start
```

Today's scaffold writes the expected host artifact paths and exercises the
broker install path, but you should still inspect what landed under:

- `/etc/systemd/system/nixlingd.service`
- `/etc/nixling/daemon-config.json`
- `/var/lib/nixling/runtime/host-runtime.json`

## 5. Check the host before touching a VM

```bash
nixling host check --strict
nixling host prepare --dry-run --json
```

On Ubuntu, the usual first-boot path is `vm start --apply`; the standalone
`host prepare --apply` surface is still a separately staged public command.
`host prepare --dry-run` is still useful because it shows you what the daemon
expects to reconcile.

## 6. Land a trusted bundle

The daemon/broker path needs a trusted bundle before a live `--apply` can do
anything useful. On the current host-install path, the default bundle root is:

```text
/var/lib/nixling/current-bundle/manifest.json
```

Copy the full bundle described in
[`../reference/manifest-bundle.md`](../reference/manifest-bundle.md) there,
then restart the daemon if you changed the bundle after install.

## 7. Plan the first VM

With the bundle in place, dry-run the first VM before applying:

```bash
nixling vm start work-vm --dry-run --json
```

If the dry-run looks correct and the host checks are clean, attempt the real
start:

```bash
sudo nixling vm start work-vm --apply
```

After the guest is reachable, finish the SSH trust step:

```bash
nixling trust work-vm
nixling status work-vm
```

## 8. Troubleshoot the common failures

### `host install --apply` wrote files but the service is not usable

That is the current scaffold state. Compare what landed with the fixture notes
in `tests/integration/distro-matrix/fixtures/ubuntu-2404/` and replace placeholder content
with the real generated unit/config artifacts for your deployment.

### `vm start --apply` says the bundle is missing

Land the trusted bundle at `/var/lib/nixling/current-bundle/manifest.json`
(and its sibling artifacts), then restart the daemon.

### `host check --strict` complains about KVM, `tun`, `fuse`, or `vhost_net`

Fix the host prerequisite first; the daemon path will not paper over missing
kernel/device state.

## See also

- [`install-fedora.md`](./install-fedora.md)
- [`headless-alpha-walkthrough.md`](./headless-alpha-walkthrough.md)
- [`../reference/support-matrix.md`](../reference/support-matrix.md)
- [`../reference/manifest-bundle.md`](../reference/manifest-bundle.md)
