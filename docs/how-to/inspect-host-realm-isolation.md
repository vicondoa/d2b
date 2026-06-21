# Inspect host realm isolation

**Diataxis category:** how-to.

Use these checks on a deployed host when investigating a gateway-backed
realm. They should show that the host is local-only and credentials live in
the gateway guest.

1. Confirm the host has no gateway credential config:

   ```bash
   test ! -e /etc/nixling/gateway.json
   ```

2. Inspect the static host policy:

   ```bash
   jq . /etc/nixling/host-realm-relay-egress-policy.json
   ```

3. Check host daemon and broker process environment/cmdline for accidental
   relay credential variables:

   ```bash
   for pid in $(pgrep -x nixlingd; systemctl show -p MainPID --value nixling-priv-broker.service); do
     tr '\0' '\n' < /proc/$pid/environ | grep -F NIXLING_RELAY_ && exit 1
     tr '\0' ' ' < /proc/$pid/cmdline | grep -F NIXLING_RELAY_ && exit 1
   done
   ```

4. If any check fails, remove host-readable relay credentials from the host
   config and enroll them inside the gateway guest with
   `nixling-gateway-enroll`.
