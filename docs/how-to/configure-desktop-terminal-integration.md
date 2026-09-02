# Configure desktop terminal integration

Desktop companions consume d2b's public Zone and shell contracts. d2b does
not import a particular bar, launcher, terminal, or Home Manager module.

## Declare a shell-capable Guest

The host declares a Guest Resource and supplies its evaluator:

```nix
d2b.zones.work.resources.work-app = {
  type = "Guest";
  spec = {
    providerRef = "Provider/runtime-cloud-hypervisor";
    systemArtifactId = "work-guest-system";
  };
};

d2b.guestSystems.work.work-app = inputs.workGuestSystem;
```

The Guest controller owns child Process and Endpoint resources. Shell
capability is admitted only after the authenticated ComponentSession is ready.

## Compose a companion

Keep one `nixpkgs` input and compose a desktop companion as a sibling flake:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    d2b.url = "github:vicondoa/d2b";
    d2b.inputs.nixpkgs.follows = "nixpkgs";
    d2b-wlterm.url = "github:vicondoa/d2b-wlterm";
    d2b-wlterm.inputs.nixpkgs.follows = "nixpkgs";
  };
}
```

The companion should use only the public socket, typed ResourceRefs, and
bounded shell/status responses. It must not read private bundle files or
reconstruct host paths, credentials, argv, or runtime scope.

## Validate

```bash
nix flake check
d2b shell list --zone work
d2b shell open Guest/work-app --zone work --name terminal
d2b guest status work-app --zone work
```

Render a visible unavailable state when the Zone, Guest, Provider, or shell
session is not ready. Do not retry through SSH or a legacy launcher path.
