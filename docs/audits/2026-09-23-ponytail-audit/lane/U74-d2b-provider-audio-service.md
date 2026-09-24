# U74 d2b-provider-audio-service

## Verdict
One dead alias, one re-export arm. net: -2 lines, -0 deps.

## Findings
- <tag:shrink> `pub type AudioServiceDriver = InteractionDriver<AudioService>` is
  read by zero sites - not by the descriptor (which constructs the generic
  `InteractionDriverFactory` directly), not by any other module in the crate,
  and by no file outside the crate (workspace `.rs` grep, `--include
  "*.rs"`, excl. this crate + bazel-out). The only textual appearances are
  the declaration (`audio_service.rs:76`) and its lib.rs re-export arm
  (`lib.rs:17`). Delete the alias and the arm; the live generic
  `InteractionDriver<AudioService>` the descriptor registers is what the
  family seam serves. [packages/d2b-provider-audio-service/src/audio_service.rs]
  (leaf)

## Not findings (keep)
- `AudioServiceFactory`, `audio_service_spec_decoder`, `AUDIO_SERVICE_RESYNC`:
  each has exactly one in-crate reader (the descriptor at audio_service.rs:
  104-107) plus their re-export arms - constructor/const/vocab are all live
  on the descriptor's own registration pathastra.
- `AudioService` (the typed row vocab): 30 outside readers - the toolkit and
  plane seams resolve the type's interactions by it. Canonical home, keep.

## Verified
Each of the four names above has a single declaration site in this crate and
its caller edges were counted by workspace-wide `grep -rlw name
--include=*.rs` excluding this crate's own src and bazel-out. `AudioService`
has 30 reader files; the three live declaration facets have 1 in-crate reader
each; `AudioServiceDriver` has 0. The type alias is the only genuinely
uncalled public item in the crate.
## U1 execution (2026-09-24)

Finding applied. R4 re-verified at HEAD: `AudioServiceDriver` zero readers workspace-wide (only def + lib arm + README prose).

- Deleted `pub type AudioServiceDriver = InteractionDriver<AudioService>;` + doc (audio_service.rs) and the `AudioServiceDriver,` arm in the lib.rs re-export; now-unused `InteractionDriver` import dropped from the `d2b_provider_wayland_policy::interaction` use block. Live `AudioServiceFactory`/`audio_service_spec_decoder`/`AUDIO_SERVICE_RESYNC`/`AudioService` untouched.

`cargo test -p d2b-provider-audio-service`: PASS (4 registration tests + doc-tests; 0 failures).
