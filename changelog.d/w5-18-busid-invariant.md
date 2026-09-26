### Fixed

- `d2b_host::media::BusId` now enforces the USB busid grammar at the type boundary: the inner `String` is private, `BusId::new` validates via `media::validate_usb_busid` and returns `Result`, and a `TryFrom<&str>` conversion is provided; the `#[serde(transparent)]` wire shape is unchanged.
- Removed the redundant busid re-validation in the broker qemu-media ops (`enroll`, `detach`, and the runtime selector path now convert the wire busid once through `BusId::try_from`) and the daemon-side attach/detach pre-checks; the grammar is enforced once at the type boundary.
- Added `BusId::as_str` for reading the wrapped busid; carveout rendering and all construction sites use the typed value.