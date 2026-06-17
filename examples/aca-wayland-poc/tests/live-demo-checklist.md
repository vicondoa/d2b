# Live demo checklist — ACA sandbox + Wayland forwarding (ADR 0032)

This is a **Layer 2, manual** procedure (it provisions real Azure resources
and needs a live Wayland compositor). It is the reproducible record behind
the acceptance bar: a Wayland-native app running inside an **Azure
Container Apps sandbox** is rendered on the operator's compositor with every
display byte carried over an **Azure Relay** hybrid connection.

It is intentionally not wired into `make test-unit` / `make test-host-integration`;
run it by hand and capture the screenshot/logs into a scratch dir (do **not**
commit raw artifacts — they can leak resource metadata).

## Prerequisites
- `az` logged in to the target subscription; `waypipe` and a running
  Wayland compositor on the host (versions of waypipe must match the image,
  which pins the host nixpkgs — both ends were 0.11.0 in the reference run).
- The `aca` sandbox data-plane CLI (the public preview build), used to drive
  the sandbox (ACA sandboxes do not run the image entrypoint).
- `podman` (or `docker`) + `nix` to build and push the image.

## 1. Provision (Bicep)
```bash
cd examples/aca-wayland-poc/azure
az group create -n rg-nixling-centralus -l centralus
az deployment group create -g rg-nixling-centralus \
  -f main.bicep -p main.bicepparam -p operatorPrincipalId="$(az ad signed-in-user show --query id -o tsv)"
```
This creates the ACR, the managed identity (AcrPull + SandboxGroup Data
Owner), the sandbox group, and the Relay namespace + `hc-nixling-display`
hybrid connection with `gateway-listen` (Listen) and `gateway-send` (Send)
SAS rules. Grant yourself the **Container Apps SandboxGroup Data Owner** role
on the sandbox group (the `operatorPrincipalId` param does this).

## 2. Build + push the image
```bash
cd examples/aca-wayland-poc/container
./build-and-push.sh           # nix build -> podman push to the deployed ACR
```

## 3. Register a sandbox disk + create a sandbox
```bash
export ACA_SUBSCRIPTION=<sub> ACA_RESOURCE_GROUP=rg-nixling-centralus \
       ACA_SANDBOX_GROUP=<sandbox-group> ACA_REGION=centralus
TOKEN=$(az acr login -n <registry> --expose-token --query accessToken -o tsv)
aca sandboxgroup disk create --image <login-server>/nixling-wayland:latest \
  --username 00000000-0000-0000-0000-000000000000 --token "$TOKEN"
aca sandbox create --disk-id <disk-id> --cpu 1000m --memory 2048Mi
```

## 4. Start the host display receivers
```bash
cd examples/aca-wayland-poc/relay-bridge && cargo build --release && cd -
export NIXLING_RELAY_NS=<namespace>.servicebus.windows.net
examples/aca-wayland-poc/host/run-host-display.sh up
```

## 5. Launch the Wayland app in the sandbox
Run the baked-in agent through the sandbox data plane, handing it a
short-lived **Send** SAS token (never the host's Listen key, never a
provider identity):
```bash
SEND_KEY=$(az relay hyco authorization-rule keys list -g rg-nixling-centralus \
  --namespace-name <namespace> --hybrid-connection-name hc-nixling-display \
  --name gateway-send --query primaryKey -o tsv)
aca sandbox exec --id <sandbox-id> -c "sh -c '
  NIXLING_RELAY_NS=<namespace>.servicebus.windows.net \
  NIXLING_RELAY_ENTITY=hc-nixling-display \
  NIXLING_RELAY_KEYNAME=gateway-send \
  NIXLING_RELAY_KEY=$SEND_KEY \
  ( /bin/nixling-sandbox-agent </dev/null >/tmp/agent.log 2>&1 & ); sleep 6; cat /tmp/agent.log'"
```
A `foot` window from the sandbox appears on the host compositor. Override
`NIXLING_APP_CMD` to run a different command under foot (e.g. a banner that
prints the sandbox hostname/kernel, as in the reference screenshot).

## 6. Verify
- The forwarded window is present on the host compositor (e.g.
  `niri msg windows | grep -c 'App ID: "foot"'` increments).
- The Azure portal "Network Audit" for the sandbox shows an **allowed**
  egress to `…servicebus.windows.net/$hc/hc-nixling-display`.
- `agent.log` shows `waypipe …server` launched after the relay bridge began
  listening on `unix-listen:/run/nixling/wp.sock` (no socat, no bind race).

## 7. Clean up (always)
```bash
aca sandbox delete --id <sandbox-id> --yes
# delete any extra disks you created, then:
examples/aca-wayland-poc/host/run-host-display.sh down
```
Leaving sandboxes running bills compute and holds the relay connection open.

## Notes / gotchas
- ACA sandboxes terminate egress TLS with a transparent proxy; the agent
  passes `--ca-file /etc/ssl/certs/adc-egress-proxy-ca.crt` to the relay
  bridge. Mozilla/webpki roots alone fail with `UnknownIssuer`.
- `0.0.0.0:8080` is LISTEN-reserved inside the sandbox; the committed path
  avoids host-port bridging entirely via the relay-bridge `unix-listen`
  target.
- `aca sandbox exec` of a long-running command must detach (`( cmd & )`),
  or the CLI retries and launches the workload twice.
- The relay sees only encrypted peer bytes + connection metadata; it is
  never the authenticated constellation principal (ADR 0032).
