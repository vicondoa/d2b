### Added

- Added the asynchronous v3 resource API contract, typed client and service
  admission layer, storage-neutral backend interface, and literal protobuf wire
  vectors for every request, response, and supporting message.
- Added native Role and RoleBinding authorization with revision-bound
  positive-decision caching, fail-closed relay and bootstrap policy, exact
  capabilities, and sealed authorization evidence that only a real evaluation
  can issue and only its paired store instance can verify.
- Added the authenticated ttrpc resource-service adapter while leaving
  production bus and Zone dispatch explicitly unwired.
