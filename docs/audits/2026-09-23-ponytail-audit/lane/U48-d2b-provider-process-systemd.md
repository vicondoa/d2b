# U48 d2b-provider-process-systemd
net: 0 lines, 0 deps

- #PR4 [partial] — the deletion half is applied at HEAD (guest_exec.rs, manifest.rs, adoption.rs all absent from src/; `manifest.rs` slash-dir pager and the `adoption.rs` admission-row cluster gone); refused half stays — the dossier at `docs/adr/ADR-046-provider-system-systemd.md:1295,1351,1468` still names `adoption.rs`, and dossiers/ADRs are authoritative context, not audit targets (plan exclusion 5, prior-refusal class #PR4-docs).
- #PR5 [applied] — `src/effect_port.rs` deleted from BOTH process providers (systemd + minijail) at HEAD, with its re-export arms and dossier destination lines; the live conformance `ProcessLaunchEffectPort` (controller.rs:43 `SystemdProcessController<P: ProcessLaunchEffectPort>`) stays as the generic spawn boundary with its effects-service/controller/drain/launch admission tests.

## Consistency notes
(Not applicable: U48 is a process-provider crate, not a types-layer/contracts crate.)

## Reopened refusals
(None reopened: #PR4's dossier-naming half was already [refused] by the prior pass as authoritative-context; no NEW evidence of the blocker changing exists at HEAD.)

## Checked
Read all 11 src/ files (audit 43, controller 129, drain 61, effects_service 401, error 36, launch 16, lifecycle 226, lib 422, metrics 14, operations 1469, sandbox 21 = 2938 lines) + 5 tests (boundaries 68, conformance 336, controller 907, execution_parents 370, lifecycle 175 = 1856). Verified #PR4 deletion half and #PR5 applied half absent from src/ and both process-provider crate trees; verified live caller surface (launch.rs:18 PROVIDER_NAME admission, controller.rs:43 ProcessLaunchEffectPort generic spawn seam, operations.rs callers). Workspace-zero-caller sweep for the deleted-effect-port and dead-module classes confirmed applied. No new findings. [packages/d2b-provider-process-systemd/] (leaf)
