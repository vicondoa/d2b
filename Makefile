# Makefile — nixling repository top-level convenience targets.
#
# Maintainer-facing targets only; CI uses .github/workflows/*.yml directly.

.PHONY: pre-tag smoke-lite i3-check

## i3-check — verify no v1.3 deferrals authored (ADR 0022 I3 invariant).
##            Wired into pre-tag and tests/static.sh per panel-docs R1 MF-1.
i3-check:
	bash tests/no-new-deferral.sh

## pre-tag — run the full live-VM smoke gate before tagging a release.
##           Requires: KVM, nixling active, both personal-dev and work-aad VMs declared.
##           Exits non-zero on any probe failure.  Updates tests/smoke-run-log.txt.
##           ALSO runs the I3 invariant grep gate (ADR 0022 + panel-docs R1).
pre-tag: i3-check
	bash tests/live-vm-smoke.sh --full

## smoke-lite — run the single-VM lite smoke gate (≤5 min).
##              Used at every panel-round HEAD per I5.
smoke-lite:
	bash tests/live-vm-smoke.sh --lite
