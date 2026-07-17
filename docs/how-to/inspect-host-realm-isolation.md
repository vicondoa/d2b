# Inspect host realm isolation

**Diataxis category:** how-to.

Use the installed private bundle artifacts to inspect the declarative
child-realm allocation plan. These checks do not prove that runtime allocation
or spawning has occurred.

1. List child realm identities and endpoint paths:

   ```bash
   sudo jq '.controllers[] |
     {realmId, controller: .daemon.user, broker: .broker.user,
      public: .sockets.publicSocketPath, brokerSocket: .sockets.brokerSocketPath}' \
     /etc/d2b/realm-controllers.json
   ```

2. Confirm no child row claims PID1 materialization:

   ```bash
   sudo jq -e '
     .invariants.noSystemdUnitsMaterialized and
     all(.controllers[];
       (.daemon.materializedService | not) and
       (.broker.materializedSocket | not) and
       (.broker.materializedService | not))
   ' /etc/d2b/realm-controllers.json
   ```

3. Inspect the ordered typed allocator requests:

   ```bash
   sudo jq '.resourceRequests |
     sort_by(.realmPath, .acquisitionOrder.phase, .acquisitionOrder.ordinal,
             .kind, .resourceId) |
     map({realmPath, resourceId, kind, share, acquisitionOrder})' \
     /etc/d2b/allocator.json
   ```

4. Confirm identity configuration contains references and fingerprints only:

   ```bash
   sudo jq -e '
     all(.. | objects;
       (has("privateKey") or has("credentialMaterial") or has("providerToken"))
       | not)
   ' /etc/d2b/realm-identity.json
   ```

If a child row reports a materialized unit, shares controller and broker
principals, or contains secret material, stop before runtime validation and
repair the declarative configuration.
