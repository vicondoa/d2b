### Added

- A Device-owned worker template's declared read-write Volume binding is now
  honored end to end. The closed `DeviceWorkerPosture` table states what each
  template may bind as a property of the bound Volume's owner (a worker may
  bind a Volume its own owning Device owns) rather than as a list of Volume
  references, the resource compiler carries the declared mounts into the
  signed template binding and refuses a binding the posture does not permit
  (`provider-device-worker-volume-not-admitted`, naming the refused binding),
  and the bundle resolver resolves an admitted binding to its host path and
  mints it into the launch policy's writable paths. A read-only binding grants
  nothing; a declared grant the storage contract cannot resolve is fail-closed
  rather than silently dropped. The shared Process projection admits such a
  declared mount at Nix eval time by ownership rather than by shape: a mount
  names either a same-Zone declared Volume (as before) or a
  controller-created child Volume of its own row's owning Device, recognized
  by the deterministic uid segment the Device manager embeds in the name. A
  well-formed name carrying another Device's uid, or a row with no owning
  Device, is still refused. The uid derivation is one shared helper
  (`nixos-modules/lib.nix`'s `resourceUidShort`), called by both the Provider
  projection that names the child and the assertion that recognizes it.
