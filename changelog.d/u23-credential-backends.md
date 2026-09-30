# Convert credential implementations onto the admitted delivery relationship

The Entra, managed-identity, and Secret Service Credential Providers each
answered "is this delivery session still the one I authorized?" from the
authenticated route alone, with their own comparison order, their own notion of
a matching consumer, and their own sequence counter. A relationship the graph
had already moved past could therefore still be served by whichever backend
compared the fewest fields.

That question now has one answer. The Credential contract owns
`AdmittedCredentialDelivery` and `admit_credential_delivery`: the relationship
answers only from committed source state, and the gate refuses - before any
client is asked - when the fence moved, when the presented session's audience
or operation class is outside the admitted policy, or when the presented
session is not the relationship's current one. `CredentialDeliveryAuthority` is
the one implementation of that port, so a refresh can no longer widen the
audience or the allowed operations, and a replaced consumer, a reconnected
Provider session, or a superseded session no longer renews the earlier delivery.

Each Provider keeps its own acquisition, custody, and refresh semantics and
gains a `dispatch_admitted` entry point that runs the shared gate; the
supervised service loop still reaches the unchanged route-derived dispatch.
Refusals name a stage and a reason from the shared admission vocabulary and
nothing else, and the rendered graph spec and status carry no credential
material.