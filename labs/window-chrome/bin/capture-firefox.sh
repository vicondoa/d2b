#!/usr/bin/env bash
# Capture a real application — Firefox — through the proxy inside niri.
#
# Firefox draws its own client-side decorations, so this is also the test for
# whether the trusted tab reads as chrome or as a second titlebar stacked on
# the application's own.
set -uo pipefail
source "$(dirname "${BASH_SOURCE[0]}")/lab-common.sh"
trap 'lab_cleanup' EXIT
: "${LAB_OUT:?}"

PROFILE="$(mktemp -d "${TMPDIR:-/tmp}/chromelab-ff.XXXXXX")"
cleanup_profile() { rm -rf "$PROFILE"; }
trap 'lab_cleanup; cleanup_profile' EXIT

capture() {
  local name="$1" x="$2" y="$3" w="$4" h="$5"
  grim -g "${x},${y} ${w}x${h}" "$LAB_OUT/$name.png" 2>>"$LAB_OUT/grim.log" \
    || lab_die "grim failed for $name"
  lab_check_image "$LAB_OUT/$name.png"
  lab_log "captured $name"
}

# A local page, so nothing touches the network and nothing personal appears.
cat > "$PROFILE/page.html" <<'HTML'
<!doctype html><html><head><meta charset="utf-8"><title>Work browser</title>
<style>
 body{margin:0;font:16px system-ui,sans-serif;background:#fbfbfd;color:#1c1c22}
 header{background:#25272b;color:#f2f2f7;padding:18px 26px;font-weight:600}
 main{padding:26px}
 .card{background:#fff;border:1px solid #e2e2ea;border-radius:8px;
       padding:18px 20px;margin-bottom:14px;max-width:620px}
 h2{margin:0 0 6px;font-size:15px}
 p{margin:0;color:#55555f;font-size:14px;line-height:1.5}
</style></head><body>
<header>Internal tooling</header>
<main>
  <div class="card"><h2>Deployment status</h2>
    <p>All services nominal. Last check completed two minutes ago.</p></div>
  <div class="card"><h2>Open requests</h2>
    <p>Three items awaiting review in the queue.</p></div>
  <div class="card"><h2>Notes</h2>
    <p>This page is local to the capture and contains no real data.</p></div>
</main></body></html>
HTML

cat > "$PROFILE/user.js" <<'JS'
user_pref("browser.shell.checkDefaultBrowser", false);
user_pref("browser.startup.homepage_override.mstone", "ignore");
user_pref("datareporting.policy.dataSubmissionEnabled", false);
user_pref("browser.aboutwelcome.enabled", false);
user_pref("browser.startup.firstrunSkipsHomepage", true);
user_pref("toolkit.telemetry.reportingpolicy.firstRun", false);
JS

display="$(lab_start_proxy work \
  --vm-name work --border-enable --border-label "Work" \
  --border-color-active "#ffb347" --border-color-inactive "#ffb347")" \
  || lab_die "proxy failed"

MOZ_ENABLE_WAYLAND=1 WAYLAND_DISPLAY="$display" \
  firefox --profile "$PROFILE" --no-remote --new-instance \
  "file://$PROFILE/page.html" \
  >"$LAB_OUT/guest-firefox.log" 2>&1 &
LAB_GUEST_PIDS+=("$!")

lab_log "waiting for firefox to map (this is slower than a terminal)"
id="$(lab_wait_window "Work browser" 240)" || {
  id="$(lab_wait_window "Mozilla Firefox" 60)" || {
    lab_log "firefox never mapped"
    tail -30 "$LAB_OUT/guest-firefox.log" >&2
    exit 1
  }
}
lab_log "firefox window id=$id"
lab_place_window "$id" 150 120 900 560
sleep 3.5

capture "ff-collapsed" 100 70 1000 660
capture "ff-collapsed-detail" 130 100 520 100

echo FIREFOX_OK > "$LAB_OUT/firefox.status"
niri msg action quit --skip-confirmation >/dev/null 2>&1
