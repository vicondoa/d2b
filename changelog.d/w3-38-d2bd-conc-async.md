### Fixed

- RS-0832: the effect-service binding revision counter (effect_service_actors.rs) and the next-desired-generation mint (provider_effects.rs) now use `Ordering::Relaxed` for their monotonic load/fetch-add/fetch-update seats: these are version-go-tag and unique-value-mint counters used only for staleness equality, so the weakest correct ordering holds and they no longer participate in the SeqCst total order.

- RS-0833: the standalone `broker_epoch` atomic in forward_rendezvous.rs now stores and loads with `Ordering::Relaxed`: the epoch is self-contained and the zones map it gates is mutex-guarded, so no Acquire/Release publication is owed at either seat.
