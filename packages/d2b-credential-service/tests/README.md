# `d2b-credential-service` hermetic tests

The Cargo integration tests pin all five operation vectors, exact Role mapping,
strict malformed and oversize rejection, admission-before-dispatch, closed
failure states, delivery binding, zeroization, and process-unique redaction
canaries.
