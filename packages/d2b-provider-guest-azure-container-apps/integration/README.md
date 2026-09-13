# ACA Provider integration fixtures

Hermetic lifecycle coverage uses the Provider's typed effect ports and
in-process fakes:

```text
make test-rust
```

The fixture path never uses Azure credentials, network access, a broker, or a
host socket. Live validation is manual-only and must be explicitly enabled by
the deployment harness.
