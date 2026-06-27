// Registry module for the d2b ACA + Wayland POC (ADR 0032).
//
// The container registry and the user-assigned managed identity are
// regional resources (each has a location), so they are deployed into the
// regional resource group (rg-<workload>-<region>) alongside the rest of
// the stack. They are split into their own deployment so the Wayland image
// can be pushed AFTER the registry exists but BEFORE the session pool
// references it.
//
// Naming scheme (see ../README.md): <prefix>-<workload>-<suffix>, or
// <prefix><workload><suffix> compact where '-' is forbidden (ACR). The
// suffix is uniqueString(subscription().id, resourceGroup().id).

targetScope = 'resourceGroup'

@description('Stable workload token used in every resource name.')
param workload string

@description('Resource-type name prefixes (CAF-aligned defaults set in main.bicep).')
param prefixes object

@description('Tags applied to every resource.')
param tags object

// Deterministic per-(subscription, resource group) suffix.
var suffix = uniqueString(subscription().id, resourceGroup().id)

// AcrPull built-in role definition id (constant across clouds).
var acrPullRoleId = '7f951dda-4ed3-4680-a7ca-43fe172d538d'

var registryName = toLower('${prefixes.registry}${workload}${suffix}')
var identityName = toLower('${prefixes.managedIdentity}-${workload}-${suffix}')

@description('Container registry. Name is compact (ACR forbids hyphens).')
resource registry 'Microsoft.ContainerRegistry/registries@2023-11-01-preview' = {
  name: registryName
  location: resourceGroup().location
  sku: {
    name: 'Basic'
  }
  properties: {
    // Identity-based pulls only; no long-lived admin credentials.
    adminUserEnabled: false
  }
  tags: tags
}

@description('User-assigned identity used for image pulls and session-pool management.')
resource identity 'Microsoft.ManagedIdentity/userAssignedIdentities@2023-01-31' = {
  name: identityName
  location: resourceGroup().location
  tags: tags
}

@description('Grant the identity AcrPull on the registry (managed-identity image pulls).')
resource acrPull 'Microsoft.Authorization/roleAssignments@2022-04-01' = {
  name: guid(registry.id, identity.id, acrPullRoleId)
  scope: registry
  properties: {
    roleDefinitionId: subscriptionResourceId('Microsoft.Authorization/roleDefinitions', acrPullRoleId)
    principalId: identity.properties.principalId
    principalType: 'ServicePrincipal'
  }
}

output registryName string = registry.name
output registryLoginServer string = registry.properties.loginServer
output registryResourceId string = registry.id
output managedIdentityName string = identity.name
output managedIdentityResourceId string = identity.id
output managedIdentityPrincipalId string = identity.properties.principalId
output managedIdentityClientId string = identity.properties.clientId
