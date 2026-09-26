### Fixed

- The daemon's operator status admission now refuses a submitted `Host`
  status that names `isolationPosture` or `isolationPostureMessage`.
  `HostReconciler::reject_operator_status_fields` was the documented
  enforcement half of the `ADR-046-telemetry-audit-and-support` no-suppression
  obligation ("operators can neither suppress nor override the user-only Host
  posture") but had no caller outside its own crate's tests, so a submission
  naming either field reached the layers below un-remarked. The refusal is
  typed (`resource-runtime-host-status-field-not-owned`) and covers the
  explicit `null` form, which is as much a suppression attempt as `"none"`.
