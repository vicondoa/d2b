### Fixed

- Daemon `internal-io`, `internal-config-invalid`, and
  `internal-broker-unavailable` refusals now keep the error they were raised
  from. `d2bd_runtime::typed_error::TypedError` carries an `Option<ErrorSource>`
  on those three variants, so `std::error::Error::source()` walks back to the
  failing `io::Error`, serde, nix, or transport error instead of stopping at
  the rendered `detail` string, and the daemon's raw-detail logging records the
  whole cause chain in an `origin` field (`origin: cause: root`) rather than
  one collapsed sentence. Every construction site that used to render an error
  into `detail` now attaches that error, and sites whose detail is not derived
  from an error pass `None`.
- The daemon wire error surface does not move: the public envelope is still
  rendered from `kind`, `exitCode`, `message`, and `remediation` only, and the
  operator-visible `detail` strings are byte-for-byte unchanged. `TypedError`
  is not `Serialize`, so an attached origin is unreachable by any serializer;
  a future `Serialize` derive on the enum must keep the field behind
  `#[serde(skip)]`.
