### Fixed

- The effect-service binding revision counters and the broker-epoch generation mint now use the weakest correct orderings (`Relaxed` loads/stores/fetch_updates) instead of `SeqCst`: the revision is a version tag read only for staleness equality and the generation is a unique-value mint, so the standalone atomics carry no paired publication needing acquisition/release (plan U23 ordering-seat relaxations).
