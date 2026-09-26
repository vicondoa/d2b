### Fixed

- Removed the `declares_state_volume` parameter from `ComponentDescriptor::new`; the constructor always starts a descriptor without a state volume, and the field is set only through `with_state_namespaces`. The wire-only `declaresStateVolume` field and its consistency check against `stateNamespaces` in the `Deserialize` path are unchanged.