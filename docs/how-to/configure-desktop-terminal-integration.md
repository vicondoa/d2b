# Configure desktop terminal integration

> Diataxis: how-to. Install the sibling desktop terminal flakes that consume
> d2b's public shell surface.

This recipe wires the optional desktop companions around persistent guest
shells:

- `d2b-toolkit` provides shared public-socket DTOs, client framing,
  redaction helpers, Wayland color parsing, and Waybar JSON helpers.
- `d2b-wlterm` is a Home Manager module and launcher for persistent d2b
  shell sessions.
- WeezTerm can be built from the `weezterm` flake when you want the terminal
  binary name and native d2b provider integration used by the launcher command.

The core d2b module does not import these flakes automatically. Keeping them
as sibling inputs preserves the one-way composition rule: desktop clients know
about d2b's public socket, while d2b does not depend on any particular bar,
launcher, or terminal emulator.

## Prerequisites

Enable persistent shells on every workload that should appear in the launcher.
During the v2 transition, keep the legacy VM runtime declaration and add a
realm workload row that points at it:

```nix
d2b.vms.work = {
  ssh.user = "alice";

  guest.control.enable = true;
  guest.exec.enable = true;
  guest.shell.enable = true;
};

d2b.realms.work.workloads.shellbox = {
  kind = "local-vm";
  legacyVmName = "work";
  launcher = {
    enable = true;
    label = "Work Shell";
    capabilities = [ "persistent-shell" "guest-exec" ];
  };
};
```

For the shell lifecycle model and CLI fallback commands, see
[Use persistent shells](./use-persistent-shells.md).

## Add aligned flake inputs

Use one `nixpkgs` input for the host and make every sibling follow it. Make the
desktop companions share the same `d2b-toolkit` input:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

    d2b = {
      url = "github:vicondoa/d2b";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    d2b-toolkit = {
      url = "github:vicondoa/d2b-toolkit";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    d2b-wlterm = {
      url = "github:vicondoa/d2b-wlterm";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.d2b-toolkit.follows = "d2b-toolkit";
    };

    weezterm = {
      url = "github:vicondoa/weezterm";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
}
```

When iterating locally, replace the `url` values with `path:/...` but keep each
input's existing `follows` lines. WeezTerm follows `nixpkgs` only because it
does not expose a toolkit input. This keeps package builds, Home Manager
evaluation, and toolkit path-dependency rewrites on compatible revisions.

## Import the Home Manager module

```nix
{ inputs, pkgs, ... }:
{
  imports = [ inputs.d2b-wlterm.homeManagerModules.default ];

  programs.d2b-wlterm = {
    enable = true;
    weztermCommand = [
      "${inputs.weezterm.packages.${pkgs.stdenv.hostPlatform.system}.default}/bin/weezterm"
      "start"
      "--"
    ];
    waybar.enable = true;
    waybar.injectHomeManager = true;
  };
}
```

The module renders `~/.config/d2b-wlterm/config.toml` and, when enabled,
injects the custom module into `programs.waybar.settings` when Home Manager also
manages Waybar. It also writes `~/.config/d2b-wlterm/waybar-module.json` for
operators who manage Waybar outside Home Manager. The upstream `d2b-wlterm`
flake has a `checks.<system>.home-manager-module` evaluation check that exercises
this rendered shape.

When realm workload metadata is present, d2b-wlterm groups shell-capable
workloads by realm and displays canonical targets such as
`shellbox.work.d2b`. It still uses the d2bd public socket and guest-control
capability checks; it does not read root-owned d2b bundle artifacts directly.

## Configure Waybar

When Waybar is managed by Home Manager, enable native injection instead of
copy-pasting JSON:

```nix
{
  programs.waybar.enable = true;
  programs.d2b-wlterm.waybar = {
    enable = true;
    injectHomeManager = true;
    barName = "mainBar";
    modulesList = "modules-right";
  };
}
```

If Waybar is managed elsewhere, import the generated
`~/.config/d2b-wlterm/waybar-module.json` manually and include
`custom/d2b-wlterm` in the desired module list.

## Validate

Run these checks from the consumer host flake:

```bash
nix flake check
home-manager build --flake .#alice
d2b shell work list
d2b-wlterm list work
d2b vm status shellbox.work.d2b
```

If `d2b-wlterm list` reports a typed shell capability error, confirm that the
VM has `guest.control.enable`, `guest.exec.enable`, and `guest.shell.enable`
set and that the VM was restarted after switching the host configuration.
