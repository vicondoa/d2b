# Trusted site-runtime contract (`site.json`).
#
# The single source the daemon may use for host session facts it must not
# guess: today the host Wayland socket the GPU worker renders into. It is
# resolved from the same options the site's session wiring uses
# (`d2b.site.waylandUser` + `d2b.site.waylandDisplay`), so the trusted bundle
# cannot name a runtime directory the site does not create. A site without a
# Wayland session emits `waylandSocket = null`; consumers refuse by name
# instead of inventing a path.
{ config, ... }:

let
  cfg = config.d2b;
  waylandUid =
    if cfg.site.waylandUser == null
    then null
    else config.users.users.${cfg.site.waylandUser}.uid or null;
in
{
  config.d2b._bundle.siteJson = {
    data = {
      schemaVersion = "v1";
      waylandSocket =
        if waylandUid == null
        then null
        else "/run/user/${toString waylandUid}/${cfg.site.waylandDisplay}";
    };
    installFileName = "site.json";
    classification = "contractPrivateNonSecret";
    sensitivity = "nonSecret";
  };
}
