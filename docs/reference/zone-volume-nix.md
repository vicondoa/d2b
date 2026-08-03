# Zone and Volume Nix authoring

The canonical authoring surface is
`d2b.zones.<zone>.resources.<name>`. A Volume resource keeps its source
selection opaque and uses typed `LayoutEntry`, `ViewSpec`, and attachment
fields:

```nix
d2b.zones.local-root.resources.state = {
  type = "Volume";
  spec = {
    providerRef = "Provider/volume-local";
    source = {
      executionRef = "Host/host-system";
      settings = {
        kind = "local-path";
        sourcePolicyId = "state-root";
      };
    };
    kind = "state";
    layout = [{
      path = "";
      type = "directory";
      ownerRef = "User/d2bd";
      groupRef = "User/d2bd";
      mode = "0700";
    }];
    views.controller = {
      path = "";
      rights = [ "read" "write" "traverse" ];
    };
    attachments = [{
      executionRef = "Guest/work";
      transport = "virtiofs";
      view = "controller";
      access = "read-only";
      mountPath = "/state";
    }];
  };
};
```

Layout and symlink paths are anchored below the Volume root. Absolute paths,
parent components, backslashes, NULs, and path-separator homoglyphs are
rejected during evaluation. ACL principals are same-Zone `User` references.

The compiler emits a closure-only store-view Volume for each configured Guest.
Its root layout keeps `state/` and `gcroots/` at the store-view root, serves
only the `live/` view to the read-only virtiofs attachment, and never emits
`/nix/store` as a served path. TPM-enabled Guests receive a separate
fail-closed, secret-adjacent state Volume.

Zone hierarchy is authored with the compiler-only `parentZone` scalar.
Validated parent edges are sealed into allocator topology and are not emitted
in the runtime-created Zone self-resource. Child-local ZoneLinks must use a
same-Zone `Provider/transport-*` resource and an exact child Zone name.
Role resource verbs and ComponentSession verbs are closed sets; `relay` is
session-only and requires an exact bounded ZoneLink grant. RoleBindings have
no expiry field.

The eval-time refusals for `Zone`, `ZoneLink`, `Provider`, `Role`,
`RoleBinding`, `Quota`, and `EmergencyPolicy` are enumerated in
[Zone-control Nix authoring](./zone-control-nix.md).
