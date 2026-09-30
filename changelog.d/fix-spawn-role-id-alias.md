### Fixed

- The `spawn-process` launch-identity fence compared the payload's wire
  `roleId` against the intent's raw `role_id`. The cloud-hypervisor runner
  is the one role that travels under a daemon-side alias, so the fence
  refused every legitimate nested-VMM launch and the guest's Cloud
  Hypervisor API socket never appeared. The alias is now evaluated once,
  by `ResolvedRunnerIntent::wire_role_id`, and the broker, the process
  provider and the supervisor all read it from there instead of each
  keeping their own copy of the rule.
- The spawn test fixture built payloads carrying the raw `role_id`, which
  no launch client ever sends, so the fence could disagree with its client
  and still pass every test. The fixture now sends the wire `roleId`.
