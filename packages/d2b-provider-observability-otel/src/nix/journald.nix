{ lib, zoneId, executionId, enable ? false }:

lib.optionalAttrs enable {
  receivers.journald = {
    start_at = "end";
    include_units = [
      "z-${zoneId}/*"
      "s-${executionId}/*"
    ];
  };

  processors.redact_journald = {
    drop_fields = [
      "MESSAGE"
      "_CMDLINE"
      "_EXE"
      "INVOCATION_ID"
    ];
    drop_message_patterns = [
      "(?i)credential"
      "(?i)secret"
      "(?i)token"
      "(?i)password"
      "(?i)/run/"
      "(?i)/var/"
    ];
  };
}
