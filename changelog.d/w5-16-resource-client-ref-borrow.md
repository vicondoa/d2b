### Fixed

- `ResolvedTarget::matches_assignment` now compares the `Execution` assignment reference against each owner form by borrow: the stored reference for a `Resource` owner and the resource type and name components for `Guest`, `Provider`, and `Host` owners, instead of materializing a `ResourceRef` and cloning just to compare (`target.rs`). The by-value `resource_ref()` accessors are unchanged.
