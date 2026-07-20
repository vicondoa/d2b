# Configure generic unsafe-local launcher items

**Diataxis category:** how-to.

Use unsafe-local only for software you already trust to run as your host user.
It is useful for presenting configured host applications and persistent host
shells through the same realm/workload UI as other providers.

1. Opt the realm in and identify eligible users:

   ```nix
   d2b.realms.host = {
     allowedUsers = [ "alice" ];
     policy.allowUnsafeLocal = true;
   };
   ```

2. Declare a workload and generic launcher items:

   ```nix
   d2b.realms.host.workloads.tools = {
     kind = "unsafe-local";

     shell = {
       enable = true;
       defaultName = "host";
       maxSessions = 8;
     };

     launcher = {
       enable = true;
       label = "Local tools";
       defaultItem = "browser";

       items = {
         browser = {
           type = "exec";
           name = "Firefox";
           icon.name = "firefox";
           argv = [ "firefox" ];
           graphical = true;
         };

         observability = {
           type = "exec";
           name = "OpenObserve";
           icon.name = "monitoring";
           argv = [ "firefox" "https://observe.example.test/" ];
           graphical = true;
         };

         terminal = {
           type = "shell";
           name = "Terminal";
           icon.name = "terminal";
         };
       };
     };
   };
   ```

   The item name and icon describe the action; they are not derived from
   `argv[0]`. Firefox and OpenObserve are therefore two ordinary exec items,
   even though both execute Firefox.

3. Rebuild after the runtime features are available. Then launch a configured
   item with:

   ```console
   d2b launch tools.host.d2b --item observability
   ```

   Omitting `--item` uses `defaultItem`, or the only item when exactly one is
   declared. Multiple items without a default produce a list of valid choices.

Do not put secrets in argv. Configured argv is private daemon bundle data, but
Nix derivations are not a secret store. Graphical items require the d2b Wayland
proxy and fail visibly if it is unavailable; they never fall back to the direct
compositor.

The per-user runtime accepts only an authenticated
`d2b.runtime.systemd-user.v2` ComponentSession for the requesting uid. If the
runtime agent is unavailable, repair the user's systemd service or login
session. Do not work around it with a host shell, direct compositor command, or
an older helper socket.
