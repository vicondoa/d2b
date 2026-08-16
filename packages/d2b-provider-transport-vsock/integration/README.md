# Integration fixtures

The package integration targets prove the native-vsock descriptor contract and
the structural no-file-descriptor boundary. Native socket creation remains in
the child-core effect adapter; Provider tests use injected streams and never
open AF_VSOCK directly.
