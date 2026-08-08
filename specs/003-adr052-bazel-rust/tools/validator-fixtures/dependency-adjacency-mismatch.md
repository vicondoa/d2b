# Dependency and adjacency mismatch

<!-- D2B-SPEC003-PLAN-TASK-CENSUS:BEGIN -->
T001
T002
<!-- D2B-SPEC003-PLAN-TASK-CENSUS:END -->

- [ ] T001 [owner: alpha] [files: alpha/one.rs] [depends: T002] First.
- [ ] T002 [owner: beta] [files: beta/two.rs] [depends: none] Second.

## Dependency graph

```text
T001 <- T002
T002 <- none
```
