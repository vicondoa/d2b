### Fixed

- An emulated `Device` now resolves its Provider's named capability instead of
  resolving nothing. The `Device` contract states that an emulated device
  carries no inventory selector, and the closed `InventorySelector` union is
  discriminated on `busClass`, so an emulated device has no selector to match
  on. The capability table sent that shape to its empty arm, an empty
  inventory is a refusal, and that refusal was raised while the driver was
  still reading the row's own bindings - so the `Device` row was abandoned
  before its Provider controller ran. The emulated TPM therefore never
  committed the state `Volume` its long-lived worker opens by pathname, the
  state directory never landed, and every spawn of that worker was refused for
  an absent state-directory leaf. The emulated TPM now names the same `tpm`
  capability a selected physical TPM names; a physical device that declares
  no selector is still refused by the contract, and the GPU, USBIP and
  security-key families are unchanged.
- The `device-worker-launch` check now reports the daemon's own account of a
  refused `Device` teardown. A refusing `d2b delete` writes its error
  envelope to the same file the accepted path writes the row into, so
  reporting the exit code alone said only that the teardown was refused and
  never which refusal. No assertion changed.
- The host-integration acceptance node spells its emulated TPM `Device` with
  an empty `inventory` rather than `inventory.selector = { }`. An empty object
  is not a member of the closed selector union, so that spelling produced a
  spec that did not decode. The check's own assertions are unchanged.
