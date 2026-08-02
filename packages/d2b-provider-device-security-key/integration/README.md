# device-security-key integration fixtures

Heavier scenarios live in:

- `lease_acquire_cancel/` for acquire, cancel, and re-acquire;
- `session_ring_capacity/` for bounded ring behavior under relay load;
- `guest_frontend_connect/` for Guest frontend authentication over AF_VSOCK.

They require the existing container or Host/Guest integration lane.

