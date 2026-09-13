### Fixed

- A durable `Process` row that reached `Ready` stayed under observation, on the
  preserved 5s resync: the pass re-reads the process through the Provider's
  liveness probe, so a controller, worker or VMM that exits leaves `Ready`, its
  `restartPolicy` decides between one budgeted restart (at the policy backoff)
  and the terminal `process-exited` report, and dependents watching the row
  learn the producer is gone instead of waiting on a row that reports `Ready`
  forever.
- A durable launch whose ticket the trusted bundle can never mint
  (`template-not-found`, `resolution-failed`, `guest-process-not-vmm`) now
  fails terminally instead of consuming the restart budget and retrying - and
  warning - forever; provider-effect and not-yet-bindable identity refusals keep
  their budgeted retry.
