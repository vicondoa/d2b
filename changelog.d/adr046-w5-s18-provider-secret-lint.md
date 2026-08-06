### Security

- Provider configuration and Device extension settings now fail closed when
  secret-shaped keys or values would be embedded in the resource bundle.

### Fixed

- Signed Device extension metadata now validates bounded schema versions and
  applies provider settings bounds only when matching schema metadata exists.
