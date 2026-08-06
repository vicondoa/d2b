# Malformed ID and header

- [ ] T01 [owner: alpha] [files: alpha/one.rs] [depends: none] Short ID.
-  [ ] T002 [owner: beta] [files: beta/two.rs] [depends: none] Extra marker space.
- [ ]T003 [owner: gamma] [files: gamma/three.rs] [depends: none] Missing post-marker space.

## Dependency graph

```text
T001 <- none
T002 <- none
T003 <- none
```
