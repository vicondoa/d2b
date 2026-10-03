### Fixed

- Fixed `runtime::tests::the_installed_admitted_effect_table_serves_no_operation` reading the
  process-wide test bundle slot instead of the wiring it meant to observe. Under `cfg(test)` the
  broker's resolver prefers that injected slot over the configured bundle path, and the spawn
  kernel tests introduced by the bundle-intent fence hold a real bundle in it for the whole test,
  so a concurrent case could answer this assertion with a neighbour's fixture. The case now owns
  the slot: it is pinned empty while it asserts that an absent verified bundle resolves nothing,
  and a real verified bundle is installed while it asserts the resolved table's own rows.