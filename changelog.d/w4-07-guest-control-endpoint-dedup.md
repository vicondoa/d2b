### Fixed

- `GuestControlEndpoint` is now defined once, in `d2b-resource-client`, and re-exported by `d2b-provider-guest-cloud-hypervisor`; construction and validation failures surface as `ClientError` variants instead of `GuestLocalError::EndpointMismatch`, and the unused `endpoint_uid()` accessor is removed.