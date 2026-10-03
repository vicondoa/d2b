# Convert Cloud Hypervisor composition to the admitted graph

The Cloud Hypervisor Guest controller now has a graph-backed composition
beside its flattened attachment lists, and the daemon projects it from the
same committed rows production already reads.

- A Device or Network a Guest declares for the workloads it runs becomes a
  child target-support ceiling. It bounds what a child may request and
  creates no binding, no reservation, and no boot dependency (AE31).
- The Guest's own consumption is one admitted `VolumeBinding` relationship
  whose consumer is that Guest. Only a committed row whose consumer is the
  Guest supplies one; a row addressed to another Guest does not (AE32).
- A child's volume request default shapes that one child's request and
  refuses every other consumer. It becomes no Guest relationship and no
  Guest access (AE33).
- Source preparation and consumer-side completion are separate conditions,
  so a Guest whose export is prepared boots and its mount is observed
  afterwards rather than waited on before it (AE6, AE21).
- Restart adopts binding evidence only under the source and consumer
  identities the relationship was admitted under, and a stop drains every
  descendant and the Guest's own use before its finalizer clears.

The daemon's existing `cloud_hypervisor_inputs` projection is unchanged and
still feeds the production entry point; the classified projection beside it
is what the new provider coverage drives.
