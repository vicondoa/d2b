# Integration fixtures

The display integration lane uses fake Zone, GPU, portal, and Process
adapters. It covers session create-to-Ready, dependency Pending, proxy failure
backoff, finalizer ambiguity, clipboard-boundary denial, and
`crossDomainTrusted = false` admission. No fixture reads a compositor socket,
starts a host singleton, or emits a path.
