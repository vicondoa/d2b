### Added

- Added the core Process Provider supervisor adapter with bounded asynchronous dispatch, broker pidfd handoff, process identity revalidation, and service-manager identity binding for trusted Process launches.

### Security

- Kept process identifiers, descriptors, unit and cgroup identity, paths, arguments, environment, and numeric user identity out of Provider diagnostics, status, errors, audit summaries, and metric labels.
