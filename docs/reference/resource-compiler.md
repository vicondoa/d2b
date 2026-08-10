# Resource compiler

The Zone resource bundle is produced by the Rust resource compiler during the
Nix build. The compiler receives one JSON declaration containing the selected
Zone resources, the realised artifact catalog, Provider package inputs, the
committed schema farm, and the strict-secret policy. It writes one immutable
resource-bundle JSON output.

The build entry point is:

```text
d2b-resource-compiler compile \
  --input <declared-input.json> \
  --output <resource-bundle.json> \
  [--strict-secrets]
```

The input and output paths are build declarations, not search roots. The
compiler never scans a package directory for an artifact and never resolves a
Provider through `PATH`.

## Build checks

For every declared Provider package, the compiler:

- anchors the selected output and resolves layout entries with Linux
  `openat2`, `RESOLVE_BENEATH`, `RESOLVE_NO_SYMLINKS`, and
  `RESOLVE_NO_MAGICLINKS`;
- verifies the detached Ed25519 signature before reading other metadata;
- requires the fixed manifest, detached signature, and config-schema files;
- requires canonical `d2b-cjson/v1` bytes for the manifest and schema;
- enumerates `bin/`, checks names, regular-file type, ELF64 format, and execute
  bits, then compares executable bytes and set digests;
- enforces `BinaryRef` and `ComponentExecution` agreement;
- compares catalog, signed-manifest, schema, package, and recomputed digests;
- rejects duplicate Provider-exported ResourceTypes.

For the resource bundle, it validates the declared schemas, Zone ownership,
canonical ordering, content hash, artifact-catalog digest, and strict
secret-shaped material policy. Diagnostics are bounded and do not include
store paths, manifest contents, configuration values, or key material.

Interpreted Provider entry points must be packaged with
`d2b.lib.buildProviderElfShim`. The shim is built and checked as an ELF
executable before the compiler can admit it as a Provider entry point.
