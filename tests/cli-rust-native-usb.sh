#!/usr/bin/env bash
set -euo pipefail

HERE=$(dirname "$(readlink -f "$0")")
ROOT=${ROOT:-$(dirname "$HERE")}
# shellcheck source=cli-rust-native-common.sh
. "$HERE/cli-rust-native-common.sh"

log "==> tests/cli-rust-native-usb.sh"
scratch=$(nl_mktemp .cli-rust-native-usb.XXXXXX)
bundle_root=$(nl_cli_smoke_bundle_tree)
cli=$(nl_cli_native_bin)

"$cli" usb --help > "$scratch/usb-help.txt"
NIXLING_MANIFEST_PATH="$bundle_root/vms.json" \
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
  "$cli" usb attach corp-vm 1-2 --dry-run > "$scratch/usb-attach-dry-run.txt"
NIXLING_MANIFEST_PATH="$bundle_root/vms.json" \
NIXLING_BUNDLE_PATH="$bundle_root/bundle.json" \
  "$cli" usb detach corp-vm 1-2 --dry-run > "$scratch/usb-detach-dry-run.txt"

diff -u "$ROOT/tests/golden/cli-output/usb-help.txt" "$scratch/usb-help.txt"
diff -u "$ROOT/tests/golden/cli-output/usb-attach-dry-run.txt" "$scratch/usb-attach-dry-run.txt"
diff -u "$ROOT/tests/golden/cli-output/usb-detach-dry-run.txt" "$scratch/usb-detach-dry-run.txt"

ok "usb --help and usb {attach,detach} --dry-run remain stable"
