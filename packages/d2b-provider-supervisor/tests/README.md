# `d2b-provider-supervisor` hermetic tests

`production_adapter.rs` runs the existing shared conformance suite through the
real bounded adapter for both Process Providers. It also covers launch failure,
vanishing observations, process identifier reuse, wait/reap owner disagreement,
descriptor-open ordering, value-free diagnostics, and async-executor jitter.

Those tests exercise the production adapter and deterministic core-owned effect
owners. They do not claim that a broker, kernel sandbox, or system manager ran;
the declared files under `integration/` name those separate tiers.
