# Home Manager in a Guest

**Diataxis category:** reference.

Home Manager is consumer-owned Guest configuration. It belongs in the NixOS
evaluation supplied through `d2b.guestSystems.<zone>.<guest>`, not in the host
Zone controller or broker configuration.

```nix
d2b.guestSystems.work.work-app = {
  config = {
    imports = [ inputs.home-manager.nixosModules.home-manager ];
    home-manager.users.alice = {
      home.stateVersion = "25.11";
    };
  };
};
```

The host still declares only the semantic `Guest/work-app` Resource and its
selected immutable system artifact. Guest lifecycle, child resources, and
host effects remain owned by d2bd and specialized Providers.
