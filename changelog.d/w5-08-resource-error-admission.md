### Fixed

- ResourceError wire admission now routes deserialization through the validating constructor, so payloads carrying a revision for a kind that forbids it, or inconsistent retry fields, are rejected on the wire instead of admitted past the constructor invariants. The wire shape is unchanged.