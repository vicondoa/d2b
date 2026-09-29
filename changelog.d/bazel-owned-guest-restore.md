### Fixed

- Refused a block node the host-integration lane could not name. Every block
  node the lane hands the emulator is now built inside the emulator's own
  31-byte name limit: a check's name alone can run past it
  (`runtime-cloud-hypervisor-guest-preflight` does), and the refusal arrived as
  a monitor error at the first snapshot - after the guest had booted - rather
  than at the launch. The check's readable name is kept in the device id, in
  the layer file, and in the lane's report, where the limit does not reach.
- Put a restored device back on a layer the lane owns. A node the emulator
  creates for a snapshot is dropped as soon as nothing references it, and a
  restore builds its chain with no device on the block graph, so the device was
  being re-attached to a node name that no longer existed. The restore now
  re-opens the layer the snapshot wrote and attaches the device to that, which
  is also what keeps the restored disk writable where the emulator's own
  snapshot node inherited the read-only image beneath it.
- Re-attached a restored device under the id its launch declared. The emulator
  reports no device id for a node-backed drive, so a restore that read the id
  back out of `query-block` was re-attaching the device under its device
  *path*, which the emulator refuses as a device model name; the properties
  ride as their own arguments for the same reason, with `bootindex` as the
  integer the emulator takes rather than the text of one.
- Waited for a device unplug the emulator actually confirmed. The wait matched
  any earlier detach's `DEVICE_DELETED` still sitting in the monitor's event
  buffer, so a second restore went ahead with the device still attached and
  failed on `Node ... is in use` instead of rolling the member back.
- Reported both halves of a restore's cost. The lane reported its own
  block-graph work as the restore, which is a fraction of a second, while the
  guest's reboot onto the restored disk - the half that decides whether a pool
  member is cheaper to reuse than to boot - was folded into that same figure
  and never measured.
