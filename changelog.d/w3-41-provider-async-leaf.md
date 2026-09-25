### Changed

- Provider-agent sessions now dispatch each request concurrently instead of
  one at a time: a slow handler near the 900s timeout ceiling no longer
  stalls the rest of the session queue, and the 64-request in-flight ceiling
  now actually bounds concurrent dispatches.
- Provider-agent responses are no longer guaranteed to arrive in request
  order; each dispatch completes independently and sends its response as it
  finishes.
- The TPM device lifecycle-lease once-gate is now an atomic flag instead of a
  never-contended async lock (no behavior change).