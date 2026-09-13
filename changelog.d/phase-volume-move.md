### Changed

- The Volume and VolumeBinding drivers now live in their own
  `d2b-provider-volume` and `d2b-provider-volume-binding` crates, together
  with their spec decoders, their effect ports, and the driver declarations
  the resource plane registers the types by. The daemon keeps the production
  effect implementations behind those ports - the volume-local layout effect
  and its durable probe, and the binding serving socket, its removal, and the
  guest-mount observation - so the family crates carry no host state. The
  registry serves each type's decoder and factory from its declaration
  instead of a hand-built provider and decoder table, and the
  `volume_driver`, `binding_driver`, and `binding_child_resource_runtime`
  daemon modules are gone.
- The VolumeBinding declaration licenses the two children the driver mints:
  the worker `Process` served by `Provider/system-minijail` and the `Endpoint`
  served by `Provider/volume-virtiofs`, each with its creation rank. The
  Volume declaration licenses the `VolumeBinding` child its admitted
  attachments derive. Both family crates take those provider references from
  the provider crates that own them, and the binding driver's own child
  retirement order is derived from the declaration's ranks rather than from a
  second table.
- The two binding row readers (`binding_readiness_current`,
  `parsed_binding_spec`) move to the binding crate, and the daemon reads
  stored binding rows through them. `d2b-provider-volume-virtiofs` exports
  its canonical `PROVIDER_REF` so the declaring crates stop respelling it.
  Operator-visible behavior is unchanged: the same volume and binding shapes
  are admitted, the same validate, recover, reconcile, finalize, and delete
  verbs run, and the worker/endpoint teardown order is preserved.
