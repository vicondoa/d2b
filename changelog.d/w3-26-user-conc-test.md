### Changed

- The user provider's scripted test doubles now use relaxed atomic ordering for their scripted-failure flags, matching the single-threaded test runtime they run on.
- The user provider reconciliation test now re-discovers a cached unrealized discovery for every unrealized phase (Pending, Degraded, Unknown), not only Pending, so a widened realized-phase short-circuit can no longer pass the suite unnoticed.