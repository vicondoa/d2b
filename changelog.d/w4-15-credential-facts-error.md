### Fixed

- A transient manager read failure during a credential dependency-facts probe
  is no longer reported as missing dependency facts: the
  `CredentialRuntime::dependency_facts` facet now returns the failure as a
  typed error that reaches the credential driver, so readiness and
  revocation decisions fail closed on a failed read instead of degrading on
  absence semantics.