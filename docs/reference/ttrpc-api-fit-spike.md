# Async ttrpc API-fit contract

The `d2b-ttrpc-api-fit-spike` crate is a hermetic Layer-1 contract test, not a
production service schema. Its checked-in bindings prove that the selected
versions of `ttrpc`, `ttrpc-codegen`, `protobuf`, and `protobuf-codegen`
generate and run genuinely asynchronous unary client and server APIs.

On ttrpc 0.9.0, do not use `ttrpc::r#async::Client::connect` for a Unix socket:
its async wrapper reaches `StdUnixStream::connect_addr` on the executor thread.
Connect with `tokio::net::UnixStream::connect(...).await`, wrap the stream with
the public `ttrpc::r#async::transport::Socket::new`, and pass that socket to
`ttrpc::r#async::Client::new`.

Regenerate the test bindings with:

```console
cd packages
cargo xtask gen-ttrpc-api-fit-spike
```
