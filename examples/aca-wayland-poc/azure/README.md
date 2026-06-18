# nixling ACA + Wayland POC — Azure deployment (Bicep)

Infrastructure-as-code for the ADR 0032 vertical of
[ADR 0032](../../../docs/adr/0032-nixling-v2-constellation-control-plane.md):
a real Azure Container Apps **sandbox** that runs a Wayland-native app, plus
an **Azure Relay** hybrid connection that carries the constellation
control/display streams. The app is forwarded to the operator's local
compositor via a Waypipe-style display provider.

This POC uses Azure Container Apps **Sandboxes**
([`Microsoft.App/sandboxGroups`](https://learn.microsoft.com/azure/container-apps/sandboxes-overview)),
not dynamic sessions: sandboxes give explicit lifecycle control (create /
suspend / resume / delete), exec, file management, port exposure, and
egress policies, and run on the Consumption plan.

All resource deployment is **Bicep**. There is no imperative `az ... create`
provisioning path for the Azure resources.

## Naming scheme

Every resource follows one scheme (configured in `main.bicep`):

```
<prefix>-<workload>-<suffix>      standard, hyphenated
<prefix><workload><suffix>        compact, where '-' is forbidden (e.g. ACR)
```

- **prefix** — a short resource-type abbreviation. Defaults follow the
  Azure Cloud Adoption Framework (CAF)
  [recommended abbreviations](https://learn.microsoft.com/azure/cloud-adoption-framework/ready/azure-best-practices/resource-abbreviations).
  Two types CAF does not define get a nixling-chosen prefix: the Container
  Apps sandbox group (`casbx`, in the CAF `ca*` family — `ca`/`cae`/`caj`)
  and the Relay namespace (`relns`, mirroring CAF's `<svc>ns` convention
  such as `sbns`).
- **workload** — a stable token for the deployment (default `nixling`).
- **suffix** — `uniqueString(subscription().id, resourceGroup().id)`: a
  deterministic hash that is stable across redeploys but unique per
  (subscription, resource group).

| Resource | Type | Prefix | Example name |
| --- | --- | --- | --- |
| Regional resource group | `Microsoft.Resources/resourceGroups` | `rg` | `rg-nixling-centralus` |
| Container registry | `Microsoft.ContainerRegistry/registries` | `cr` | `crnixling<suffix>` |
| Managed identity | `Microsoft.ManagedIdentity/userAssignedIdentities` | `id` | `id-nixling-<suffix>` |
| Sandbox group | `Microsoft.App/sandboxGroups` | `casbx` | `casbx-nixling-<suffix>` |
| Relay namespace | `Microsoft.Relay/namespaces` | `relns` | `relns-nixling-<suffix>` |
| Hybrid connection | `.../hybridConnections` | `hc` | `hc-nixling-display` |

## Resource group layout

- **`rg-<workload>-<region>`** (e.g. `rg-nixling-centralus`) — every
  resource in this POC is regional, so they all live here.
- **`rg-common`** is reserved by the scheme for genuinely subscription-global
  resources. This POC has none, so it is not created.

## Layout

```
azure/
├── main.bicep            subscription-scoped orchestrator (do not edit)
├── main.bicepparam       << customize this >>
└── modules/
    ├── registry.bicep    container registry + managed identity + AcrPull
    └── sandbox.bicep     sandbox group + Relay + SandboxGroup Data Owner
```

## Customize

Edit [`main.bicepparam`](./main.bicepparam). Every value has a default, so
change only what you need: `location`, `workload`, image repo/tag, tags,
and (optionally) the per-resource-type `prefixes` to match a different
house style.

## Two planes

Container Apps sandboxes use a two-plane architecture:

- **ARM control plane** (`management.azure.com`) — creates/updates/deletes
  the **sandbox group** (the management boundary). This is what the Bicep
  here deploys.
- **ADC data plane** (`management.azuredevcompute.io`) — creates and manages
  individual **sandboxes**, disk images, snapshots, volumes, ports, and
  egress policies, scoped to a sandbox group. These are **not** ARM/Bicep
  resources; the realm gateway drives them at runtime using the
  *Container Apps SandboxGroup Data Owner* role this deployment grants to
  the managed identity.

## Deploy

Prerequisites: `az login` (Owner/Contributor + User Access Administrator on
the subscription, since the deployment creates role assignments), and a
Bicep CLI (`az bicep install`, or on NixOS `nix run nixpkgs#bicep` with
`az config set bicep.use_binary_from_path=true`). Use a region where
Container Apps sandboxes are available (the default `centralus` is).

```bash
# Register the resource providers once per subscription:
az provider register --namespace Microsoft.App
az provider register --namespace Microsoft.Relay
az provider register --namespace Microsoft.ContainerRegistry
az provider register --namespace Microsoft.ManagedIdentity

# Preview, then deploy (subscription scope: it creates the resource group):
az deployment sub what-if \
  --location centralus \
  --template-file main.bicep \
  --parameters main.bicepparam

az deployment sub create \
  --name nixling-aca-wayland \
  --location centralus \
  --template-file main.bicep \
  --parameters main.bicepparam
```

The deployment outputs (registry login server, image reference, sandbox
group resource id, relay namespace + hybrid connection + the `Listen`/`Send`
SAS policy names) feed the next steps: building and pushing the Wayland
container image, registering it as a sandbox disk image, and enrolling the
realm gateway.

> **Note on `az deployment sub create`.** Some `azure-cli` builds (including
> the one packaged for NixOS at the time of writing) fail subscription-scope
> deployments with an `InvalidRequestContent` "unexpected character `$`"
> error that reproduces even on an empty template. If you hit that, deploy
> the two modules at resource-group scope instead: `az group create -n
> rg-nixling-<region> -l <region>`, then `az deployment group create` for
> `modules/registry.bicep` and `modules/sandbox.bicep` (the sandbox module
> takes the registry's `managedIdentityPrincipalId` output). Both modules
> are RG-scoped and self-contained.

## Trust-boundary notes (ADR 0032)

- The Relay namespace is a ciphertext-only rendezvous. Relay SAS keys
  authenticate relay access only — never a constellation principal.
- The `Listen` SAS policy is held by the realm **gateway** (listener). The
  gateway mints short-lived per-sandbox **`Send`** tokens for each sandbox
  from the `Send` policy; the policy key itself never enters a sandbox. SAS
  keys are fetched out-of-band during gateway enrollment and are **not**
  emitted as deployment outputs.
- Both ends connect outbound (gateway listener and in-sandbox sender), so no
  inbound ports are opened. Per-sandbox egress is further constrained by the
  data-plane egress policy the gateway sets.
- These resources hold no nixling host credentials; the host daemon never
  receives the relay/provider credentials (they live in the gateway guest).
