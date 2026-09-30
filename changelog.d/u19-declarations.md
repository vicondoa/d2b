# Core metadata provider declarations (U19, KTD1)

The Zone, Host, User, and Provider families described themselves only through a
per-type `DriverDescriptor` the daemon's composition root registered by hand.
Nothing tied the type the plane served to a declared artifact, a component, an
execution target, or the one method a family actually publishes, so a family
could drift from its own registration with no check to notice.

Each of the four owning crates now exports its own unified declaration: the
serializable `ProviderDeclarationSpec` generators project, and the
`ProviderImplementationBindings` that realize it. The declarations are derived
from the crates' own tables rather than transcribed - the artifact id is the
family name the plane already registers, and the declared service and its
methods are read from the same `ServiceDecl` the descriptor carries - so the
two halves cannot name different providers or claim a method the family does
not serve. Every crate proves the agreement in its own tests, including the
refusal of a bound driver that serves a type the declaration does not own.

The Host and User families declare two components each rather than one: the
controller reconciles rows and publishes no method, and the hosted effects
service answers the family's single `inspect-host` / `inspect-user` method at
the Host execution target. Collapsing them would give one of the two an
identity it does not have.

The declared configuration and target digests are placeholders on purpose.
`project_provider_graph` refuses a declaration whose declared digest disagrees
with the verified build output's, so a placeholder fails closed at the
projection instead of passing as a real build identity. The packaging stage
binds the verified digest; until then no build output is admitted.

`ProviderDeclaration::validate` also learned to recognize a bound zone-plane
service method as implemented. It previously required every declared method to
name a committed operation row with a bound handler, which no method of a
zone-plane effects service can do, so no Host or User declaration could have
validated at all. A method that names an operation row still requires its
bound handler, so both existing refusals are unchanged.
