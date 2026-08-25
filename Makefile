# Makefile - d2b repository top-level convenience targets.
#
# Public compatibility targets. Bazel owns Layer-1 target selection,
# dependency ordering, parallelism, caching, and aggregation.

.DEFAULT_GOAL := check

# The dispatcher is deliberately explicit. A target is listed in exactly one
# environment class so a new public lane cannot silently inherit host-local or
# Bazel behavior from a name pattern.
D2B_MAKE_BAZEL_TARGETS := \
	check check-fast check-tier0 bazel-check test-unit \
	test-lint test-rust test-rust-main test-rust-broker \
	test-rust-guest-shell-runner test-rust-local test-rust-no-bash-ast \
	test-rust-schema test-rust-supply-chain test-rust-leaf-main-workspace \
	test-rust-leaf-schema test-rust-leaf-fixture-contracts test-rust-leaf-broker \
	test-rust-leaf-guest-shell-runner test-rust-leaf-no-bash-ast \
	test-rust-leaf-supply-chain test-fixture-contracts test-proofs test-flake \
	test-flake-realized test-flake-aarch64 test-flake-x86 test-nix-unit \
	test-performance-budgets test-drift test-policy test-changelog
D2B_MAKE_LOCAL_TARGETS := \
	check-ci test-integration test-host-integration perf \
	pre-tag smoke-lite heavy-check heavy-test-integration \
	heavy-test-host-integration heavy-flake-check heavy-lane-integration \
	heavy-lane-host-integration heavy-lane-perf heavy-lane-pre-tag \
	heavy-lane-smoke-lite heavy-lane-guard
# Meta helpers that invoke Bazel directly but are not Layer-1 test aliases.
D2B_MAKE_UTILITY_TARGETS := heavy-gate-build changelog-fold

D2B_MAKE_GOALS := $(if $(strip $(MAKECMDGOALS)),$(MAKECMDGOALS),$(.DEFAULT_GOAL))
D2B_MAKE_CLASSIFIED_GOALS := $(filter \
	$(D2B_MAKE_BAZEL_TARGETS) $(D2B_MAKE_LOCAL_TARGETS) \
	$(D2B_MAKE_UTILITY_TARGETS),$(D2B_MAKE_GOALS))
D2B_MAKE_RECURSIVE := $(MAKE)
D2B_MAKE_REENTRY ?= 0
NIX_FLAKE := nix --extra-experimental-features 'nix-command flakes'
D2B_MAKE_SHELL_READY := $(shell \
	if [ "$${D2B_PROJECT_SHELL:-}" = d2b ] && \
	   [ -n "$${D2B_BAZEL_BIN:-}" ] && [ -x "$${D2B_BAZEL_BIN}" ]; then \
		printf 1; \
	else \
		printf 0; \
	fi)

ifneq ($(strip $(D2B_MAKE_CLASSIFIED_GOALS)),)
ifneq ($(D2B_MAKE_SHELL_READY),1)
ifeq ($(D2B_MAKE_REENTRY),0)
D2B_MAKE_DISPATCH_REQUIRED := 1
else
$(error d2b Make dispatcher: re-entry marker is set but the d2b shell contract is incomplete (D2B_PROJECT_SHELL=d2b and executable D2B_BAZEL_BIN are required))
endif
endif
endif

ifeq ($(D2B_MAKE_DISPATCH_REQUIRED),1)
.PHONY: __d2b_make_dispatch $(D2B_MAKE_GOALS)

$(D2B_MAKE_GOALS): __d2b_make_dispatch

__d2b_make_dispatch:
	@set -eu; \
	if ! command -v nix >/dev/null 2>&1; then \
		echo "d2b Make dispatcher: Nix is required for $(D2B_MAKE_GOALS); enter the d2b shell or install Nix" >&2; \
		exit 127; \
	fi; \
	exec $(NIX_FLAKE) \
		develop --no-write-lock-file .#bazel -c \
		env D2B_MAKE_REENTRY=1 $(D2B_MAKE_RECURSIVE) --no-print-directory \
		D2B_MAKE_REENTRY=1 $(D2B_MAKE_GOALS)
