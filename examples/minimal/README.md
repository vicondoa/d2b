# Minimal host-local realm

This example declares one host-local realm and one headless NixOS workload.
It is the smallest useful d2b configuration and intentionally enables no
graphics, audio, TPM, or USB device providers.

## Configuration shape

Import `d2b.nixosModules.default`, declare a runtime provider, then bind the
workload to it:

```nix
d2b.realms.personal = {
  path = "personal";
  placement = "host-local";
  broker = {
    enable = true;
    hostMutation = true;
  };
  network = {
    mode = "declared";
    lanSubnet = "10.99.0.0/24";
    uplinkSubnet = "192.0.2.0/30";
  };
  providers.runtime = {
    type = "runtime";
    implementationId = "cloud-hypervisor";
  };
  workloads.personal-dev = {
    provider = "runtime";
    config = {
      networking.hostName = "personal-dev";
      users.users.alice = {
        isNormalUser = true;
        uid = 1000;
      };
    };
  };
};
```

The normalized realm and workload identifiers key the generated network,
storage, process, and allocator rows. PID1 owns only the fixed local-root
controller and broker units; workload processes are supervised by their realm
controller.

## Verify

```bash
nix flake check --no-build --all-systems
nix eval --no-write-lock-file \
  .#nixosConfigurations.demo.config.system.build.toplevel.drvPath
```

After activation, use the realm-aware CLI inspection and lifecycle commands
described in the root README. See `examples/multi-env` for two isolated realms
and `examples/graphics-workstation` for a desktop workload.
