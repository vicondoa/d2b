# Positive

- [ ] T001 [owner: alpha] [files: alpha/one.rs] [depends: none] First.
- [ ] T002 [owner: beta] [files: beta/two.rs] [depends: T001] Second.

## Dependency graph

```text
T001 <- none
T002 <- T001
```
