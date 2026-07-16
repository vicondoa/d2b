# Migrate d2b v1.2 to v2

**Diataxis category:** how-to.

This guide covers the v2 realm-native control-plane transition. It is written
for operators with an existing v1.2/v1.3 local d2b host and one or more local
VMs already running on `d2b.envs` / `d2b.vms.<vm>.env`.

## What changes

- Existing local VM names and `d2b.vms.<vm>.env` placement continue to work.
- The reserved `local` realm remains host-resident.
- `d2b.realms.<realm>` becomes the public declaration surface for realm intent,
  direct socket access metadata, provider declarations, policy references, and
  future realm-local lifecycle.
- `/etc/d2b/realm-controllers.json` records deterministic host-local realm
  controller metadata, local runtime providers/workloads, preserved VM paths,
  and capability summaries.
- Desktop-facing metadata now includes d2b-provided canonical realm targets for
  clipboard picker, Wayland proxy, and display-list JSON surfaces.
- Provider-managed sandboxes and remote full-host nodes fail closed when their
  advertised capability set does not match the selected provider model.

The migration is intentionally **metadata-first**. NixOS activation does not move
large VM state, rewrite `/var/lib/d2b/vms/<vm>`, or migrate network bridges.

## Before you start

1. Commit or stash local host configuration changes.
2. Back up `/var/lib/d2b`, especially per-VM state, store-view metadata, TPM
   state, media registry state, and daemon state.
3. Confirm the current host is healthy:

   ```bash
   d2b list --json
   d2b host doctor --read-only
   ```

4. Note every VM's current env:

   ```bash
   d2b list --json | jq '.[] | {name, env, runtimeKind}'
   ```

## Step 1: keep existing envs and VMs

Do not remove or rename `d2b.envs` or `d2b.vms.<vm>.env` during the v2 cutover.
Those declarations remain the active runtime substrate.

For example, keep this:

```nix
d2b.envs.work = {
  lanSubnet = "10.44.0.0/24";
  uplinkSubnet = "192.0.2.0/30";
};

d2b.vms.laptop = {
  env = "work";
  ssh.user = "alice";
};
```

Do not map distinct trust-boundary realms onto the same env unless you
intentionally want them on the same L2 broadcast domain. Today,
`d2b.realms.<realm>.env` and `network.envs` point at existing
`d2b.envs` bridges; sharing that env means sharing the bridge, net VM, DHCP/NAT
surface, and east-west policy. Work, personal, provider, and remote-host realms
that require isolation should use separate envs and L2 bridges.

## Step 2: add host-local realm metadata

Add a host-local realm that points at the existing env.  This records realm
intent and materializes deterministic host-local realm scaffolding without
moving VM state:

```nix
d2b.realms.work = {
  placement = "host-local";
  env = "work";
  network.envs = [ "work" ];
  allowedUsers = [ "alice" ];
  allowedGroups = [ "realm-work" ];
};
```

After `nixos-rebuild switch`, the framework emits
`/etc/d2b/realm-controllers.json`. Host-local rows include:

- deterministic realm daemon/broker unit names and principals;
- direct socket access metadata;
- local-root allocator binding metadata;
- `localRuntime.providers[]` for `local-cloud-hypervisor` and/or
  `local-qemu-media`;
- `localRuntime.workloads[]` rows preserving existing `/var/lib/d2b/vms/<vm>`,
  `/run/d2b/vms/<vm>`, store-view, and guest-control paths.

> **Migration warning**: if you declare `d2b.realms.work` with
> `network.envs = ["work"]` but leave `network.mode = "none"` (the default)
> and no `workloads`, the framework emits a soft advisory warning during
> `nixos-rebuild` pointing here.  This is intentional: it nudges you to
> complete the transition without blocking activation.

## Step 2a: optionally declare realm workloads (new v2 surface)

