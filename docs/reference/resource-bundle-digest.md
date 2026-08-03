# Resource bundle digest framing

The Zone resource-bundle and artifact-catalog domains use the
`d2b-digest/v1` framing profile. The SHA-256 input is the UTF-8 bytes of this
canonical JSON object:

```json
{
  "domain": "<domain tag>",
  "framing": "d2b-digest/v1",
  "payload": "<canonical JSON payload as a string>"
}
```

Object keys use the repository's canonical JSON ordering. Keeping `domain` and
`payload` as separate framed fields prevents boundary ambiguity: changing
`("ab", "c")` to `("a", "bc")` changes both the preimage and digest. No raw NUL
separator is part of this production hashing input.

The Nix compiler uses `resources-bundle.nix:framedDigest`; the realised
artifact renderer and catalog use the same JSON frame; Rust runtime
verification uses `framed_canonical_digest`. The domain tags remain
`d2b:v3:resource-bundle` and `d2b:v3:artifact-catalog`.