else

# Recipe shells must not inherit exported Bash functions from their caller.
# Function resolution precedes PATH lookup, so an inherited cargo/nix/jq
# function could silently redirect a gate that intends to execute a binary.
SHELL := $(CURDIR)/tests/tools/scrub-shell-environment

.PHONY: pre-tag smoke-lite \
        check check-ci check-fast check-tier0 \
        bazel-check \
        test-unit \
        test-lint test-rust test-rust-main \
        test-rust-broker test-rust-guest-shell-runner test-rust-local test-rust-no-bash-ast \
        test-rust-schema test-rust-supply-chain \
        test-rust-leaf-main-workspace \
        test-rust-leaf-schema \
        test-rust-leaf-fixture-contracts test-rust-leaf-broker \
        test-rust-leaf-guest-shell-runner test-rust-leaf-no-bash-ast \
        test-rust-leaf-supply-chain \
        test-fixture-contracts test-proofs test-flake test-flake-realized \
        test-flake-aarch64 test-flake-x86 test-nix-unit \
        test-performance-budgets \
        test-drift test-policy test-changelog \
        test-integration test-host-integration perf \
        heavy-lane-guard heavy-lane-integration heavy-lane-host-integration \
        heavy-lane-perf \
        heavy-lane-pre-tag heavy-lane-smoke-lite \
        heavy-gate-build heavy-gate-provision heavy-check heavy-flake-check \
        heavy-test-integration heavy-test-host-integration \
        clean

# Current Nix system double, used to address per-system flake.checks attrs.
# Falls back to x86_64-linux if `nix` is unavailable (e.g. a docs-only host).
SYSTEM ?= $(shell nix eval --extra-experimental-features 'nix-command flakes' \
	        --impure --raw --expr builtins.currentSystem 2>/dev/null || echo x86_64-linux)

# ===========================================================================
# Test interface. Every Bazel-backed target below dispatches to the matching
# public suite in bazel/checks/BUILD.bazel.
#
#   make check          complete Bazel Layer-1 gate.
#   make check-ci       check + test-integration for local/manual compatibility.
#   make test-<layer>   focused Bazel suite.
#   make test-integration  type-9 container integration; local host/manual pre-PR.
#   make test-host-integration  type-10 runNixOSTest; local NixOS/KVM pre-PR.
#   make heavy-<lane>      the same lane, serialized through the two-slot
#                          per-uid heavy-gate semaphore (see "Heavy lanes").
# ===========================================================================

## check-ci - run the Layer-1 gate, then the conditional container lane.
check-ci:
	$(BAZEL_RUN) //bazel/checks:check
	$(MAKE) test-integration

## check-fast - compatibility alias for check; check-tier0 is the fast subset.

## bazel-check - complete Bazel graph. Locally this defaults to the
## BuildBuddy profile; the facade falls back to local execution when the
## credential is unavailable. CI sets D2B_BAZEL_PROFILE=local.
D2B_BAZEL_PROFILE ?= remote
D2B_BAZEL_TEST_TAG_FILTERS ?= -manual,-gpu,-kvm

BAZEL_RUN = \
	env \
	D2B_BAZEL_JOB="$@" \
	D2B_BAZEL_TEST_TAG_FILTERS="$(D2B_BAZEL_TEST_TAG_FILTERS)" \
	tests/tools/bazel-check --profile "$(D2B_BAZEL_PROFILE)" --
export D2B_BAZEL_PROFILE D2B_BAZEL_TEST_TAG_FILTERS

check-tier0: D2B_BAZEL_TEST_TAG_FILTERS := -gpu,-kvm
test-rust-main: D2B_BAZEL_TEST_TAG_FILTERS := -local,-no-remote-exec,-manual,-exclusive,-gpu,-kvm

$(D2B_MAKE_BAZEL_TARGETS):
	$(BAZEL_RUN) //bazel/checks:$@