`d2b.realms.<realm>.workloads.<workload>` is the v2 public surface for
workload declarations.  It replaces `d2b.vms.<vm>`.  You can add workload
metadata at any time during the transition; the legacy `d2b.vms` entry stays
the active runtime substrate until you are ready to remove it.

Set `legacyVmName` to preserve the existing state path without any
activation-time migration:

```nix
d2b.realms.work = {
  placement = "host-local";
  env = "work";
  network.envs = [ "work" ];
  allowedUsers = [ "alice" ];

  workloads.laptop = {
    kind = "local-vm";
    legacyVmName = "laptop";    # maps stateDir → /var/lib/d2b/vms/laptop
    localVm.ssh.user = "alice";

    # Optional desktop-launcher metadata for Waybar, wlcontrol, wlterm,
    # clip-picker, and other realm-aware desktop consumers.
    launcher = {
      enable = true;
      label = "Work Laptop";
      icon.id = "computer-laptop";
      capabilities = [ "guest-exec" "graphics" ];
    };
  };
};
```

The workload `stateDir` defaults to `/var/lib/d2b/vms/<workload-id>`.  If
the workload id matches the legacy VM name (e.g. both are `laptop`) the
paths are identical and no data moves.  Setting `legacyVmName` is required
only when the realm workload id differs from the legacy VM name.

After this metadata is present, d2b emits
`/etc/d2b/realm-workloads-launcher.json`. Desktop consumers use that private
non-secret artifact (or public daemon surfaces derived from it) to group
launchers by realm, show canonical targets such as `laptop.work.d2b`, and
cluster duplicate app icons within one realm. The canonical target is also
accepted by local CLI status and guest-exec paths while the legacy VM remains
the underlying substrate:

```bash
d2b vm status laptop.work.d2b
d2b vm exec -d laptop.work.d2b -- firefox
```

`qemu-media` workloads use the same shape with `kind = "qemu-media"` and
`qemuMedia.*` options mirroring `d2b.vms.<vm>.qemuMedia.*`:

```nix
workloads.installer = {
  kind = "qemu-media";
  qemuMedia.source = {
    kind = "image-file";
    path = "/var/lib/d2b/images/fedora.iso";
    format = "iso";
  };
  launcher.enable = true;
  launcher.label = "Fedora Installer";
};
```

## Step 2b: optionally declare realm network (new v2 surface)

`d2b.realms.<realm>.network` with `mode = "declared"` is the v2 replacement
for `d2b.envs.<env>`.  Switching to it means d2b creates bridges + a net VM
under realm-derived names instead of env-derived names.  **Do not switch to
`mode = "declared"` until you are prepared for the interface and MAC address
change documented in Step 3.**

During the transition keep `mode = "none"` (default) or `mode = "inherit-env"`.
When you are ready to switch:

```nix
d2b.realms.work = {
  network.mode = "declared";
  network.lanSubnet = "10.44.0.0/24";   # from d2b.envs.work.lanSubnet
  network.uplinkSubnet = "192.0.2.0/30"; # from d2b.envs.work.uplinkSubnet

  # Optional: preserve MAC so upstream DHCP bindings do not change.
  # network.externalNetwork.attachment.macAddress = "<old-mac>";

  # Optional: port-forwards if you had d2b.envs.work.externalNetwork.portForwards.
  network.externalNetwork.portForwards = [{
    protocol = "tcp";
    listenPort = 2222;
    workload = "laptop";
    targetPort = 22;
  }];
};
```

Only remove `d2b.envs.work` after confirming the realm-declared network
works correctly.

## Step 3: rebuild and restart the daemon

> **⚠ Interface and MAC address warning**: If the realm network configuration
> changes bridge or interface names from legacy `d2b-<env>-*` forms to
> hash-derived realm names, any nftables rules or firewall policies that
> reference the old interface names will drift silently. Run
> `nft list ruleset | grep d2b` before and after the switch to confirm rules
> are correct.
>
> The net VM renamed from `sys-<env>-net` to `sys-<realm>-net` may also
> present a **different MAC address** to the uplink. If your router or DHCP
> server has a static binding for the old net VM MAC, that binding will no
> longer match after the rename. Options:
> - Set an explicit MAC address in your realm network declaration to preserve
>   the legacy value.
> - Update the upstream DHCP binding to the new MAC address.
> - Verify with `d2b list --json | jq '.[] | select(.kind == "net-vm") | .mac'`
>   before and after switching.

