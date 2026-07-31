### Added

- Added Secret Service, Entra identity-Guest, and managed-identity Credential Provider implementations with hermetic admission, delivery-binding, lease, placement, fault, and redaction coverage. Their binaries still report production runtime wiring as unavailable, so these Providers are not yet consumer-reachable Credential sources.

### Security

- Provider contracts keep credential bytes behind injected secret-holding clients and prevent Providers from selecting the consumer, audience, route, or delivery limit. Production authenticated delivery sessions and telemetry sinks are not wired yet.
