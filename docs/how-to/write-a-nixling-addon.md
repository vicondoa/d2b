# Writing a nixling addon

> How-to: publish a sibling flake that composes with `nixling` on one
> VM at a time.
>
> Reading time: ~10 minutes.
> Difficulty: intermediate.

## What's an addon?

A `nixling` addon is a **sibling flake** that exports an ordinary NixOS
module and gets composed into a specific guest VM. It is not a special
plugin API.

[`vicondoa/nixos-entra-id`](https://github.com/vicondoa/nixos-entra-id)
is the canonical example: the Entra-specific module lives outside the
framework, and the consumer imports it only into the VM that needs it.
See [`examples/with-entra-id/`](../../examples/with-entra-id/) for the
concrete composition pattern.

## Step 1: Keep the addon framework-agnostic

Your addon should look like a normal flake that exposes a NixOS module,
for example `nixosModules.default`. It should not need a `nixling`
input just to define its option schema or guest-side behavior.

The contract is deliberately small:

- `nixling` owns the host-side VM lifecycle, sidecars, networking, and
  CLI.
- The addon owns guest-side workload behavior.
- The consumer composes the two on a per-VM basis.

## Step 2: Make the consumer align `nixpkgs`

Addons should evaluate against the **same `nixpkgs` as the consumer's
`nixling`**. Do that in the **consumer flake**, not inside the addon.

```nix
inputs = {
  nixling.url = "github:vicondoa/nixling/v0.3.0";
  my-addon.url = "github:example/my-addon";

  my-addon.inputs.nixpkgs.follows = "nixling/nixpkgs";
};
```

Do **not** make the addon depend on `nixling` just to inherit its
`nixpkgs`. That would couple an ordinary NixOS module bundle to one
specific framework. The addon should stay reusable outside `nixling`.

## Step 3: Import it at the per-VM seam

The composition point is the VM's guest-module import list:

```nix
nixling.vms.<vm>.config.imports = [
  inputs.<addon>.nixosModules.default
];
```

That imported module runs inside a standard NixOS module context for the
**guest**. From the addon's point of view this is just normal NixOS
module evaluation: `lib`, `pkgs`, `config`, `options`, `modulesPath`,
and any consumer-provided `specialArgs` behave the same way they would
on bare metal.

The `examples/with-entra-id/flake.nix` example shows the common pattern:

```nix
nixling.vms.work-vm = {
  enable = true;
  tpm.enable = true;
  env = "work";
  index = 10;
  ssh.user = "alice";

  config.imports = [
    nixos-entra-id.nixosModules.default
    ./work-vm.nix
  ];
};
```

## Step 4: Use your own option namespace

**NEVER** declare addon options under `nixling.*`. That namespace
belongs to the framework.

Use your own namespace instead, for example:

- `entraId.*`
- `myAddon.*`
- `corpVpn.*`

That keeps ownership clear and avoids collisions with future framework
options.

## Step 5: Add an eval-only test

A minimal addon test can stay purely at evaluation time. The goal is to
prove that your module and `nixling` can be imported into one NixOS
module graph without errors.

```nix
# test-eval.nix
(import <nixpkgs/nixos/lib/eval-config.nix> {
  modules = [
    inputs.nixling.nixosModules.default
    {
      nixling.vms.test-vm.config.imports = [
        inputs.my-addon.nixosModules.default
      ];

      # minimal config
    }
  ];
}).config
```

Assume `inputs` is in scope the same way it would be in your flake's
`checks` or a tiny harness expression. Keep the fixture minimal: one VM,
one env, and only the addon options needed to reach the code path you
care about.

## Step 6: Cross-link the README

An addon README should link back to the main
[`nixling` README](../../README.md) and say which `nixling` version or
version range it was tested against.

Good README conventions:

- link to `nixling` for the host-framework concepts the addon does not
  re-explain;
- name the concrete `nixling` release used during validation;
- point at a minimal consumer example if the addon ships one.

## Step 7: Version independently

Addons are versioned independently from `nixling`. They do not need to
share tag numbers.

The real compatibility contract is the NixOS module system provided by
`nixpkgs`. If the consumer pins a `nixpkgs` revision where both
`nixling` and the addon evaluate cleanly, the version numbers do not
need to match.

## Checklist

Before you publish an addon release:

- export a normal `nixosModules.default`;
- keep the addon out of the `nixling.*` namespace;
- document the consumer-side `nixpkgs` follow rule;
- show the `nixling.vms.<vm>.config.imports` seam in the README;
- keep at least one eval-only test in CI.

## Host-prepare addon hook contract

Host prepare uses a privileged broker that the daemon uses to reconcile
host state (cgroup delegation, bridges/TAPs, NetworkManager
unmanaged config, `/etc/hosts` managed block, `inet nixling`
nftables table, per-link sysctls, `modprobe`, device-node opens).
**Addons MUST NOT bypass the broker** — that is, an addon may not
ship a privileged systemd service that mutates any of those host
surfaces directly. Instead:

- If an addon needs a kernel module loaded, declare it in the
  consumer-side `nixling.site.modules` allow-list so the module
  matrix can audit and probe it via the `ModprobeIfAllowed` broker
  variant (ADR 0014). Do not ship a `boot.kernelModules` entry that
  silently bypasses the matrix.
- If an addon needs an extra per-link sysctl, declare it as a typed
  `SysctlIntent` on the addon's per-VM module so the broker can
  apply it via `ApplySysctl` and audit the before/after value. Do
  not write to `/proc/sys/` from an `ExecStartPre`.
- If an addon needs a NetworkManager unmanaged-config row or an
  `/etc/hosts` managed-block entry, surface it as a typed
  `NmUnmanagedEntry` / `HostsEntry` on the per-VM module. The
  broker applies it under the nixling ownership marker; addon-owned
  ownership markers are rejected (`nm-managed-foreign-conflict`).
- Addons that need an extra firewall rule beyond `inet nixling`'s
  four default chains must declare it as a typed extension on the
  per-env config. The broker preserves foreign nftables tables
  (per ADR 0013), but addon-emitted rules inside `inet nixling`
  must go through the typed extension so the audit log records
  ownership.

If your addon legitimately cannot be expressed inside the closed
broker enum, open an issue: extending the enum is an explicit
panel-gated decision (ADR-required), and a privileged sidecar that
bypasses the broker would silently weaken the host-prepare trust
boundary described in [`SECURITY.md`](../../SECURITY.md) §
host-prepare trust-boundary delta.

See [`docs/reference/privileges.md`](../reference/privileges.md) for
the full broker operation matrix and
[`docs/explanation/host-prepare.md`](../explanation/host-prepare.md)
for the conceptual model + recovery runbook.
