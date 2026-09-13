# Guest family integration

Package-local scenario declarations for the `Guest` family. Each file is a
declaration, not an executable target: the repository's host-integration lane
runs the guest lifecycle end to end under KVM, and the crate's own suites
cover the driver's behavior over a scripted effect port.

The files here are checked in as evidence of intent for the scenarios that
lane must eventually run; the crate's `tests/` are the executable surface.
