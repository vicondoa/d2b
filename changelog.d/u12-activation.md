### Changed

- Converted the `activation-nixos.d2bus.org.NixosGeneration` reconciler to
  the v3 `ResourceDriver`/`ResourceDriverFactory` contract: the pure
  activation policy still plans the runner, a Host target dispatches the
  preserved `ApplyHostGenerationHandoff` broker effect through a driver
  effect port, and a Guest target mints the activation-runner
  `EphemeralProcess` as an owned child through the manager so the Process
  controller owns the launch (no controller-side spawn).
- Moved the activation spec decode onto the manager-wired decoder, so the
  closed generation contract (Provider reference, Host/Guest execution
  target, artifact identifier, prior-generation type) is the validation
  fence, and retried failures keep the preserved retryable classification.