# ===========================================================================
# Sub-targets. Each target is a thin alias over one public Bazel suite.
# ===========================================================================

## test-integration - L2 podman container integration tests. Public heavy lane:
## it acquires a heavy-gate slot, then runs the raw work behind the gate so it
## can never oversubscribe a concurrent lane, even when invoked directly or via
## `make check-ci`.
test-integration: heavy-gate-build
	$(HEAVY_GATE) $(MAKE) heavy-lane-integration

## heavy-lane-integration - the raw L2 container work. Internal: reachable only
## from inside the gate (see heavy-lane-guard).
heavy-lane-integration: heavy-lane-guard
	bash tests/test-integration.sh

# ===========================================================================
# Additional targets (helper utilities, legacy aliases, meta gates).
# ===========================================================================

## test-host-integration - G-host: runNixOSTest VM integration tests (the
## `vmChecks` flake output, NOT swept by `nix flake check`). Each test boots a
## real NixOS VM with the d2b daemon surface and asserts live broker /
## daemon / host-posture behaviour (socket activation, bridge isolation,
## state-dir ACLs, broker privilege posture) - the hermetic, non-destructive
## successor to the `D2B_LIVE`-against-the-real-host scripts. Needs KVM (a local
## NixOS host; TCG software emulation is the slow fallback when /dev/kvm is
## absent). x86_64-linux only (a same-system VM builder is required).
## Public heavy lane: acquires a slot, then runs the raw work behind the gate.
## Set D2B_VM_CHECK=<name> to build one named vmChecks entry.
test-host-integration: heavy-gate-build
	$(HEAVY_GATE) $(MAKE) heavy-lane-host-integration

