# d2b-provider-transport-azure-relay — unit-test audit
tests: 21 · src files: 8
net: -0 tests, -0 lines

## Findings (biggest net first)
- gap: BadSchemaVersion guard — unsupported sealed-envelope `schema_version` rejected (src/guest_credential.rs:load_sealed_inner) — no test anywhere in crate (src or tests/) exercises a non-1 schema_version; security-relevant fail-closed path. 
- gap: BadFileType guard — credential path must be a regular file (src/guest_credential.rs:read_policy_file) — no test opens a non-regular path (e.g. directory) anywhere; defensive against fifo/socket/device surprises.

- gap: BadSealKey guard — `SealingKey::load` rejects wrong-length key bytes (src/guest_credential.rs:SealingKey::load) — every test writes a correct 32-byte key file;malformed-length key never exercised. 

Nothing to cut. Ship. Checked all 21 unit fns across src/auth.rs, src/guest_credential.rs, src/guest_zone_link.rs against product code and same-crate tests/*.rs; every test pins a distinct behavior, no duplicate or trivial candidate found.

## Keep
- `auth_debug_and_errors_redact_canary_material` — pins RelayCredential Debug redacts bearer secret; also exercises oversize-TTL build_connect rejection. 
- `auth_rejects_empty_and_zero_lifetime_inputs` — pins mint_sas empty key_name + zero TTL rejections and build_connect empty Entra bearer → InvalidCredential. 
- `connect_debug_redacts_url_and_header_material` — pins RelayConnect Debug redacts SAS query + ServiceBusAuthorization header. 
- guest_credential: 13 keepers pin sealed-envelope policy fail-closed (BadMode/NixStorePath/BadOwner), plaintext-load 0600+Debug-redaction, unseal roundtrip + metadata + file mode + no-plaintext-in-file, AAD absent-vs-zero-expiry distinction,(wrong-key Crypto,(port exact-binding acquire/revoke + Debug redaction,(drop-hook active-row cleanup,(from_sealed no materialization + path redaction,(legacy-plaintext generation-0 guard,(load_sealed + port expiry fail-closed. 
- guest_zone_link: 5 keepers pin RelayRole→credential-role mapping,(from_sealed placement guard before credential open,,(scoped compose with no sealed credential + ObservationUnavailable,(open-marker schemaVersion/generation/digest/mode/no-secret,(missing/invalid sealed open emits no marker.