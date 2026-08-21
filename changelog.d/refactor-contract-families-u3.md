### Changed

- Split resource, provider, credential, semantic-service, and Zone/session
  contracts into narrow `d2b-contracts-*` crates while preserving wire and
  schema behavior. Resource protocol messages and generated drift ownership
  now live with `d2b-contracts-resource`; `d2b-resource-api` retains the
  generated ttrpc adapters. Broker, provider, and Zone/session crates import
  resource-owned types from `d2b-contracts-resource` directly instead of
  compatibility umbrellas.