heavy-lane-host-integration: heavy-lane-guard
	@set -eu; \
	system="$$(nix eval --raw --impure --expr builtins.currentSystem)"; \
	if [ "$$system" != "x86_64-linux" ]; then \
	echo "test-host-integration: vmChecks are x86_64-linux only (need a same-system VM builder); skipping on $$system"; \
	exit 0; \
	fi; \
	if [ ! -e /dev/kvm ]; then \
	echo "test-host-integration: /dev/kvm absent - runNixOSTest will fall back to slow TCG emulation"; \
	fi; \
	root="$$(pwd)"; \
	if [ -n "$${D2B_VM_CHECK:-}" ]; then \
	names="$$D2B_VM_CHECK"; \
	else \
	names="$$(nix eval --raw --impure --no-warn-dirty --expr "builtins.concatStringsSep \" \" (builtins.attrNames (builtins.getFlake \"git+file://$$root\").vmChecks.$$system)")"; \
	fi; \
	requested="$${D2B_HOST_VM_CHECK:-}"; \
	if [ -n "$$requested" ]; then \
	case "$$requested" in \
	*[!A-Za-z0-9._-]*) \
	echo "test-host-integration: invalid D2B_HOST_VM_CHECK (use one discovered vmCheck name): $$requested" >&2; \
	exit 1;; \
	esac; \
	fi; \
	if [ -z "$$names" ]; then \
	if [ -n "$$requested" ]; then \
	echo "test-host-integration: unknown vmCheck '$$requested' (available: none)" >&2; \
	exit 1; \
	fi; \
	echo "test-host-integration: no vmChecks present"; \
	exit 0; \
	fi; \
	if [ -n "$$requested" ]; then \
	case " $$names " in \
	*" $$requested "*) names="$$requested";; \
	*) \
	echo "test-host-integration: unknown vmCheck '$$requested' (available: $$names)" >&2; \
	exit 1;; \
	esac; \
	fi; \
	run_dir="$$(mktemp -d "$${TMPDIR:-/tmp}/d2b-host-integration.XXXXXX")"; \
	chmod 700 "$$run_dir"; \
	cleanup() { rm -rf -- "$$run_dir"; nix-store --gc --print-roots >/dev/null 2>&1 || true; }; \
	trap cleanup EXIT; \
	trap 'exit 129' HUP; \
	trap 'exit 130' INT; \
	trap 'exit 143' TERM; \
	trap 'exit 131' QUIT; \
	attic_cache=""; \
	attic_config=""; \
	if [ -n "$${XDG_CONFIG_HOME:-}" ]; then \
	attic_config="$$XDG_CONFIG_HOME/attic/config.toml"; \
	elif [ -n "$${HOME:-}" ]; then \
	attic_config="$$HOME/.config/attic/config.toml"; \
	fi; \
	if ! command -v attic >/dev/null 2>&1; then \
	echo "test-host-integration: Attic unavailable - skipping closure upload"; \
	elif [ -z "$$attic_config" ] || [ ! -e "$$attic_config" ]; then \
	echo "test-host-integration: Attic config absent - skipping closure upload"; \
	else \
	fail_attic_state() { echo "test-host-integration: configured Attic state is invalid or ambiguous" >&2; exit 1; }; \
	attic_meta="$$(ATTIC_CONFIG="$$attic_config" nix eval --impure --json --expr 'let config = builtins.fromTOML (builtins.readFile (builtins.getEnv "ATTIC_CONFIG")); names = builtins.attrNames (config.servers or {}); server = if config ? "default-server" then config."default-server" else if builtins.length names == 1 then builtins.head names else throw "ambiguous Attic servers"; endpoint = config.servers.$${server}.endpoint or (throw "missing Attic endpoint"); in { inherit server endpoint; }' 2>/dev/null)" || fail_attic_state; \
	attic_server="$$(printf '%s' "$$attic_meta" | jq -er '.server | select(test("^[A-Za-z0-9][A-Za-z0-9._+-]*$$"))')" || fail_attic_state; \
	attic_base="$$(printf '%s' "$$attic_meta" | jq -er '.endpoint | capture("^(?<scheme>https?)://(?<authority>[^/@?#]+)(?:/[^?#]*)?$$") | ((.scheme | ascii_downcase) + "://" + (.authority | ascii_downcase))')" || fail_attic_state; \
	attic_name="$$(nix config show --json | jq -er --arg base "$$attic_base" '.substituters.value | if type == "string" then split(" ") else . end | map(try capture("^(?<scheme>https?)://(?<authority>[^/@?#]+)(?<path>/[^?#]*)?(?:\\?[^#]*)?$$") catch empty | select(((.scheme | ascii_downcase) + "://" + (.authority | ascii_downcase)) == $$base) | ((.path // "") | rtrimstr("/") | split("/") | last)) | map(select(test("^[A-Za-z0-9][A-Za-z0-9_+-]*$$"))) | unique | select(length == 1) | .[0]')" || fail_attic_state; \
	attic_cache="$$attic_server:$$attic_name"; \
	if ! attic cache info "$$attic_cache" >"$$run_dir/attic-info.log" 2>&1; then \
	echo "test-host-integration: configured Attic cache preflight failed" >&2; \
	exit 1; \
	fi; \
	echo "test-host-integration: Attic cache preflight passed"; \
	fi; \
	echo "test-host-integration: building host tools with local Bazel"; \
	'$(BAZEL_BIN)' build --config=local \
	//packages/d2b:d2b \
	//packages/d2bd:d2bd \
	//packages/d2b-priv-broker:d2b-priv-broker \
	//packages/d2b-host:d2b-activation-helper \
	//packages/d2b-host-activation-helper:d2b-host-activation-helper \
	//packages/d2b-cutover:d2b-cutover-runner \
	//packages/d2b-unsafe-local-helper:d2b-unsafe-local-helper \
	//packages/d2b-resource-compiler:d2b-resource-compiler \
	//packages/d2b-provider-display-wayland:d2b-wayland-proxy; \
	bazel_bin="$$(realpath -e "$$('$(BAZEL_BIN)' info --config=local bazel-bin)")"; \
	stage="$$run_dir/bundle"; \
	mkdir -m 700 "$$stage"; \
	stage_tool() { source="$$(realpath -e "$$bazel_bin/$$1")"; case "$$source" in "$$bazel_bin"/*) ;; *) echo "test-host-integration: Bazel output escaped bazel-bin" >&2; return 1;; esac; [ -f "$$source" ] && [ -x "$$source" ] || { echo "test-host-integration: invalid Bazel output $$1" >&2; return 1; }; install -m 755 "$$source" "$$stage/$$2"; }; \
	stage_tool packages/d2b/d2b d2b; \
	stage_tool packages/d2bd/d2bd d2bd; \
	stage_tool packages/d2b-priv-broker/d2b-priv-broker d2b-priv-broker; \
	stage_tool packages/d2b-host/d2b-activation-helper d2b-activation-helper; \
	stage_tool packages/d2b-host-activation-helper/d2b-host-activation-helper d2b-host-activation-helper; \
	stage_tool packages/d2b-cutover/d2b-cutover-runner d2b-cutover-runner; \
	stage_tool packages/d2b-unsafe-local-helper/d2b-unsafe-local-helper d2b-unsafe-local-helper; \
	stage_tool packages/d2b-resource-compiler/d2b-resource-compiler d2b-resource-compiler; \
	stage_tool packages/d2b-provider-display-wayland/d2b-wayland-proxy d2b-wayland-proxy; \
	echo "test-host-integration: staged Bazel host-tool bundle"; \
	set --; \
	for name in $$names; do \
	set -- "$$@" "git+file://$$root#vmChecks.$$system.$$name"; \
	done; \
	echo "test-host-integration: building vmChecks: $$names"; \
	D2B_HOST_TOOL_BUNDLE="$$stage" D2B_HOST_RUNTIME_PATH="$$run_dir/absent-host-runtime.json" \
	nix build --impure --out-link "$$run_dir/result" --print-build-logs --print-out-paths "$$@" >"$$run_dir/outputs"; \
	cat "$$run_dir/outputs"; \
	if [ -n "$$attic_cache" ]; then \
	: >"$$run_dir/attic-closure-all"; \
	while IFS= read -r output; do \
	drv="$$(nix-store -qd "$$output")" || { \
	echo "test-host-integration: could not resolve a vmCheck derivation for Attic" >&2; \
	exit 1; \
	}; \
	if ! nix-store -qR --include-outputs "$$drv" >>"$$run_dir/attic-closure-all"; then \
	echo "test-host-integration: could not resolve a vmCheck dependency closure for Attic" >&2; \
	exit 1; \
	fi; \
	done <"$$run_dir/outputs"; \
	sort -u -o "$$run_dir/attic-closure-all" "$$run_dir/attic-closure-all"; \
	awk 'NR == FNR { skip[$$0] = 1; next } !skip[$$0]' \
	"$$run_dir/outputs" "$$run_dir/attic-closure-all" >"$$run_dir/attic-closure"; \
	if [ ! -s "$$run_dir/attic-closure" ]; then \
	echo "test-host-integration: no dependency closure paths available for Attic" >&2; \
	exit 1; \
	fi; \
	if ! attic push --stdin "$$attic_cache" <"$$run_dir/attic-closure" >"$$run_dir/attic-push.log" 2>&1; then \
	echo "test-host-integration: Attic closure upload failed" >&2; \
	exit 1; \
	fi; \
	echo "test-host-integration: Attic closure upload succeeded"; \
	fi

