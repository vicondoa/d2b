### U53 d2b-provider-credential-secret-service

- [U53](lane/U53-d2b-provider-credential-secret-service.md) — net -50 lines, -0 deps

Family findings (shared home = PR1 `d2b-provider-toolkit/src/credential.rs`):
1. **Deadline/absolute-unix-ms helper trio** triplicated across the credential family — secret-service (lib.rs:49,1501-1527,1596-1603), entra (lib.rs:1165-1212), managed-identity (lib.rs:938-952,1124-1130).
2. **`reject_process_environment_credential_chain` env-scan** triplicated family-wide (secret-service, entra, managed-identity) — the shared env-frame wrapper has no scan body.

Per-crate local shrink possible in this crate under PR3's kept sync surface. Both findings are the remaining instances of the family dedup PR1 was meant to complete.
