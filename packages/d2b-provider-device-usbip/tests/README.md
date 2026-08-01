# `d2b-provider-device-usbip` hermetic tests

These Cargo integration tests exercise the controller with fake effect ports,
the shared firewall action contract, dependency scoping, worker declarations,
bus-id validation, rejection behavior, retention ordering, and redaction. They
perform no external I/O, module load, process spawn, socket bind, or device open.
