# Concurrent conflict

<!-- D2B-SPEC003-PLAN-TASK-CENSUS:BEGIN -->
T001
T002
<!-- D2B-SPEC003-PLAN-TASK-CENSUS:END -->

- [ ] T001 [owner: alpha] [files: shared/one.rs] [depends: none] First.
- [ ] T002 [owner: beta] [files: shared/one.rs] [depends: none] Second.

## Dependency graph

```text
T001 <- none
T002 <- none
```
