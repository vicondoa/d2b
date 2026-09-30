# EndpointBinding and exact endpoint delivery

An `EndpointBinding` grants one consumer the ONE exact `Endpoint` its request
named. The `d2b-provider-endpoint` crate now owns that relationship end to end
(`src/binding.rs`): admission against the endpoint's own declaration, the
pinned `(dev, ino)` identity the exact endpoint resolves to, the delivery form
the attachment kind declares, and the ordered teardown that retires the
endpoint before the producer that owns it.

Access can no longer be broadened by constructing a path. An
`EndpointBindingRequest` has no locator, the delivery is a descriptor or a
private exact-socket presentation over one inode, and
`fence_delivery_payload` / `fence_delivery_environment` refuse any value that
names an absolute host path, the socket's containing directory, a relative
escape, or a destination the relationship does not own. A consumer admitted
against one compositor endpoint reaches that endpoint only.

Readiness is EFFECTIVE access rather than ACL presence. The broker's new
exact-endpoint helpers read the POSIX access ACL the kernel stores through the
pinned `O_PATH` descriptor and compute the permission an access check actually
applies - the named entry already ANDed with the mask - so an entry a later
mode reconciliation has nullified is reported not-effective instead of applied,
following the recorded solution in
`docs/solutions/infrastructure/posix-acl-mask-nullified-by-chmod-on-mode-0700-directories.md`.
The traverse bit is checked across every ancestor, and the containing directory
is granted traverse only, never listing.

Nothing in the production composition changed: the new broker helpers are
called from no production path yet, and the old session-runtime-directory
socket grants are untouched.
