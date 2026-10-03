### Added

- The `Device` source now admits typed `DeviceBinding` relationships
  through one path. A request may name only a capability the source's
  trusted inventory resolved and only while the host still backs it, the
  physical authority key comes from that inventory rather than a device-node
  path, and exclusive and shared claims are arbitrated once across every
  other live relationship on the same authority. A relationship that is
  revoking or draining keeps holding its claim until release evidence
  arrives, so a device is never handed to a second consumer while the first
  is still closing.
- Provider-created device workers take an explicitly attenuated *leg* of
  their parent's reservation. A helper names the parent's reservation, its
  own identity, the exact capability and physical authority the parent
  holds, a permitted operation subset, and the broker epoch the parent was
  fenced against. It cannot introduce another source, widen the parent's
  operations, outlive the parent's revocation, or hold a claim of its own,
  so an exclusive parent claim supports its own bounded helper without
  competing for an allocation.
- An absence observation now decides the safe outcome per relationship
  rather than per family: a capability the inventory no longer backs is
  revoked, a capability that cannot be proven effective is degraded, and
  everything else keeps its use exactly as it was. Another owner's claim and
  another capability on the same device are untouched.
- TPM state is an ordinary `VolumeBinding` pair. The long-lived swtpm
  worker claims the `swtpm-process` view read-write and the one-shot
  pre-start flush claims the `controller` view read-only, each through its
  own `VolumeBindingRequest`, and the durable state identity both derive is
  the same after a controller restart. A framework that resolves any other
  Volume as this Device's state is refused as a state-integrity failure
  rather than adopted, so the NVRAM and tamper marker cannot move.
- `DeviceInventorySource` is a new declared facet: the trusted host
  device-node matrix is the only place a Device row's named capabilities
  resolve to opaque physical authorities, and a row whose inventory cannot
  be read admits nothing.
- `GpuDeviceGrants` carries the named capabilities a GPU or video worker
  needs, and a worker whose declared shape needs one the Device has no
  admitted binding for is refused before any reservation, device open, or
  spawn. A decode setting now selects which *admitted* capability the video
  worker asks for instead of which node list its launch template carries.

### Changed

- The swtpm launch no longer accepts a caller-supplied `uid=`/`gid=` socket
  owner. A numerical broker identity in a launch argument is identity a
  caller selects, and swtpm refuses the chown inside the launch's user
  namespace anyway; each socket is created owned by the uid swtpm runs as,
  which is what the state directory's ACL and `mode=0660` are written
  against. The free-form `extraArgs` tail is gone from the swtpm and GPU
  argv generators, so the rendered flag set is the whole of what a caller
  can influence.
- The swtpm, GPU, and video argv generators refuse an input that names a
  device node. A device capability is delivered by the worker's own admitted
  `DeviceBinding`, so a command line is never the way to reach a render node
  or any other device.
