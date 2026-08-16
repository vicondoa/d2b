### Added

- Added an internal Process Provider supervisor adapter with bounded asynchronous dispatch, pidfd handoff, process identity revalidation, and service-manager identity binding, covered through hermetic adapter tests. No production runtime constructs this supervisor yet, and real broker, namespace, cgroup, and service-manager boundaries remain unverified.

### Security

- The adapter's Provider-facing diagnostics and status omit process identifiers, descriptors, unit and cgroup identity, paths, arguments, environment, and numeric user identity. Existing broker audit and journal fields still require separate hardening.
