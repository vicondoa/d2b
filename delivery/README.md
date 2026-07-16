# Delivery authorities and shared ownership

`delivery/manifest.json` remains the W4 authority and is not moved or rewritten.
W5, W6, and W7 own only:

| Wave | Authority | Implementation ownership |
| --- | --- | --- |
| W5 | `delivery/manifests/w5.json` | Runtime service and dispatch implementation, consuming the frozen service/allocator contracts. |
| W6 | `delivery/manifests/w6.json` | User, desktop, and device service implementation, consuming the frozen placement and service contracts. |
| W7 | `delivery/manifests/w7.json` | Declarative Nix, process, and resource emission against the frozen allocator API; extension of the existing provider registry only. |

`shared-contracts.json` is machine-enforced root authority. It protects the
cross-wave protobuf/generated service contracts, allocator model, workspace
dependency table and lock, delivery tooling, foreign wave manifests, and the
positive W5/W6/W7/frozen implementation-prefix partition. Unowned paths and
prefix-root symlink or gitlink changes fail closed; documentation exceptions
are explicit. Run the parent copy of the tooling, not the candidate copy:

```console
make -C "$TRUSTED_PARENT_ROOT" wave-policy-check \
  CANDIDATE_ROOT="$WAVE_WORKTREE"
```

The trusted checker requires its clean source worktree to equal the exact Git
Town parent commit corroborated by the candidate's unique open ordinary GitHub
PR in the policy-pinned repository. Branch, wave, base, head, and every ancestor
edge back to the shared root are derived and corroborated; caller-selected
wave/base options do not exist, partial linearization is rejected, and a
self/`HEAD` base fails. Git object reads and diffs disable replacements, grafts,
and shallow traversal, and any repository carrying the corresponding metadata
is rejected. A new shared DTO, dependency, generated contract, or policy
requirement returns to
`adr0045-post-w4-contracts`; it is not added on a wave branch.