## Public heavy lanes: acquire a slot, then run the raw work behind the gate.
perf: heavy-gate-build
	$(HEAVY_GATE) $(MAKE) heavy-lane-perf

heavy-lane-perf: heavy-lane-guard
	$(BAZEL_RUN) //bazel/checks:test-performance-budgets

## heavy-lane-guard - fail closed when a heavy-lane internal target is invoked
## outside the gate. It does not trust the mere presence of D2B_HEAVY_GATE
## (any process can export that); instead it asks the wrapper to verify that
## this process genuinely holds a slot via its open file description lock.
##
## verify-slot reports its verdict purely through its exit status, so branch on
## the typed codes rather than collapsing every nonzero status into one:
##
##   0  a genuinely held slot            -> proceed
##   3  no slot is held                  -> reacquire by running the PUBLIC lane
##                                          (which acquires a slot and re-runs
##                                          this lane through the gate). A shared
##                                          prerequisite cannot exec that itself
##                                          without double-running the parent
##                                          recipe, so guide the operator to the
##                                          acquiring entrypoint and fail closed
##                                          with the typed unheld code.
##   *  the verifier itself malfunctioned -> propagate the exact code unchanged
##                                          and fail closed
##
## Collapsing 3 and every malfunction into one "outside the semaphore" exit hid
## a broken gate behind a slot-bypass message; keeping the codes distinct lets a
## caller tell "ran the raw target directly" apart from "the verifier is broken".
heavy-lane-guard: heavy-gate-build
	@rc=0; $(HEAVY_GATE_BIN) heavy-gate verify-slot || rc=$$?; \
	if [ "$$rc" -eq 0 ]; then \
	  exit 0; \
	elif [ "$$rc" -eq 3 ]; then \
	  echo "heavy lane invoked outside the heavy-gate semaphore (no slot held)." >&2; \
	  echo "Run the public lane (e.g. 'make test-integration'), which acquires a slot" >&2; \
	  echo "and re-runs this lane through the gate; do not run the internal target directly." >&2; \
	  exit "$$rc"; \
	else \
	  echo "heavy-gate verify-slot failed closed (exit $$rc); refusing to run heavy work unsynchronised." >&2; \
	  exit "$$rc"; \
	fi

