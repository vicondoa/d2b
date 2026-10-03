Configuration, telemetry, and observability now consume admitted relationships
instead of deciding access themselves.

A configuration Provider's approval is a declaration, not an authority. Config
generates the canonical `VolumeBinding` request naming the exact `Volume`
source, the exact `Guest` consumer, the stable slot, the named view, the access
level, and the consumer-side presentation, then invokes the admitted activation
through the shared binding contract. The Role evaluation's grant and the Volume
owner's scoped decision arrive from outside, so a configuration document cannot
authorize its own publication, cannot publish through a source decision scoped
to a different relationship, and cannot publish a presentation the selected
backend does not realize. The Provider's own realization support is a fixed set
rather than a caller-supplied one, so a caller cannot widen it to make a facet
this Provider never realizes look admitted.

Telemetry delivery rides an admitted `EndpointBinding`, `NetworkBinding`, or
`CredentialBinding` rather than a socket path. A delivery route has no
constructor from a resource name, a socket path, or a service-catalog row: it
holds the `BindingEvidence` a source provider minted, and that evidence is only
reachable from an admission the shared contract produced. Every ingress
admission now takes the route and the observed dependency evidence, so a route
whose relationship stopped admitting new use, or whose committed revision moved,
refuses delivery instead of admitting a frame.

Revocation stops delivery rather than narrowing it. Revoking any one of a
route's relationships records the source it lost and refuses the route at the
revoking stage; it does not fall back to whatever admission survives. The route
is pinned to the one transport its endpoint owner admitted, so a stopped
relationship cannot be resumed by re-presenting the same producer elsewhere -
the other transports are refused at the admitting stage.

The emitters keep their redaction and bounded transport unchanged. A refusal is a
stage plus a field-free reason, and the resource travels beside it rather than
inside it: the rendered diagnostic names the exact resource and the enforcing
stage and carries no frame content, label value, socket path, or credential
byte. The emitter socket refuses a drain outright on a stopped relationship
rather than admitting a datagram it would later discard.

The telemetry Service and Binding drivers read the ingest route's own observed
classification at its current row generation instead of treating a live row as a
usable route, and a Binding fences rather than materializing a collector whose
Service admits no ingest route.
