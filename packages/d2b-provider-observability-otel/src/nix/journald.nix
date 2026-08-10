{ lib, zoneId, executionId, enable ? false }:

let
  journalMarker = "true";
  journalProcessors = {
    "filter/journald" = {
      # Journald entries must carry a string MESSAGE. Dropping malformed
      # marked records keeps the redaction transform fail-closed without
      # affecting application logs on the shared logs pipeline.
      error_mode = "propagate";
      logs.log_record = [
        "attributes[\"d2b.journald\"] == \"true\" and (body[\"MESSAGE\"] == nil or not IsString(body[\"MESSAGE\"]))"
      ];
    };

    "transform/journald" = {
      # The receiver places journal fields in the log body. Keep the same
      # metadata absent from attributes too, since OTLP logs share this
      # pipeline.
      error_mode = "propagate";
      log_statements = [
        "delete_key(log.body, \"_CMDLINE\") where log.attributes[\"d2b.journald\"] == \"true\" and IsMap(log.body)"
        "delete_key(log.body, \"_EXE\") where log.attributes[\"d2b.journald\"] == \"true\" and IsMap(log.body)"
        "delete_key(log.body, \"INVOCATION_ID\") where log.attributes[\"d2b.journald\"] == \"true\" and IsMap(log.body)"
        "replace_pattern(log.body[\"MESSAGE\"], \"(?i)(credential|secret|token|password)[^\\n]*\", \"[REDACTED]\") where log.attributes[\"d2b.journald\"] == \"true\" and IsMap(log.body) and IsString(log.body[\"MESSAGE\"])"
        "replace_pattern(log.body[\"MESSAGE\"], \"(?i)(/run/|/var/)[^[:space:]]+\", \"[REDACTED_PATH]\") where log.attributes[\"d2b.journald\"] == \"true\" and IsMap(log.body) and IsString(log.body[\"MESSAGE\"])"
      ];
    };

    "attributes/journald" = {
      include = {
        match_type = "strict";
        attributes = [
          { key = "d2b.journald"; value = journalMarker; }
        ];
      };
      actions = [
        { key = "_CMDLINE"; action = "delete"; }
        { key = "_EXE"; action = "delete"; }
        { key = "INVOCATION_ID"; action = "delete"; }
        { key = "d2b.journald"; action = "delete"; }
      ];
    };
  };
in
{
  # Keep both top-level namespaces present when the feature is disabled. The
  # guest component merges these namespaces before applying its own toggle.
  receivers = lib.optionalAttrs enable {
    journald = {
      start_at = "end";
      units = [
        "z-${zoneId}/*"
        "s-${executionId}/*"
      ];
      attributes."d2b.journald" = journalMarker;
    };
  };
  processors = lib.optionalAttrs enable journalProcessors;
}
