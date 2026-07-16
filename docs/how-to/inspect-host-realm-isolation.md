# Inspect host realm isolation

**Diataxis category:** how-to.

Use these checks on a deployed host to confirm that realm/provider credentials
have not leaked into the local-root daemon or broker.

1. Confirm no obsolete gateway configuration was installed:

   ```bash
   test ! -e /etc/d2b/gateway.json
   ```

2. Inspect the static host policy:

   ```bash
   jq . /etc/d2b/host-realm-relay-egress-policy.json
   ```

3. Check host daemon and broker process environment/cmdline for accidental
   relay credential variables:

   ```bash
   for pid in $(pgrep -x d2bd; systemctl show -p MainPID --value d2b-priv-broker.service); do
     tr '\0' '\n' < /proc/$pid/environ | grep -F D2B_RELAY_ && exit 1
     tr '\0' ' ' < /proc/$pid/cmdline | grep -F D2B_RELAY_ && exit 1
   done
   ```

4. If any check fails, remove host-readable Relay credentials and obsolete
   gateway artifacts. Do not use the legacy gateway enrollment helper.
   Credentials belong in the exact provider agent that owns their consumer;
   only opaque leases may cross that boundary.
