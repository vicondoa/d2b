# Gas City BuildBuddy fixture

This fixture is pinned to Bazel **8.7.0** by `.bazelversion`.  It is a
hermetic contract fixture, not a live BuildBuddy endpoint:

- `fake_upstream.py` accepts only the proxy-injected
  `x-buildbuddy-api-key` header.
- `/cache/<key>` covers authenticated cache upload and download.
- `/execute` covers an authenticated remote-execution round trip.
- `test_round_trip.py` never contacts `remote.buildbuddy.io`.

The production module fixes the real upstream to `remote.buildbuddy.io:443`,
uses Envoy HTTP/2 with CA/SNI/SAN verification, and keeps the key in the
proxy credential boundary.  The uncredentialed runner is represented here by
the client that can only make requests after the proxy has injected the
header.
