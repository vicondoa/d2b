# Cycle

- [ ] T001 [owner: alpha] [files: alpha/one.rs] [depends: T002] First.
- [ ] T002 [owner: beta] [files: beta/two.rs] [depends: T001] Second.

## Dependency graph

```text
T001 <- T002
T002 <- T001
```
