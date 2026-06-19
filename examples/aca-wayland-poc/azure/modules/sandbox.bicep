// Sandbox module for the nixling ACA + Wayland POC (ADR 0032, Wave P0).
//
// Azure Container Apps **Sandboxes** (Microsoft.App/sandboxGroups) are the
// isolation boundary for the POC: ephemeral, strongly-isolated compute
// instances with explicit lifecycle control, exec, file management, port
// exposure, and egress policies. The container (running the Wayland app +
// Waypipe + relay bridge) dials Azure Relay outbound.
//
// The sandbox GROUP is the only ARM resource: individual sandboxes, disk
// images, snapshots, volumes, and egress policies are created through the
// ADC data plane (management.azuredevcompute.io), not ARM/Bicep. This
// module also keeps the Azure Relay namespace + hybrid connection that
// carries the constellation control/display streams, and grants the shared
// identity data-plane ownership of the sandbox group.
//
// Naming scheme (see ../README.md): <prefix>-<workload>-<suffix>, suffix =
// uniqueString(subscription().id, resourceGroup().id).

targetScope = 'resourceGroup'

@description('Stable workload token used in every resource name.')
param workload string

@description('Resource-type name prefixes (CAF-aligned defaults set in main.bicep).')
param prefixes object

@description('Tags applied to every resource.')
param tags object

@description('Principal id of the identity that manages sandboxes via the data plane (from the registry module).')
param managedIdentityPrincipalId string

@description('Resource id of the user-assigned managed identity to attach to the sandbox group, so workloads inside the sandbox can fetch Entra tokens (plane 2: container -> Azure) from the injected IDENTITY_ENDPOINT. From the registry module.')
param managedIdentityResourceId string

@description('Optional object id of an operator (user/group) to also grant the SandboxGroup Data Owner role, so a human can drive sandboxes in the portal/CLI. Leave empty to skip.')
param operatorPrincipalId string = ''

@description('Principal type for operatorPrincipalId (User or Group).')
@allowed([
  'User'
  'Group'
])
param operatorPrincipalType string = 'User'

// Deterministic per-(subscription, resource group) suffix.
var suffix = uniqueString(subscription().id, resourceGroup().id)

var sandboxGroupName = toLower('${prefixes.sandboxGroup}-${workload}-${suffix}')
var relayNamespaceName = toLower('${prefixes.relayNamespace}-${workload}-${suffix}')
var hybridConnectionName = toLower('${prefixes.hybridConnection}-${workload}-display')

// "Container Apps SandboxGroup Data Owner" built-in role: required to
// create/manage sandboxes, disk images, and egress policies on the group
// through the ADC data plane.
var sandboxDataOwnerRoleId = 'c24cf47c-5077-412d-a19c-45202126392c'

// "Azure Relay Sender" built-in role: lets the sandbox's managed identity
// send on the hybrid connection using an Entra token (plane 2), so the
// container never receives a Relay SAS key/token.
var relaySenderRoleId = '26baccc8-eea7-41f1-98f4-1762cc7f685d'

@description('Sandbox group: the top-level management boundary for ephemeral, isolated sandboxes. Sandboxes themselves are created via the data plane. The user-assigned identity is attached here so workloads inside sandboxes can fetch Entra tokens for plane-2 (container -> Azure) access via the injected IDENTITY_ENDPOINT.')
resource sandboxGroup 'Microsoft.App/sandboxGroups@2026-02-01-preview' = {
  name: sandboxGroupName
  location: resourceGroup().location
  tags: tags
  identity: {
    type: 'UserAssigned'
    userAssignedIdentities: {
      '${managedIdentityResourceId}': {}
    }
  }
}

@description('Grant the shared identity data-plane ownership of the sandbox group so the realm gateway can create/suspend/exec/expose sandboxes.')
resource sandboxDataOwner 'Microsoft.Authorization/roleAssignments@2022-04-01' = {
  name: guid(sandboxGroup.id, managedIdentityPrincipalId, sandboxDataOwnerRoleId)
  scope: sandboxGroup
  properties: {
    roleDefinitionId: subscriptionResourceId('Microsoft.Authorization/roleDefinitions', sandboxDataOwnerRoleId)
    principalId: managedIdentityPrincipalId
    principalType: 'ServicePrincipal'
  }
}

@description('Optionally grant a human operator the same data-plane ownership so they can drive sandboxes in the portal/CLI. Created only when operatorPrincipalId is set.')
resource operatorDataOwner 'Microsoft.Authorization/roleAssignments@2022-04-01' = if (!empty(operatorPrincipalId)) {
  name: guid(sandboxGroup.id, operatorPrincipalId, sandboxDataOwnerRoleId)
  scope: sandboxGroup
  properties: {
    roleDefinitionId: subscriptionResourceId('Microsoft.Authorization/roleDefinitions', sandboxDataOwnerRoleId)
    principalId: operatorPrincipalId
    principalType: operatorPrincipalType
  }
}

@description('Azure Relay namespace (Standard SKU required for hybrid connections).')
resource relayNamespace 'Microsoft.Relay/namespaces@2024-01-01' = {
  name: relayNamespaceName
  location: resourceGroup().location
  sku: {
    name: 'Standard'
  }
  tags: tags
}

@description('Hybrid connection carrying the constellation control/display streams. Both gateway (listener) and sandbox (sender) connect outbound; client authorization is required.')
resource hybridConnection 'Microsoft.Relay/namespaces/hybridConnections@2024-01-01' = {
  parent: relayNamespace
  name: hybridConnectionName
  properties: {
    requiresClientAuthorization: true
  }
}

@description('Grant the sandbox group identity the Azure Relay Sender role on the namespace, so a workload inside a sandbox can authenticate to the hybrid connection with an Entra token from its managed identity (plane 2) instead of a Relay SAS token. This is the productionized container -> Relay path; no SAS key ever enters the sandbox.')
resource relaySender 'Microsoft.Authorization/roleAssignments@2022-04-01' = {
  name: guid(relayNamespace.id, managedIdentityPrincipalId, relaySenderRoleId)
  scope: relayNamespace
  properties: {
    roleDefinitionId: subscriptionResourceId('Microsoft.Authorization/roleDefinitions', relaySenderRoleId)
    principalId: managedIdentityPrincipalId
    principalType: 'ServicePrincipal'
  }
}

@description('SAS policy granting Listen on the hybrid connection (held by the realm gateway listener). Keys are fetched out-of-band during gateway enrollment; they are never emitted as deployment outputs.')
resource listenRule 'Microsoft.Relay/namespaces/hybridConnections/authorizationRules@2024-01-01' = {
  parent: hybridConnection
  name: 'gateway-listen'
  properties: {
    rights: [
      'Listen'
    ]
  }
}

@description('SAS policy granting Send on the hybrid connection. SUPERSEDED by the Azure Relay Sender role on the sandbox identity (plane 2): the productionized container authenticates to Relay with an Entra token from its managed identity, so no SAS key/token enters the sandbox. Retained for the transitional gateway-minted-token path and for non-MI senders.')
resource sendRule 'Microsoft.Relay/namespaces/hybridConnections/authorizationRules@2024-01-01' = {
  parent: hybridConnection
  name: 'gateway-send'
  properties: {
    rights: [
      'Send'
    ]
  }
}

output sandboxGroupName string = sandboxGroup.name
output sandboxGroupResourceId string = sandboxGroup.id
output relayNamespaceName string = relayNamespace.name
output hybridConnectionName string = hybridConnection.name
output relayListenRuleName string = listenRule.name
output relaySendRuleName string = sendRule.name