# ===========================================================================
# Heavy lanes.
#
# Every Layer-2, host-integration, live, and perf-heavy command runs
# through ONE semaphore: `xtask heavy-gate`. It grants two slots per uid
# via open file description locks, so concurrent lanes cannot oversubscribe the
# shared Nix store or KVM device. Do not add a second
# lock file, sleep-and-retry loop, or per-crate heavy-lane guard.
#
# Run the heavy-* target instead of the bare target whenever another heavy lane
# might be running; the bare targets stay available for a serial console.
# ===========================================================================

BAZEL_BIN ?= $(if $(D2B_BAZEL_BIN),$(D2B_BAZEL_BIN),bazel)
HEAVY_GATE_BIN := $(CURDIR)/bazel-bin/packages/xtask/xtask
HEAVY_GATE = $(HEAVY_GATE_BIN) heavy-gate --

## heavy-gate-build - build the semaphore wrapper through its Bazel owner.
heavy-gate-build:
	'$(BAZEL_BIN)' build --config=local //packages/xtask:xtask

## heavy-gate-provision - create or repair the protected slot namespace for the
## current numeric uid without resolving a user name through NSS. This is the
## explicit post-login path for network-backed users and the developer setup
## path on hosts that do not consume the NixOS module. Because /run is a tmpfs,
## run it once per boot when the gate reports missing provisioning. It never
## creates a fallback under a user-owned root.
heavy-gate-provision:
	@target_uid="$$(id -u)"; \
	sudo -- sh -eu -c '\
	  target_uid="$$1"; root=/run/d2b-heavy-gates; \
	  case "$$target_uid" in ""|*[!0-9]*) echo "heavy-gate provisioning: invalid target uid" >&2; exit 1;; esac; \
	  if [ -L "$$root" ] || { [ -e "$$root" ] && [ ! -d "$$root" ]; }; then echo "heavy-gate provisioning: refusing an unsafe runtime root" >&2; exit 1; fi; \
	  install -d -m 0755 -o root -g root "$$root"; \
	  uid_dir="$$root/uid-$$target_uid"; \
	  if [ -L "$$uid_dir" ] || { [ -e "$$uid_dir" ] && [ ! -d "$$uid_dir" ]; }; then echo "heavy-gate provisioning: refusing an unsafe per-user directory" >&2; exit 1; fi; \
	  install -d -m 0755 -o root -g root "$$uid_dir"; \
	  for index in 0 1; do \
	    slot="$$uid_dir/slot-$$index"; \
	    if [ -L "$$slot" ] || { [ -e "$$slot" ] && [ ! -f "$$slot" ]; }; then echo "heavy-gate provisioning: refusing an unsafe slot file" >&2; exit 1; fi; \
	    if [ ! -e "$$slot" ]; then install -m 0600 -o "$$target_uid" -g root /dev/null "$$slot"; else chown "$$target_uid":root "$$slot"; chmod 0600 "$$slot"; fi; \
	  done; \
	  echo "heavy-gate provisioning: protected slots are ready for this boot"' \
	  sh "$$target_uid"

