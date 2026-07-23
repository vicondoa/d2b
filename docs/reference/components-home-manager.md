# `d2b.vms.<vm>.homeManager.*`

> Reference for the `home-manager` component module.
> Source: [`nixos-modules/components/home-manager.nix`](../../nixos-modules/components/home-manager.nix)
> Host-side propagation: [`nixos-modules/host.nix`](../../nixos-modules/host.nix)

## What this component does

Imports [Home Manager] into the guest **as a NixOS module** (the
same composition the framework itself documents for the host), pre-
wires it with the framework's sensible defaults (`useGlobalPkgs`,
`useUserPackages`, `backupFileExtension = "hm-backup"`, `inputs` in
`extraSpecialArgs`), and exposes a `d2b.homeManager.users`
attrset whose per-user values are forwarded into upstream
`home-manager.users`. One `nixos-rebuild switch` (or, for
VM-only changes, one `d2b switch <vm> --apply`) rebuilds the guest's
system + home environment atomically — there is no separate
`home-manager switch` invocation inside the VM.

[Home Manager]: https://github.com/nix-community/home-manager

## Options (host-side)

| Option | Type | Default | Description |
|---|---|---|---|
| `d2b.vms.<vm>.homeManager.enable` | bool | `false` | Import Home Manager into this VM. Pulls in [`components/home-manager.nix`](../../nixos-modules/components/home-manager.nix) which loads `inputs.home-manager.nixosModules.home-manager` and applies the framework's HM defaults. |
| `d2b.vms.<vm>.homeManager.users` | attrs (unspecified) | `{ }` | Per-user HM config attrsets. Propagated host→guest by `host.nix` into the guest's `d2b.homeManager.users`. Each value is a NixOS HM module (see "Expected user-value shape" below). |

## Options (guest-side propagation)

`host.nix` forwards the host-side `homeManager.users` attrset into
the guest config under a `mkIf` gate:

```nix
(lib.mkIf vm'.homeManager.enable {
  d2b.homeManager.users = vm'.homeManager.users;
})
```

The matching guest-visible option (declared in
`components/home-manager.nix`):

| Option | Type | Default | Description |
|---|---|---|---|
| `d2b.homeManager.users` | attrs (unspecified) | `{ }` | Per-user HM config attrsets, populated by `host.nix` from the host-side option. Each value is fed straight into upstream `home-manager.users.<user>`. |

### Expected user-value shape

Each value is a NixOS Home Manager module. The minimum surface the
framework expects (lifted verbatim from the module's docstring):

```nix
{
  alice = {
    imports = [ ./home/alice/core.nix ];
    home.username = "alice";
    home.homeDirectory = "/home/alice";
    home.stateVersion = "25.11";
  };
}
```

`extraSpecialArgs = { inherit inputs; }` is set by the component,
so any HM module imported here can use `inputs` in its arguments
without further plumbing.

## Host-side resources created

None. The component module is imported only into the **guest's**
config (`++ lib.optional vm'.homeManager.enable
./components/home-manager.nix` in `host.nix`). There is no
per-VM systemd unit, no per-VM system user, no state directory on
the host beyond what other components already create.

## Guest-side resources created

- `imports = [ inputs.home-manager.nixosModules.home-manager ]` —
  upstream HM as a NixOS module.
- `home-manager.useGlobalPkgs = true` — share nixpkgs with the
  guest system; no separate HM nixpkgs evaluation.
- `home-manager.useUserPackages = true` — user packages flow via
  `users.users.<name>.packages`.
- `home-manager.backupFileExtension = "hm-backup"` — on first
  application, any conflicting pre-existing dotfile is renamed to
  `<file>.hm-backup` rather than failing the activation.
- `home-manager.extraSpecialArgs = { inherit inputs; }` — the
  consumer's flake `inputs` is available inside every HM module's
  argument set.
- `home-manager.users = config.d2b.homeManager.users` — the
  propagated attrset.

## Runtime invariants

- HM activation runs as part of guest `nixos-rebuild switch`. There
  is no separate `home-manager` binary to invoke and no separate
  switch step.
- `home-manager.useGlobalPkgs = true` guarantees the guest's HM and
  system see the same nixpkgs; overlays declared in the consumer's
  flake apply to both.
- `backupFileExtension` is `"hm-backup"` — never `.bak` (which would
  collide with other tooling). After the first activation on a
  fresh VM, expect a handful of `.hm-backup` files in the relevant
  users' homes; inspect, then delete.

## Hardening notes

The Home Manager component runs entirely inside the guest as part
of the system's own activation flow. It does not spawn any new
privileged host-side process, does not bind any new socket, and
does not add any new host UID/GID. All sandboxing relevant to the
guest itself is inherited from the surrounding microVM boundary —
HM's `useUserPackages = true` puts user packages in
`/etc/profiles/per-user/<name>/` which the guest activation already
manages with the usual NixOS guarantees.

## Common gotchas / failure modes

- **`home.stateVersion` mismatch / forgotten.** Every HM user
  module needs `home.stateVersion`. Forgetting it produces a
  loud HM eval error pointing at the user. Match the guest's
  `system.stateVersion` unless you have a reason to diverge.
- **Untracked files invisible to flakes.** New files under
  `home/<user>/` must be `git add`ed before they participate in
  the build. The #1 "why didn't my change apply?" pitfall —
  exactly the same as for top-level NixOS modules.
- **`.hm-backup` files after first activation.** Expected; HM moves
  conflicting pre-existing dotfiles aside instead of failing. Diff
  against the new HM-managed copy, delete once you're satisfied.
- **`home.file` content is read-only.** Files materialised via
  `home.file` are symlinks into `/nix/store`. Apps that write back
  to their own config will either fail or create their own
  `.bak` files. For live-editable dotfiles, use
  `config.lib.file.mkOutOfStoreSymlink` to point straight at the
  source-tree path instead — same pattern as the framework's own
  AGENTS.md describes for the host.
- **Secrets in HM.** Not in scope for the v0.1.0 component. Use
  `sops-nix` (multi-secret) or `agenix` per-VM via
  `d2b.vms.<vm>.config.imports = [ … ];` — the component does
  not auto-import either.

## See also

- [Design / threat model](../explanation/design.md)
- [Manifest schema](./manifest-schema.md)
- [Home Manager — NixOS module flavour][hm-nixos]
- [`examples/minimal`](../../examples/minimal/) and
  [`examples/graphics-workstation`](../../examples/graphics-workstation/)
  — both demonstrate consuming the component.

[hm-nixos]: https://nix-community.github.io/home-manager/index.xhtml#sec-install-nixos-module
