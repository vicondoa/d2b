### Changed

- Moved the validated USB busid type to `d2b_host::media::BusId`, beside the busid grammar validator it enforces, so broker media and firewall call sites build the validated value from the host media vocabulary instead of naming the `nftables` module; `d2b_host::nftables` now renders the validated value without owning the type.
