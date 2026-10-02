test(plane): prove the wired families from the plane's own provider set

Seven subsystems were registered by the daemon's composition and exercised
only through fixtures that assembled the same parts by hand. Each is now driven
through `resource_plane_v3::provider_set`, the composition the daemon runs, so
the driver under test is the one a Zone starts.

- The common Guest target's four runtime Providers are decided from a row's
  own `spec.providerRef` and nothing else. A Cloud Hypervisor row now has to
  reach the Zone's Cloud Hypervisor controller session, a qemu-media row has to
  commit the runtime Volume and the qemu worker Process its Provider derives,
  the two Azure Providers have to take different branches of the same driver,
  and a row naming a Provider the family does not own has to be refused rather
  than guessed onto one of them.
- The two telemetry types are wired below the generated registration table, so
  nothing in the generated closure names them. A Binding row has to materialize
  the Serving Provider's own collector Process and ingest Endpoint, an
  unadmitted Service has to fence it with no child committed, and a projection
  Service has to publish its own Ready projection.
- The display Wayland policy row carries no Provider selector and no children,
  so the family branch the composition built its descriptor over is the only
  thing that decides its outcome. It now has to publish the policy family's
  Ready projection rather than the session branch's.

Every test was run against a mutation that takes the production path apart -
rerouting the Cloud Hypervisor branch, renaming the qemu worker Process,
swapping the two Azure branches, registering the Guest family with no drivers,
dropping the Binding's ingest admission, changing the Serving Provider's
declared child, degrading a projection Service, and sending the display policy
effect down the session branch - and each one failed under its own mutation.

The `test_inputs` fixture also gained a seam for a caller-chosen Guest facet
set, mirroring the interaction-facet seam beside it. Without it the fixture's
default facet set binds a controller generation no plane test could reconcile
through, which is why the Guest effects never ran under the plane suite.