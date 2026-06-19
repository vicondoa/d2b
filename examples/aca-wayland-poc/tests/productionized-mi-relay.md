# Historical live reproduction — MI-auth ACA + Wayland forwarding probe

**Layer 2, manual** (provisions real Azure + needs a live Wayland
compositor). This is the reproducible record behind the productionized
acceptance bar for the earlier MI-auth relay probe: a Wayland-native `foot`
inside an **Azure Container Apps sandbox** rendered on the operator's
compositor, with every display byte on an **Azure Relay** hybrid connection.
The current P0 gateway path uses a gateway-minted short-lived Send bearer
because ACA Entra Relay substreams later proved unreliable for Waypipe; the
long-lived SAS rule key still never enters the workload.

Unlike `live-demo-checklist.md` (the original SAS-bridged POC), this path
used the productionized `nixling-relay` endpoint binary
(`packages/nixling-provider-relay`) on both ends and the sandbox's MI Entra
token (plane 2). Do **not** commit raw screenshots/logs.

## Critical protocol note (why the sender omits `sb-hc-id`)

Azure Relay Hybrid Connection senders MUST NOT supply their own `sb-hc-id`:
the relay mints the GUID rendezvous id and embeds it in the accept message.
A caller-supplied non-GUID id yields an unserviceable rendezvous address and
the listener's accept connect fails `HTTP 400 Bad Request`. The official
`hyco-websocket` SDK omits the id; `nixling-provider-relay::build_connect`
does too. Don't reintroduce a sender-side id.

## Prerequisites
- `az` logged in; `waypipe` + a running Wayland compositor on the host
  (the image pins waypipe to the host nixpkgs — both ends 0.11.0 here).
- The `aca` sandbox data-plane CLI (ACA sandboxes don't run the image
  entrypoint; drive them via `aca sandbox exec`).
- `podman` + `nix` to build/push the image.
- Bicep already deployed (see `live-demo-checklist.md` §1): ACR, the relay
  namespace + `hc-nixling-display`, the sandbox group, and the
  user-assigned MI with the **Azure Relay Sender** role (declared in
  `azure/modules/sandbox.bicep`) for the historical MI probe.

## 1. Build + push the image with the productionized sender
```bash
cd examples/aca-wayland-poc/container
./build-and-push.sh "$REGISTRY" nixling-wayland:mi   # builds nixling-relay from the workspace
```
`image.nix` built `nixling-relay` from `../../../packages` and the historical
agent (`bridge/nixling-sandbox-agent.sh`) fetched the MI token from
`$IDENTITY_ENDPOINT` and ran `nixling-relay sender`.

## 2. Fresh disk + sandbox from the just-pushed image
A disk snapshots the image at create time, so create a NEW disk after the
push (an older `:mi` disk has the old binary):
```bash
TOKEN="$(az acr login -n "$REGISTRY" --expose-token --query accessToken -o tsv)"
aca sandboxgroup disk create --region "$REGION" \
  --image "$LOGIN_SERVER/nixling-wayland:mi" --name nixling-mi-fixed \
  --username 00000000-0000-0000-0000-000000000000 --token "$TOKEN"
aca sandbox create --region "$REGION" --disk-id <new-disk-id> --cpu 1000m --memory 2048Mi
```

## 3. Host receiver: waypipe client + relay listener
```bash
waypipe --no-gpu -c zstd -s /run/user/1000/wpc.sock client &     # if not already running
NIXLING_RELAY_NAMESPACE=<ns>.servicebus.windows.net \
NIXLING_RELAY_ENTITY=hc-nixling-display \
NIXLING_RELAY_KEY_NAME=gateway-listen NIXLING_RELAY_KEY=<listen-key> \
  nixling-relay listener --target unix:/run/user/1000/wpc.sock &
```

## 4. Drive the sandbox agent (foot over the MI relay)
Exec the agent in the sandbox with the relay coordinates + app command;
detach it so the exec returns while foot keeps running:
```bash
aca sandbox exec --id <sandbox-id> -c "$(cat <<'EOF'
export NIXLING_RELAY_NS='<ns>.servicebus.windows.net'
export NIXLING_RELAY_ENTITY='hc-nixling-display'
export NIXLING_APP_CMD='foot --title=ACA-MI-RELAY'
( /bin/nixling-sandbox-agent </dev/null >/tmp/agent.log 2>&1 & )
sleep 16; cat /tmp/agent.log
EOF
)"
```
Expect: `MI token acquired (... chars); starting nixling-relay sender (no
SAS)`, then `waypipe server -- foot ...`.

## 5. Verify it opened
```bash
niri msg windows | grep -A1 ACA-MI-RELAY          # window present, app_id foot
# sandbox-side process tree (minimal image: read /proc, no ps):
aca sandbox exec --id <sandbox-id> -c 'sh -c "cat /proc/*/comm | sort | uniq -c"'
#   -> foot, nixling-relay, waypipe x2 all alive
```
Pass oracle: a `foot` window titled `ACA-MI-RELAY` is mapped on the
compositor AND the sandbox holds zero Azure secret (it used its MI).

## Teardown (resources bill)
```bash
aca sandbox delete --id <sandbox-id>
kill <relay-listener-pid> <waypipe-client-pid>
```
