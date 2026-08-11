Inventory the build, review, and comment-resolution artifacts named by the
workflow metadata.

Confirm that each path is inside the assigned managed workspace and that the
inputs are present before dispatching judgment.  Record paths and concise
status only; do not copy prompt text, credentials, or unbounded command output
into the artifact.

This stage is deterministic control work and runs through `gc.run-operator`.