```bash
sudo nixos-rebuild switch
sudo systemctl restart d2bd.service
```

`d2bd` restarts are continuation events. Running VMs should remain running and
be re-adopted by the daemon. If you intentionally changed VM closures, restart
affected VMs through normal lifecycle commands after the daemon is ready.

For a production rollout, verify this on the host after the switch:

```bash
systemctl is-active d2bd.service
d2b status laptop
d2b vm status laptop.work.d2b
```

The static gates cover the service posture and metadata contract; live
running-VM adoption is still a host validation step because it depends on active
runner processes and pidfds.

## Step 4: verify realm metadata

Check the generated realm artifact:

```bash
sudo jq '.runtimeState, .controllers[].realmPath' /etc/d2b/realm-controllers.json
sudo jq '.controllers[] | {realmPath, placement, localRuntime}' /etc/d2b/realm-controllers.json
```

Expected properties:

- `runtimeState` is `metadata-only`.
- Host-local controllers have `placement = "host-local"`.
- `localRuntime.invariants.metadataOnly`,
  `existingGlobalVmPathsPreserved`, `noStateMigrationDuringActivation`, and
  `brokerEffectsRemainRealmDelegated` are all `true`.
- VM state paths still point at the existing per-VM roots.

Then verify CLI resolver behavior:

```bash
d2b realm list --json
d2b realm inspect work --json
```

Bare local VM commands still use the existing host fast path:

```bash
d2b status laptop
d2b vm restart laptop --apply
```

Use fully-qualified realm targets for realm-aware status, detached exec, and
desktop launchers:

```bash
d2b vm status laptop.work.d2b
d2b vm exec -d laptop.work.d2b -- true
d2b vm display list --target laptop.work.d2b --json
```

### Validation evidence in this repository

The migration guide relies on existing Layer-1 coverage:

- `tests/unit/nix/cases/realms.nix` checks that `realm-controllers.json`
  materializes host-local controller metadata, direct socket groups, local
  runtime rows, preserved `/var/lib/d2b/vms/<vm>` and `/run/d2b/vms/<vm>` paths,
  and the `metadataOnly`, `existingGlobalVmPathsPreserved`,
  `noStateMigrationDuringActivation`, and
  `brokerEffectsRemainRealmDelegated` invariants.
- `packages/d2b-core/tests/realm_controller_config.rs` parses and validates the
  typed `RealmControllersJson` DTO, including local runtime provider/workload
  rows and fail-closed invariant validation.
- `tests/unit/nix/cases/d2bd-startup-smoke.nix` checks daemon restart posture
  such as `restartIfChanged`, while `nixos-modules/host-daemon.nix` keeps the
  shutdown hook guarded so normal daemon restarts are not host-shutdown teardown.

These gates do not replace live host validation after a switch; they prove the
rendered configuration preserves the metadata-first contract that the host
validation exercises.

## Step 5: verify desktop metadata

Clipboard picker requests now include optional d2b-provided realm identity and
capability-preflight fields. The picker should treat these as trusted d2b
metadata and keep guest titles/app ids as presentation hints only.

Wayland proxy processes use the workload's canonical realm target when the
VM maps unambiguously to a realm workload. Existing app-id and title rewriting
still behaves as before:

- app ids are still prefixed as `d2b.<vm>.<guest-app-id>`;
- titles still receive `[<vm>] `;
- the realm target is separate trusted metadata for d2b-aware tooling.

The generated launcher metadata should also expose the canonical target:

