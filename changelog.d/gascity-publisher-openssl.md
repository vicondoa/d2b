### Fixed

- Fixed Gas City GitHub publication startup to sign App JWTs with the
  packaged immutable OpenSSL executable instead of relying on the service
  PATH.
- Fixed repository installation identity reads to authenticate with the
  GitHub App JWT instead of an installation token.
- Fixed rate-limited GitHub 403 responses to retry with bounded provider
  hints while keeping ordinary authorization failures permanent.
- Fixed GitHub pull-request reconciliation to query the exact owner and branch
  with a bounded response instead of fetching the full pull-request history.
