### Fixed

- `desired_binding_intents` now takes the volume `ResourceRef` by borrow instead of by value, so the volume driver and the shared runtime no longer clone the reference before every binding-intent derivation; the owned references stored inside each `BindingIntent` are cloned from the borrow as before.