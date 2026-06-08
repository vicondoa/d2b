#!/usr/bin/env sh
# Shared state-dir traversal ACL helper for nixlingStateDirAcl.
# Caller must set STATE_DIR and LAUNCHER_GROUP. SETFACL_BIN may
# override the setfacl binary path used by NixOS activation.
: "${STATE_DIR:?STATE_DIR must be set}"
: "${LAUNCHER_GROUP:?LAUNCHER_GROUP must be set}"
SETFACL_BIN=${SETFACL_BIN:-setfacl}
if [ -d "$STATE_DIR" ]; then
  "$SETFACL_BIN" -m "g:$LAUNCHER_GROUP:--x" "$STATE_DIR" 2>/dev/null || true
fi
