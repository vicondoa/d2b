### Fixed

- Device-TPM state-directory preparation no longer blocks a runtime worker.
  Its broker leg - the `prepare-directory` envelope round trip, a blocking
  seqpacket connect, frame write, reply poll and frame read bounded by the
  existing kernel io budget - now runs on the bounded kernel seat, and the
  trusted storage row's `User`/`Group` principal lookups (NSS reads with no
  async form) run on the bounded probe seat. A slow or wedged broker or NSS
  backend now refuses later seat jobs and is reported as a retryable effect
  failure instead of stalling every task scheduled on the worker. The leg's
  own error mapping is unchanged: a broker transport, protocol or refusal
  error still fails the preparation closed.
