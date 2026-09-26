### Changed

- `d2bd` audio mutations (RS-0537): a `setVolume` / `mute` request that names a
  VM absent from the public manifest, or a VM whose manifest entry does not
  declare audio, is refused with the structured
  `TypedError::AudioVmNotFound` (`kind` `audio-vm-not-found`, exit `2`) and
  `TypedError::AudioNotEnabled` (`kind` `audio-not-enabled`, exit `70`) instead
  of a flattened `TypedError::InternalIo { context, detail }`. The audio status
  path already reported the same per-VM classes through `AudioErrorKind`, so a
  mutation caller now tells a user-input refusal from an internal I/O failure
  by `kind`/exit code instead of by string-matching the message text. The rows
  documented in `docs/reference/error-codes.md` do not move; the two new daemon
  wire kinds are documented there.

### Fixed

- `d2bd` `TypedError` (RS-0963): the audio mutation refusals no longer collapse
  their failure class into a `String` detail on the `internal-io` variant -
  the class travels as a typed variant, the way the status path carries
  `AudioErrorKind`. Lock, read, write, and host-enforcement failures keep their
  `internal-io` classification and their log-only detail, and the
  `internal-io` envelope text is unchanged.
