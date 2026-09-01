# Video Provider

**Diataxis category:** reference.

Video decode is an optional Provider capability attached to a Zone Guest. The
Provider owns the guest media endpoint, GPU/video runner, device allowlist,
and broker launch contract. It is not a separate lifecycle service.

The Guest controller observes the Provider's Process and Endpoint resources
and keeps the Guest Pending or Degraded when video prerequisites are absent.
No stock runtime fallback, free-form video arguments, host device path, or
caller-selected runner is accepted.

Validate the rendered contract with the owner-local Provider and broker tests.
Inspect the current result with:

```bash
d2b guest status <name> --zone <zone>
d2b provider status <name> --zone <zone>
```
