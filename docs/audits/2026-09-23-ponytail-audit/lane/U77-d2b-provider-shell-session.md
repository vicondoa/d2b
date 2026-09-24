# U77 d2b-provider-shell-session

## Refusals honored
- #S3 [refused, honored] Cross-cutting: per-type declaration boilerplate in the interaction family (~250 lines claimed) — measured ~200 lines across six crates, only ~70 safely removable; *_spec_decoder() wrappers consumed by each crate's registration test and *Driver/*Factory aliases are public surface
- #S4 [partial, honored] Cross-cutting: dead per-type constants no caller reads — six *_CONTROLLER_REF constants + six re-export arms deleted; paired *_RESYNC constants refused — each is the return value of its own InteractionType::resync()
- #S5 [refused, honored] Cross-cutting: spec_ref duplicated in four crates — no importable shared pointer-ref parser in scope (wayland-policy exposes only key_ref/owned_child_ensure/resource_uid); the four copies stay

## Census (one honest sentence)
Crate = 3 source files (lib.rs 18, shell_session.rs 165) + tests/registration.rs 220 = 403 lines on disk at HEAD, of which 18 are a declaration-only ResourceDriver shared per cross-crate #S3 refusal; workspace-wide caller census shows shell_session surface consumed only through d2bd registration (tests/registration.rs) — all deletions above were already worked by the cross-cutting family rows cited, none remain within this crate's ownership.

net: 0 lines, 0 deps (all U1 rows honored with no new evidence; refusals stand)
