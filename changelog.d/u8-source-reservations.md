Unify source reservations and add the pre-drain lifecycle stage.

The broker gains one reservation owner that serializes each source claim:
a source provider still decides semantic admission, and helpers bind an
explicitly attenuated leg of the parent's reservation instead of taking a
competing writer or device claim. The pending/effect/close/release record
rides the broker's existing durable state cells, so a claim is pre-committed
before its handle exists and retired when it is released.

The resource runtime gains a pre-drain lifecycle stage that runs before the
generic children-first finalization. ResourceManager commits the durable
deleting mark and blocks new child authority for the row, the driver fences
its own use and closes its consumer and helpers, and only a successful
pre-drain authorizes the cascade. Cancellation is handled from every state,
so a request cancelled before it reserved anything and a relationship that
was prepared but never active both finish without waiting for consumer
activity that cannot exist.

The authority index no longer carries Guest-specific ownership: the
teardown participant is a family-neutral identity the owning controller
supplies, and the common reservation record is extracted from the storage row
so every claim owner advances one shape. Child dependency edges can be
scoped to a stage, so a binding helper references its own parent reservation
without requiring the parent to be active.
