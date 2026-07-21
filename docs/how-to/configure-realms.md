# Configure realms

**Diataxis category:** how-to.

Declare the destructive-cutover acknowledgement, then define providers and
provider-bound workloads inside each realm:

```nix
{
  d2b.acceptDestructiveV2Cutover = true;

  d2b.realms.work = {
    placement = "host-local";
    allowedUsers = [ "alice" ];

    providers.local-runtime = {
      type = "runtime";
      implementationId = "cloud-hypervisor";
    };

    providers.local-network = {
      type = "network";
      implementationId = "local-realm";
    };

    workloads.laptop = {
      provider = "local-runtime";
      autostart = true;
      config = {
        networking.hostName = "laptop";
      };
      launcher = {
        enable = true;
        label = "Work laptop";
        items.terminal = {
          type = "shell";
          icon.name = "terminal";
        };
      };
    };

    network = {
      mode = "declared";
      lanSubnet = "10.44.0.0/24";
      uplinkSubnet = "192.0.2.0/30";
    };
  };
}
```

Provider instance names are realm-local. Each provider declares exactly one
primary `type` and one `implementationId`; each workload selects an enabled
runtime provider through `provider`.

Do not add old VM, env, gateway, relay, `legacyVmName`, inherit-env, or
provider-placeholder declarations. They are unknown options rather than
migration aliases.
