# Configure a clipboard picker

**Diataxis category:** how-to.

D2b does not include a default picker flake input. Install the separate
`d2b-clip-picker` project and pass its package or binary path explicitly.

## With a picker flake input

```nix
{
  inputs.d2b-clip-picker.url = "github:vicondoa/d2b-clip-picker";

  outputs = { nixpkgs, d2b, d2b-clip-picker, ... }: {
    nixosConfigurations.host = nixpkgs.lib.nixosSystem {
      modules = [
        d2b.nixosModules.default
        ({ pkgs, ... }: {
          users.users.alice = { isNormalUser = true; uid = 1000; };
          d2b.site = {
            waylandUser = "alice";
            clipboard = {
              enable = true;
              niri.external = true;
              clipd.executablePath = "/run/current-system/sw/bin/d2b-clipd";
              # Or set clipd.package once the daemon package is wired.
              picker.package = d2b-clip-picker.packages.${pkgs.system}.default;
              policy.crossRealm.enable = true;
              niri.fallback.enable = true;
            };
          };
        })
      ];
    };
  };
}
```

If niri is declared through NixOS, use `programs.niri.enable = true` instead of
`d2b.site.clipboard.niri.external = true`.

## Explicit paste action keybind

When no trusted no-patch Niri paste-intent hook is available, keep native host
cross-realm popups disabled and use the explicit d2b paste action:

```nix
d2b.site.clipboard = {
  niri.fallback.enable = true;
  modes.hostCrossRealmPicker = true;
};
```

Bind `d2b.site.clipboard.niri.fallback.command` (default:
`d2b clipboard arm`) in niri. The command opens the picker for the currently
focused target. After selection, `d2b-clipd` publishes
the chosen payload as a d2b-owned host selection and triggers the paste replay;
the picker itself still never writes to the clipboard.

## Probe the session clipboard

Use `d2b-clip-debug` from the Wayland session you want to inspect. It exercises
only the standard unprivileged Wayland clipboard protocol:

```bash
d2b-clip-debug wl-copy "hello from this Wayland session"
d2b-clip-debug wl-paste text/plain
```

These probes do not talk to the picker protocol, do not receive privileged
data-control globals, and do not bypass `d2b-clipd` for VM boundary transfers.

## What the picker must not receive

- no `NIRI_SOCKET`;
- no Wayland transfer FDs;
- no data-control or primary-selection authority;
- no virtual-keyboard or input-synthesis permission;
- no persistence or policy authority.

The picker talks only over its inherited socketpair and sends `Select` or
`Cancel` for the current request.