```bash
sudo jq '.workloads[] | {legacyVmName, canonicalTarget, realmName, workloadName}' \
  /etc/d2b/realm-workloads-launcher.json
```

Display sessions listed with JSON include `canonicalTarget`, `identitySource`,
and `capabilityPreflight`:

```bash
d2b vm display list --json | jq '.sessions[] | {canonicalTarget, identitySource, capabilityPreflight}'
```

## Step 6: migrate provider-backed and remote realms deliberately

Provider-managed sandboxes and remote full-host nodes are separate models.
Do not reuse one as the other:

- Azure Container Apps sandboxes use the canonical
  `azure-container-apps` runtime provider. Its exact capabilities are plan,
  ensure, start, stop, inspect, adopt, and destroy; other provider authorities
  are not implied.
- Remote full-host nodes must be real full d2b hosts. Registration rejects
  provider-managed-isolation capability sets even if the node is mislabeled as a
  full host.

For provider-backed realms, keep provider credentials outside the host Nix
store. Declare only non-secret references in realm provider metadata:

```nix
d2b.realms.work.providers.aca = {
  kind = "aca";
  placement = "provider-agent";
  capabilityRefs = [ "aca" "relay" ];
  configRef = "work-aca-non-secret";
};
```

The realm provider fields are planning metadata in the current Nix schema.
They do not compose an ACA or Relay provider agent. Do not recreate the old
gateway files or enrollment flow, and do not fall back to SSH, raw provider
shells, raw guest-control tunnels, or host-held relay credentials.

## Step 7: cleanup only after verification

Do not delete old provider state during activation. Cleanup is an
operator action after verification:

1. Confirm all local VMs still start, stop, and report status.
2. Confirm realm metadata and desktop metadata look correct.
3. Confirm provider-managed sandboxes or remote full-host nodes re-enroll under
   the new provider model.
4. Remove only the obsolete non-secret config/state you can identify and restore
   from backup if needed.

Never delete TPM state casually. If TPM state is lost, follow the
TPM-specific recovery and IdP re-enrollment procedures rather than treating the
realm migration as recovery.

## Rollback

If the v2 metadata causes trouble before you have adopted provider/remote
realms:

1. Remove or disable the new `d2b.realms.<realm>` declaration.
2. Rebuild and restart `d2bd`.
3. Continue using existing `d2b.envs` and `d2b.vms.<vm>.env` declarations.

Because the migration does not move per-VM state during activation, rollback
does not require moving `/var/lib/d2b/vms/<vm>`.

## Troubleshooting

| Symptom | Meaning | Next step |
| --- | --- | --- |
| `realm-controllers.json` missing | The host did not build with realm metadata enabled. | Check `d2b.realms.*` declarations and rebuild. |
| `localRuntime` missing for a host-local realm | No enabled VM is associated with the realm's existing env. | Check `d2b.realms.<realm>.env` and `d2b.vms.<vm>.env`. |
| Provider sandbox denies `persistent-shell` | The sandbox image does not report a guestd-compatible agent. | Rebuild/re-enroll the sandbox with the provider-agent contract; do not use provider shell fallback. |
| Remote full-host registration rejected as `not-full-host` | The node is not a full d2b host or advertises provider-managed isolation. | Register it as provider-managed, or install full d2b on the remote host. |
| Desktop helper shows no canonical target | The helper is older than the v2 metadata contract or the source is host-only. | Update the helper; host clipboard sources intentionally have no VM target. |

## Related references

- [Realm option schema](../reference/realm-options.md)
- [Realm controller configuration](../reference/realm-controller-config.md)
- [Realm access resolver](../reference/realm-access-resolver.md)
- [Provider-managed sandboxes](../reference/provider-managed-sandboxes.md)
- [Remote full-host nodes](../reference/remote-full-host-nodes.md)
- [Display and virtual I/O capabilities](../reference/display-io-capabilities.md)
- [Clipboard picker protocol](../reference/clipboard-picker-protocol.md)
