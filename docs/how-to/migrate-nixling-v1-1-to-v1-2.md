# Migrate nixling v1.1 to v1.2

## TL;DR — required steps for operators

For most v1.1 hosts the upgrade is two commands:

```bash
sudo nixos-rebuild switch
sudo systemctl restart nixlingd.service
```

After these two steps, all VMs continue working with no further
operator action. If you have users with
`extraGroups = [ "nixling-launcher" ]` in your `/etc/nixos` config,
you must additionally update that to `[ "nixling" ]` before
`nixos-rebuild` — see "What consumers must change" below.

v1.2 unifies the host-side lifecycle Unix groups
`nixling-launcher` and `nixling-launchers` into one canonical
`nixling` group. For most operators this is transparent: the v1.2
activation helper re-chgrps `/var/lib/nixling` and `/run/nixling`
state by numeric legacy gid during the next `nixos-rebuild switch`.

## What consumers must change

Search your host configuration for legacy group references:

```bash
rg -n '"nixling-launcher(s)?"|\bnixling-launcher(s)?\b' \
  /etc/nixos /etc/nixos/flake.nix
```

Update user memberships from the legacy names to `nixling`:

```nix
# before
users.users.alice.extraGroups = [ "nixling-launcher" ];

# after
users.users.alice.extraGroups = [ "nixling" ];
```

If this is missed:

**Symptom**: `nixling vm <op>` fails with `permission denied`
(daemon-side rejection during public-socket SO_PEERCRED gate).

**Recovery**:

1. Edit `/etc/nixos/configuration.nix`: change
   `extraGroups = [ "nixling-launcher" ];` →
   `extraGroups = [ "nixling" ];`.
2. `sudo nixos-rebuild switch`
3. `sudo systemctl restart nixlingd.service`
4. Affected users may need to log out + back in to pick up the
   new group membership in their login session.

## Required post-switch step

After switching to v1.2, restart the long-lived daemon once so it picks
up the new socket group and daemon config:

```bash
sudo systemctl restart nixlingd.service
```

Phase A and Phase B ship together as v1.2, so one restart after the
v1.2 switch is sufficient. If you intentionally split them into point
releases, restart after each switch that changes daemon code or socket
group configuration.

## Custom keysDir override

The fd-safe helper migrates `/var/lib/nixling` and `/run/nixling`. If
you have a custom `nixling.site.keysDir` outside those roots, migrate it
after the switch by numeric gid. Example:

```bash
legacy_gid=$(getent group nixling-launcher | cut -d: -f3)
[ -n "$legacy_gid" ] && sudo find /custom/nixling/keys -xdev -gid "$legacy_gid" -exec chgrp nixling {} +
```

`find -exec chgrp` is acceptable here as an operator one-liner for a
trusted custom directory. The built-in activation path uses the fd-safe
helper so framework-owned roots avoid symlink races.

## Tombstones

The legacy `nixling-launcher` and `nixling-launchers` Unix groups remain
on the system in v1.2 as empty migration tombstones: zero membership,
gid preserved in `/etc/group`. `getent group nixling-launcher` may still
return a record with an empty member list. The tombstones let the
activation helper find legacy numeric gids on direct upgrades; they are
slated for removal in a v1.3 follow-up (see the CHANGELOG deferred row).

## Audit label stability

The broker caller-role audit label remains `"nixling-launcher"` for
format stability. That string is an audit/authz class identifier, not a
Unix group lookup. See
[`docs/reference/naming-conventions.md`](../reference/naming-conventions.md#broker-caller-role-audit-labels).
