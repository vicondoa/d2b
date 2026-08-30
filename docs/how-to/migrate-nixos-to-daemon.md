# Migrate a host to the Zone daemon

New deployments use the daemon-only Zone control plane. This guide is a
short migration checklist for hosts that still carry older lifecycle
configuration.

## 1. Back up and inspect

Keep a known-good NixOS generation and back up d2b state. Then inspect the
current host without mutation:

```bash
d2b host check --json
d2b auth status --json
d2b zone list
```

## 2. Declare current resources

Replace legacy hierarchy declarations with Zone-owned Resources and immutable
artifacts. A Guest's evaluator belongs under
`d2b.guestSystems.<zone>.<guest>`; the Guest controller owns its child graph.

```nix
d2b.zones.work.resources.work-app = {
  type = "Guest";
  spec = {
    providerRef = "Provider/runtime-cloud-hypervisor";
    systemArtifactId = "work-guest-system";
  };
};
```

Remove old owner configuration only after the corresponding Zone resources
and Provider assignments are present. Do not preserve a compatibility service
as a second lifecycle authority.

## 3. Switch and verify

```bash
nixos-rebuild switch --flake .#desktop
d2b guest status work-app --zone work
d2b guest start work-app --zone work --dry-run
d2b guest start work-app --zone work --apply
d2b host doctor --read-only
```

The only framework root units are `d2bd.service`,
`d2b-broker.socket`, and `d2b-broker.service`. A daemon restart adopts
matching current runners; stale identity is quarantined.

## 4. Roll back

If the new generation is unhealthy, boot or activate the known-good NixOS
generation and keep the old d2b state intact. Do not delete TPM, lock, cgroup,
or Guest store state as a shortcut. Re-run read-only checks before retrying.

Historical v0/v1/v2 migration pages retain detailed old option names for
archival context. They are not current configuration instructions.
