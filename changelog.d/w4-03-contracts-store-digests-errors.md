### Fixed

- The store-contract payload digests (`StoredResource.payload_digest`,
  `StoredSchema.payload_digest`, and `PreparedStoreMutation.payload_digest`)
  are now validated SHA-256 digest values (`StateDigest`/`SchemaFingerprint`)
  parsed at the backend boundary, so a non-digest string can no longer flow
  through the store boundary; the wire shape is unchanged.
- A wire payload schema is now validated through the same closed-object and
  `writeOnly` gates as an authored one, so a schema that declares open
  properties or gives a `writeOnly` property a `default`, `enum`, `const`,
  or `examples` value is refused on deserialization instead of admitted.