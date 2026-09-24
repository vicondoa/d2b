# U44 d2b-provider-transport-azure-relay

Lean already. Ship.

## Checked
Full-source read of every module (`auth.rs` 411, `backpressure.rs` 78,
`credential_client.rs` 531, `guest_credential.rs` 1,041, `guest_zone_link.rs`
588, `relay_transport.rs` 1,498, `transport_settings.rs` 90) plus the
`tests/fake_relay_transport.rs` (1410) conformance/carrier-support surface.
Scanned for the prior finding's classes: duplicate SAS mint (`mint_sas` /
`build_connect` single declarations, `src/auth.rs:184,226`, consumed by
`relay_transport.rs` `open_inner`), duplicate hand-rolled hex/digest renderers
(`digest_hex` declared once, `guest_zone_link.rs:379`, single call
`guest_zone_link.rs:272`), duplicated identifier/namespace validators
(`valid_namespace`/`valid_entity` once each, `transport_settings.rs:69,84`),
and clock-seam triplication (two remote-units helpers, `guest_zone_link.rs:26`
secs, `guest_credential.rs:665` ms - each locally used, separate units, not a
fold). Re-verifd all blocker ledger rows for this crate: prior #S9 [applied]
(already deleted ~580 lines: sealed-credential write half, `RelayTransportService`
and handles, `src/reconnect.rs`, duplicate `mint_sas`, unbound-acquire default,
audit/metrics). Re-opened nothing. No new evidence of zero-caller surface.

net: 0 lines, 0 deps
