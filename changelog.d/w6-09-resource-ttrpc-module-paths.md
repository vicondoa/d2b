### Fixed

- The generated `d2b-resource-api` ttrpc surface no longer carries the
  `d2b_resource_v3` alias module: the generator now rewrites the emitted
  message-module references to the canonical
  `d2b_contracts_resource::resource_proto` path, and the
  `pub use protobuf;` re-export is removed from the published crate root.
  The ttrpc protocol bytes are unchanged.