# Actual task omitted from census

<!-- D2B-SPEC003-PLAN-TASK-CENSUS:BEGIN -->
T001
<!-- D2B-SPEC003-PLAN-TASK-CENSUS:END -->

- [ ] T001 [owner: alpha] [files: alpha/one.rs] [depends: none] Declared.
- [ ] T002 [owner: beta] [files: beta/two.rs] [depends: T001] Omitted from census.

## Dependency graph

```text
T001 <- none
T002 <- T001
```
