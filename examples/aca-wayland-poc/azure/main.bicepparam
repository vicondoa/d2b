// Customizable parameters for the d2b ACA + Wayland POC deployment.
//
// This is the file you edit. The templates under modules/ and main.bicep
// stay untouched. Deploy with:
//
//   az deployment sub create \
//     --name d2b-aca-wayland \
//     --location centralus \
//     --template-file main.bicep \
//     --parameters main.bicepparam
//
// Every value below has a sensible default in main.bicep; uncomment and
// change only what you need.

using './main.bicep'

// Region. Use a well-supported region where Container Apps sandboxes are
// available; centralus is the recommended default.
param location = 'centralus'

// Token embedded in every resource name (rg-<workload>-<region>,
// casbx-<workload>-<suffix>, ...).
param workload = 'd2b'

// House-style override for resource-type prefixes. Leave commented to use
// the CAF-aligned defaults (rg/cr/id + d2b-defined casbx/relns/hc).
// param prefixes = {
//   resourceGroup: 'rg'
//   registry: 'cr'
//   managedIdentity: 'id'
//   sandboxGroup: 'casbx'
//   relayNamespace: 'relns'
//   hybridConnection: 'hc'
// }

// Container image (built + pushed to the generated registry separately,
// then registered as a sandbox disk image via the data plane).
param imageRepository = 'd2b-wayland'
param imageTag = 'latest'

// Optionally grant a human operator the SandboxGroup Data Owner role so
// they can drive sandboxes in the portal/CLI. Set to your user object id
// (az ad signed-in-user show --query id -o tsv). Leave '' to skip.
param operatorPrincipalId = ''
param operatorPrincipalType = 'User'

// Tags. Set values meaningful for your subscription.
param tags = {
  workload: 'd2b'
  component: 'd2b-aca-wayland-poc'
  managedBy: 'bicep'
}