## heavy-check - the Layer-1 PR-equivalent gate under the heavy-lane semaphore.
heavy-check: heavy-gate-build
	$(HEAVY_GATE) $(BAZEL_RUN) //bazel/checks:check

## heavy-test-integration / -host-integration - explicit aliases for
## the public heavy lanes, kept for muscle memory and scripts. The public lanes
## now acquire the semaphore themselves; a redundant outer gate here is safe
## because the inner invocation verifies and reuses the inherited slot.
heavy-test-integration: test-integration
heavy-test-host-integration: test-host-integration

## heavy-flake-check - the building `nix flake check` under the semaphore.
##                     `make test-flake` is the cheap --no-build sibling.
heavy-flake-check: heavy-gate-build
	$(HEAVY_GATE) $(NIX_FLAKE) flake check --print-build-logs

# --- pre-existing maintainer targets ---------------------------------------

## pre-tag - run the full live-VM smoke gate before tagging a release.
##           Requires: KVM, d2b active, both personal-dev and work-aad VMs declared.
##           Exits non-zero on any probe failure.  Updates $${TMPDIR:-/tmp}/d2b-smoke-run-log.txt.
##           Public heavy lane: acquires a slot, then runs the raw live work behind
##           the gate - the live smoke suite is the most destructive, stateful lane
##           in the tree and must never bypass the sole-use semaphore.
pre-tag: heavy-gate-build
	$(HEAVY_GATE) $(MAKE) heavy-lane-pre-tag

## heavy-lane-pre-tag - the raw full live-VM smoke work. Internal: reachable only
## from inside the gate (see heavy-lane-guard).
heavy-lane-pre-tag: heavy-lane-guard
	bash tests/integration/live/live-vm-smoke.sh --full

## smoke-lite - run the single-VM lite smoke gate (≤5 min).
##              Public heavy lane: acquires a slot, then runs the raw live work
##              behind the gate.
smoke-lite: heavy-gate-build
	$(HEAVY_GATE) $(MAKE) heavy-lane-smoke-lite

## heavy-lane-smoke-lite - the raw lite live-VM smoke work. Internal: reachable
## only from inside the gate (see heavy-lane-guard).
heavy-lane-smoke-lite: heavy-lane-guard
	bash tests/integration/live/live-vm-smoke.sh --lite

.PHONY: changelog-fold

## test-changelog - the changelog policy gate (also the CI test-changelog job).
##                  Requires code changes to ship release notes as either a
##                  CHANGELOG.md entry or a changelog.d/ fragment, and validates
##                  the structure of every fragment present.
## changelog-fold - fold every changelog.d/ fragment into the CHANGELOG.md
##                  '## [Unreleased]' block and delete the consumed fragments.
##                  Run at merge time; see changelog.d/README.md.
changelog-fold:
	'$(BAZEL_BIN)' run --config=local //packages/xtask:xtask -- changelog-fold
# ===========================================================================
# Disk hygiene.
#
#   make clean   Remove this worktree's build output directories and scratch
#                tree, then collect unreferenced Nix store paths. The shared
#                sccache directory is deliberately kept, so the next build
#                re-links rather than recompiling from scratch.
#
# Knobs: D2B_CLEAN_DRY_RUN=1, D2B_CLEAN_SKIP_GC=1, D2B_CLEAN_KEEP_SCRATCH=1.
clean:
	bash tests/tools/clean-worktree.sh

endif
