<!-- d2b PR template. The checklist below is MANDATORY and validated by
     repository policy.

     Open or update the PR after focused preflight. Final CI, validator, and
     panel lanes may be pending while the PR is open; all must pass before
     merge.

     Keep validation/panel/seal payloads external. Do not paste raw evidence.
     Do not include AI agent, assistant, or model metadata or tool attribution
     in this PR body. -->

## Summary

<!-- What changed and why. -->

## Stack and immutable tree

- **Dependencies (PRs/branches):** <!-- `none`, or ordered root-to-leaf list -->
- **Base ref and commit:**
- **Head ref and commit:**
- **Stack `prospective_merge_tree_oid` value(s):**
- **Delivery `candidate_id`:**
- **Delivery `content_id`:**
- **Git Town parent graph / ordinary PR status:** <!-- current / needs restack, with check link -->
- **External snapshot/evidence status:** <!-- pending / current, status link only -->

## Required check status

<!-- `pending` is valid at PR opening. Every applicable row must be complete
     and bound to the integrated tree before merge. Summarize status only; do
     not embed command output, panel records, or seal payloads. -->

- **Focused preflight:** <!-- pass + command/check summary -->
- **GitHub CI:** <!-- pending / pass / fail -->
- **Final local/host validator lane:** <!-- pending / pass / fail / justified N/A -->
- **Full ten-role panel:** <!-- pending / 10/10 / findings -->
- **Wave seal and merge eligibility:** <!-- pending / pass / fail -->

## Testing checklist (mandatory before merge)

- [ ] **Focused preflight passed before PR creation/update** for this exact
      candidate.
- [ ] **`make check` passes for the final tree** in the required CI or validator
      lane. This may be pending when the PR opens.
- [ ] **`make test-integration` passes in the final validator lane**, **or**
      state `N/A: pure policy/docs/checklist change with no daemon, broker, NixOS
      module, runtime, network, or generated-artifact behavior change`.
- [ ] **`make test-host-integration` passes in the final validator lane**, **or**
      state `N/A: pure policy/docs/checklist change with no daemon, broker, NixOS
      module, runtime, network, or generated-artifact behavior change`.
- [ ] **Manual `make test-hardware` run** on a NixOS host **with the real
      devices**, if this change touches graphics/GPU, video decode,
      USBIP/YubiKey, hardware-TPM, or a full d2b-microVM boot; otherwise state
      `N/A: no device/passthrough or full-microVM-boot surface touched`.
- [ ] **New/changed tests are represented in the canonical manifest/`xtask`
      graph** and the closed taxonomy remains valid.
- [ ] **Docs + generated CI updated in lockstep** through canonical generation
      commands.
- [ ] **The final tree is unchanged since validation and panel review**; any
      content change requires a new snapshot and both lanes to rerun.
- [ ] **All pending lanes completed and the tree-bound wave seal passed before
      merge**.

## Notes

<!-- Dependency, tree, check-status, release-note, or deferral summaries only. -->
