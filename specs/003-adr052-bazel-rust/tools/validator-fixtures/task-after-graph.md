# Task after graph

<!-- D2B-SPEC003-PLAN-TASK-CENSUS:BEGIN -->
T001
T002
<!-- D2B-SPEC003-PLAN-TASK-CENSUS:END -->

- [ ] T001 [owner: alpha] [files: alpha/one.rs] [depends: none] First.

## Dependency graph

```text
T001 <- none
```

- [ ] T002 [owner: beta] [files: beta/two.rs] [depends: T001] Hidden after graph.
