### Fixed

- ProcessTemplateBinding validation no longer allocates a String per construction when checking the `/bin/<binary>` path suffix.
- The zone session codec state machines (request deadlines, fragment reassembly, receive/send nonce sequences, attachment credits) and header canonical round-trips now have unit tests.
- EmergencyPolicySpec now has tests for deadline/reason/control-character rejection, effective scope with no enabled policy, and the serde default round-trip.