# Type-G runNixOSTest: the Gas City contributor module under real systemd.
#
# The module is the production module under test.  Only the long-running
# provider commands are replaced with local deterministic doubles: the
# identities, credentials, slices, mounts, lifecycle edges, resource
# declarations, and filesystem/network hardening all come from the named
# module.
{ pkgs, self }:

let
  inherit (pkgs) lib;
  contributor = self.packages.${pkgs.system}.gas-city-contributor;
  contributorPython = "${contributor}/bin/python3";
  contributorScripts = "${contributor}/share/gas-city-contributor/pack/scripts";
  generation =
    builtins.substring 0 32 (builtins.hashString "sha256" (toString contributor));
  relayAuth = builtins.hashString "sha256" "gascity-fdproxy:acme/project:d2b";
  fixtureSource = ../fixtures/gas-city;

  credentialProbe = pkgs.writeShellScript "gascity-host-credential-probe" ''
    set -eu
    role="$1"
    expected="$2"
    root=/run/gascity-contributor/test
    install -d -m 0770 "$root"
    marker="$root/credentials-$role"
    credential_dir="''${CREDENTIALS_DIRECTORY-}"
    {
      printf 'uid=%s\n' "$(id -u)"
      if test -n "$credential_dir"; then
        printf 'credential-directory=present\n'
      else
        printf 'credential-directory=missing\n'
      fi
      for name in copilot-token discord-bot-token github-app-private-key buildbuddy-api-key; do
        if test -n "$credential_dir" && test -r "$credential_dir/$name"; then
          printf '%s=visible\n' "$name"
        else
          printf '%s=hidden\n' "$name"
        fi
      done
      for source in \
        /etc/gascity-test/copilot-token \
        /etc/gascity-test/discord-token \
        /etc/gascity-test/github-key \
        /etc/gascity-test/buildbuddy-key; do
        if test -r "$source"; then
          printf '%s=readable\n' "$source"
        else
          printf '%s=denied\n' "$source"
        fi
      done
      if test -r /nix/var/nix/daemon-socket/socket; then
        printf 'nix-daemon=readable\n'
      else
        printf 'nix-daemon=hidden\n'
      fi
      printf 'expected=%s\n' "$expected"
    } > "$marker"
    chmod 0640 "$marker"
  '';

  fakeSidecar = pkgs.writeTextFile {
    name = "gascity-host-fake-sidecar";
    executable = true;
    text = ''
      #!${contributorPython}
      import grp
      import json
      import os
      import pathlib
      import signal
      import socket
      import sys
      import time

      path = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else None
      group = sys.argv[2] if len(sys.argv) > 2 else None
      response_kind = sys.argv[3] if len(sys.argv) > 3 else None
      stopping = False

      def stop(_signum, _frame):
          global stopping
          stopping = True

      signal.signal(signal.SIGTERM, stop)
      signal.signal(signal.SIGINT, stop)
      listener = None
      if path is not None:
          path.parent.mkdir(mode=0o770, parents=True, exist_ok=True)
          path.unlink(missing_ok=True)
          listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
          listener.bind(str(path))
          os.chmod(path, 0o660)
          if group is not None:
              os.chown(path, -1, grp.getgrnam(group).gr_gid)
          listener.listen(8)
          listener.settimeout(0.2)
      try:
          while not stopping:
              if listener is None:
                  time.sleep(0.2)
                  continue
              try:
                  connection, _address = listener.accept()
              except socket.timeout:
                  continue
              try:
                  if response_kind == "discord":
                      connection.recv(8192)
                      connection.sendall(
                          json.dumps(
                              {
                                  "protocol": "gascity-decision/1",
                                  "ok": True,
                                  "result": [],
                              },
                              separators=(",", ":"),
                          ).encode("ascii")
                          + b"\n"
                      )
              finally:
                  connection.close()
      finally:
          if listener is not None:
              listener.close()
          if path is not None:
              path.unlink(missing_ok=True)
    '';
  };

  fakeAcp = pkgs.writeTextFile {
    name = "gascity-host-fake-acp";
    executable = true;
    text = ''
      #!${contributorPython}
      import json
      import os
      import pathlib
      import signal
      import sys
      import time

      fixture = pathlib.Path("/var/lib/gascity-contributor/state/fixture")
      fixture.mkdir(mode=0o770, parents=True, exist_ok=True)
      stopping = False

      def append(name, value):
          with (fixture / name).open("a", encoding="utf-8") as stream:
              stream.write(str(value) + "\n")

      def stop(_signum, _frame):
          global stopping
          stopping = True

      signal.signal(signal.SIGTERM, stop)
      signal.signal(signal.SIGINT, stop)

      try:
          home = pathlib.Path(os.environ["COPILOT_HOME"])
          settings = json.loads((home / "settings.json").read_text(encoding="utf-8"))
          if set(settings) != {"model", "contextTier"}:
              raise RuntimeError("profile settings are not the only ACP authority")
          append("acp-settings", json.dumps(settings, sort_keys=True))
          append("acp-token-projection", "present" if os.environ.get("COPILOT_GITHUB_TOKEN") else "missing")
          append("acp-home-settings", "present" if (home / "settings.json").is_file() else "missing")
          append("acp-home-token", "present" if list(home.glob("*token*")) else "absent")
          append("acp-net-namespace", os.readlink("/proc/self/ns/net"))
          for source in (
              "/etc/gascity-test/discord-token",
              "/etc/gascity-test/github-key",
              "/etc/gascity-test/buildbuddy-key",
          ):
              try:
                  pathlib.Path(source).read_text(encoding="utf-8")
              except OSError:
                  append("acp-" + pathlib.Path(source).stem, "denied")
              else:
                  append("acp-" + pathlib.Path(source).stem, "readable")
          worktree = pathlib.Path.cwd()
          with (worktree / "fixture-progress.txt").open("a", encoding="utf-8") as stream:
              stream.write("progress\n")
          pid_path = fixture / "acp-current.pid"
          pid_path.write_text(str(os.getpid()) + "\n", encoding="utf-8")
          count_path = fixture / "acp-launch-count"
          count = int(count_path.read_text(encoding="utf-8")) if count_path.exists() else 0
          count_path.write_text(str(count + 1) + "\n", encoding="utf-8")
          for raw in sys.stdin:
              if stopping:
                  break
              if not raw.strip():
                  continue
              try:
                  request = json.loads(raw)
                  response = {
                      "jsonrpc": "2.0",
                      "id": request.get("id") if isinstance(request, dict) else None,
                      "result": {"fixture": True},
                  }
                  sys.stdout.write(json.dumps(response, separators=(",", ":")) + "\n")
                  sys.stdout.flush()
              except (ValueError, AttributeError):
                  continue
          while not stopping:
              time.sleep(0.05)
      except (OSError, KeyError, ValueError, RuntimeError) as error:
          append("acp-error", str(error))
          raise SystemExit(1)
    '';
  };

  fakeEgress = pkgs.writeTextFile {
    name = "gascity-host-fake-egress";
    executable = true;
    text = ''
      #!${contributorPython}
      import array
      import grp
      import json
      import os
      import pathlib
      import signal
      import socket
      import sys
      import threading

      SOCKET = pathlib.Path("/run/gascity-contributor/egress.sock")
      MARKERS = pathlib.Path("/run/gascity-contributor/test")
      ALLOWED_UIDS = {45101, 45102, 45103, 45105, 45106}
      ALLOWED_HOSTS = {
          "api.github.com",
          "api.githubcopilot.com",
          "copilot-proxy.githubusercontent.com",
          "discord.com",
          "gateway.discord.gg",
          "github.com",
          "remote.buildbuddy.io",
      }
      AUTH = os.environ.get("GC_FDPROXY_AUTH", "")
      stopping = False

      def record(name, value):
          MARKERS.mkdir(mode=0o770, parents=True, exist_ok=True)
          with (MARKERS / name).open("a", encoding="utf-8") as stream:
              stream.write(str(value) + "\n")

      def stop(_signum, _frame):
          global stopping
          stopping = True

      def peer_uid(connection):
          raw = connection.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)
          return int.from_bytes(raw[4:8], byteorder=sys.byteorder, signed=True)

      def serve_fixture(connection, host):
          try:
              request = connection.recv(65536)
              if request:
                  body = ("fixture egress " + host).encode("ascii")
                  connection.sendall(
                      b"HTTP/1.1 200 OK\r\nContent-Length: "
                      + str(len(body)).encode("ascii")
                      + b"\r\nConnection: close\r\n\r\n"
                      + body
                  )
          except OSError:
              pass
          finally:
              connection.close()

      def send_response(connection, value, descriptor=None):
          payload = json.dumps(value, separators=(",", ":")).encode("ascii") + b"\n"
          if descriptor is None:
              connection.sendall(payload)
              return
          descriptors = array.array("i", [descriptor])
          connection.sendmsg(
              [payload],
              [(socket.SOL_SOCKET, socket.SCM_RIGHTS, descriptors.tobytes())],
          )

      def serve_one(connection):
          try:
              uid = peer_uid(connection)
              if uid not in ALLOWED_UIDS:
                  record("egress-denied-peer", uid)
                  return
              record("egress-peer", uid)
              while not stopping:
                  raw = bytearray()
                  while b"\n" not in raw:
                      chunk = connection.recv(8192)
                      if not chunk:
                          return
                      raw.extend(chunk)
                      if len(raw) > 8192:
                          return
                  line, remainder = bytes(raw).split(b"\n", 1)
                  if remainder:
                      return
                  try:
                      request = json.loads(line)
                  except ValueError:
                      return
                  valid = (
                      isinstance(request, dict)
                      and request.get("version") == "fdproxy/1"
                      and request.get("operation") == "connect"
                      and request.get("auth") == AUTH
                      and isinstance(request.get("request_id"), str)
                      and isinstance(request.get("host"), str)
                      and type(request.get("port")) is int
                  )
                  if not valid:
                      return
                  host = request["host"]
                  port = request["port"]
                  request_id = request["request_id"]
                  if host not in ALLOWED_HOSTS or port != 443:
                      record("egress-denied", host + ":" + str(port))
                      send_response(
                          connection,
                          {"version": "fdproxy/1", "request_id": request_id, "ok": False},
                      )
                      continue
                  left, right = socket.socketpair()
                  record("egress-allow", host + ":" + str(port))
                  threading.Thread(
                      target=serve_fixture,
                      args=(right, host),
                      daemon=True,
                  ).start()
                  try:
                      send_response(
                          connection,
                          {"version": "fdproxy/1", "request_id": request_id, "ok": True},
                          left.fileno(),
                      )
                  finally:
                      left.close()
          except OSError:
              pass
          finally:
              connection.close()

      signal.signal(signal.SIGTERM, stop)
      signal.signal(signal.SIGINT, stop)
      SOCKET.parent.mkdir(mode=0o770, parents=True, exist_ok=True)
      if SOCKET.exists():
          SOCKET.unlink()
      listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
      listener.bind(str(SOCKET))
      os.chmod(SOCKET, 0o660)
      os.chown(SOCKET, -1, grp.getgrnam("gascity-egress-channel").gr_gid)
      listener.listen(32)
      listener.settimeout(0.2)
      try:
          while not stopping:
              try:
                  connection, _address = listener.accept()
              except socket.timeout:
                  continue
              threading.Thread(target=serve_one, args=(connection,), daemon=True).start()
      finally:
          listener.close()
          SOCKET.unlink(missing_ok=True)
  '';
  };

  fakeAgent = pkgs.writeTextFile {
    name = "gascity-host-fake-agent";
    executable = true;
    text = ''
      #!${contributorPython}
      import json
      import os
      import pathlib
      import signal
      import socket
      import subprocess
      import sys
      import time
      import grp

      PACKAGE = "${contributor}";
      PYTHON = "${contributorPython}";
      LAUNCHER = "${contributorScripts}/agent-launcher.py";
      # launcher limits: --max-agents 1 --max-active-runs 1
      SANDBOX = "${contributorScripts}/agent-sandbox.py";
      FDPROXY = "${contributorScripts}/fdproxy.py";
      BWRAP = "${contributor}/bin/bwrap";
      SETTINGS = "${contributor}/share/gas-city-contributor/copilot";
      FAKE_ACP = "${fakeAcp}";
      RUNTIME = pathlib.Path("/run/gascity-contributor");
      FIXTURE = pathlib.Path("/var/lib/gascity-contributor/state/fixture");
      WORKTREE = pathlib.Path("/var/lib/gascity-contributor/state/worktrees/test-run");
      LEASE_ROOT = pathlib.Path("/var/lib/gascity-contributor/state/leases");
      AGENT_RUNTIME = pathlib.Path("/run/gascity-contributor/agent");
      PRIVATE_SOCKET = pathlib.Path("/run/gascity-agent/agent.sock");
      PUBLIC_SOCKET = RUNTIME / "agent.sock";
      READINESS = RUNTIME / "readiness.json";
      GENERATION = os.environ["GC_FIXTURE_GENERATION"];
      stopping = False

      def stop(_signum, _frame):
          global stopping
          stopping = True

      def write_readiness():
          READINESS.write_text(
              json.dumps(
                  {
                      "generation": GENERATION,
                      "state_schema": "1",
                      "ready": True,
                      "effective_profiles": {
                          "coding": "code-luna",
                          "review": "review-sol",
                      },
                      "error_code": None,
                  },
                  sort_keys=True,
                  separators=(",", ":"),
              )
              + "\n",
              encoding="utf-8",
          )
          os.chmod(READINESS, 0o640)

      def append(name, value):
          FIXTURE.mkdir(mode=0o770, parents=True, exist_ok=True)
          with (FIXTURE / name).open("a", encoding="utf-8") as stream:
              stream.write(str(value) + "\n")

      def read_line(connection):
          data = bytearray()
          while b"\n" not in data:
              chunk = connection.recv(4096)
              if not chunk:
                  return b""
              data.extend(chunk)
          return bytes(data).split(b"\n", 1)[0]

      def serve_public(listener):
          while not stopping:
              try:
                  client, _address = listener.accept()
              except socket.timeout:
                  continue
              try:
                  raw = client.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)
                  uid = int.from_bytes(raw[4:8], byteorder=sys.byteorder, signed=True)
                  if uid == 45100:
                      client.sendall(b"fixture-agent/1\n")
              except OSError:
                  pass
              finally:
                  client.close()

      signal.signal(signal.SIGTERM, stop)
      signal.signal(signal.SIGINT, stop)
      token_path = pathlib.Path(os.environ["CREDENTIALS_DIRECTORY"]) / "copilot-token"
      token = token_path.read_text(encoding="utf-8").strip()
      if not token:
          raise SystemExit("fixture Copilot credential is empty")
      FIXTURE.mkdir(mode=0o770, parents=True, exist_ok=True)
      WORKTREE.mkdir(mode=0o770, parents=True, exist_ok=True)
      os.chmod(WORKTREE, 0o700)
      for private_root in (LEASE_ROOT, AGENT_RUNTIME):
          private_root.mkdir(mode=0o700, parents=True, exist_ok=True)
          os.chmod(private_root, 0o700)
      context = WORKTREE / "durable-context.json"
      if not context.exists():
          context.write_text(
              json.dumps(
                  {
                      "run_id": "host-run",
                      "bead_id": "host-bead",
                      "generation": GENERATION,
                      "state_schema": "1",
                      "open_work": ["fixture-progress.txt"],
                      "summary": "durable fixture context",
                      "branch": "gascity/host-run",
                      "commits": [],
                      "worktree": str(WORKTREE),
                      "review_state": "working",
                      "retry_counters": {},
                      "next_action": "continue",
                  },
                  sort_keys=True,
              )
              + "\n",
              encoding="utf-8",
          )
      write_readiness()
      RUNTIME.mkdir(mode=0o770, parents=True, exist_ok=True)
      if PUBLIC_SOCKET.exists():
          PUBLIC_SOCKET.unlink()
      public = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
      public.bind(str(PUBLIC_SOCKET))
      os.chmod(PUBLIC_SOCKET, 0o660)
      os.chown(PUBLIC_SOCKET, -1, grp.getgrnam("gascity-agent-channel").gr_gid)
      public.listen(8)
      public.settimeout(0.2)
      public_thread = __import__("threading").Thread(
          target=serve_public, args=(public,), daemon=True
      )
      public_thread.start()
      launcher_env = dict(os.environ)
      launcher_env["COPILOT_GITHUB_TOKEN"] = token
      launcher = subprocess.Popen(
          [
              PYTHON,
              LAUNCHER,
              "--server",
              "--socket",
              str(PRIVATE_SOCKET),
              "--settings-root",
              SETTINGS,
              "--copilot",
              PYTHON,
              "--state-root",
              "/var/lib/gascity-contributor/state/agent-state",
              "--worktree",
              "/var/lib/gascity-contributor/state/worktrees",
              "--lease-root",
              "/var/lib/gascity-contributor/state/leases",
              "--runtime-root",
              "/run/gascity-contributor/agent",
              "--runtime-path",
              "${contributor}/bin",
              "--runtime-path",
              "${contributor}/share/gas-city-contributor",
              "--sandbox-script",
              SANDBOX,
              "--fdproxy-script",
              FDPROXY,
              "--sandbox-python",
              PYTHON,
              "--bwrap-path",
              BWRAP,
              "--max-agents",
              "1",
              "--max-active-runs",
              "1",
              "--client-uid",
              "45101",
              "--generation",
              GENERATION,
              "--state-schema",
              "1",
              "--allow-unsafe-fixture",
              "--fixture-child-script",
              FAKE_ACP,
              "--require-ready",
              "--readiness-status",
              str(READINESS),
          ],
          env=launcher_env,
          close_fds=True,
      )
      active = None
      try:
          while not stopping:
              if pathlib.Path("/run/gascity-contributor/test/cancel").exists():
                  if active is not None:
                      active.close()
                      active = None
                  append("cancelled", "yes")
                  while not stopping:
                      time.sleep(0.1)
                  break
              if active is None:
                  for _attempt in range(100):
                      if PRIVATE_SOCKET.exists():
                          break
                      if launcher.poll() is not None:
                          raise SystemExit("fixture launcher stopped")
                      time.sleep(0.05)
                  active = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                  active.settimeout(2.0)
                  active.connect(str(PRIVATE_SOCKET))
                  active.sendall(
                      (
                          json.dumps(
                              {
                                  "protocol": "gascity-agent/1",
                                  "operation": "launch",
                                  "profile": "code-luna",
                                  "tool_policy": "coding",
                                  "run_id": "host-run",
                                  "bead_id": "host-bead",
                                  "generation": GENERATION,
                                  "state_schema": "1",
                                  "worktree": str(WORKTREE),
                                  "state_root": "/var/lib/gascity-contributor/state/agent-state",
                                  "fds": [],
                                  "require_ready": True,
                              },
                              separators=(",", ":"),
                          )
                          + "\n"
                      ).encode("utf-8")
                  )
                  response = json.loads(read_line(active))
                  if response.get("ok") is not True:
                      append("launcher-error", response)
                      raise SystemExit("fixture launch was rejected")
                  active.settimeout(0.25)
              try:
                  data = active.recv(4096)
              except socket.timeout:
                  continue
              except OSError:
                  data = b""
              if not data:
                  active.close()
                  active = None
                  time.sleep(0.2)
      finally:
          if active is not None:
              active.close()
          public.close()
          PUBLIC_SOCKET.unlink(missing_ok=True)
          if launcher.poll() is None:
              launcher.terminate()
              try:
                  launcher.wait(timeout=3)
              except subprocess.TimeoutExpired:
                  launcher.kill()
                  launcher.wait()
  '';
  };

  fakeMain = pkgs.writeTextFile {
    name = "gascity-host-fake-main";
    executable = true;
    text = ''
      #!${contributorPython}
      import json
      import os
      import pathlib
      import select
      import signal
      import socket
      import time

      PACKAGE = pathlib.Path("${contributor}");
      RUNTIME = pathlib.Path("/run/gascity-contributor");
      STATE = pathlib.Path("/var/lib/gascity-contributor/state");
      FIXTURE = STATE / "fixture"
      MARKERS = RUNTIME / "test"
      stopping = False

      def attempt_read(path):
          try:
              pathlib.Path(path).read_text(encoding="utf-8")
          except (OSError, UnicodeError):
              return False
          return True

      def attempt_write(path):
          try:
              with pathlib.Path(path).open("a", encoding="utf-8") as stream:
                  stream.write("unexpected\n")
          except OSError:
              return False
          return True

      def stop(_signum, _frame):
          global stopping
          stopping = True

      signal.signal(signal.SIGTERM, stop)
      signal.signal(signal.SIGINT, stop)
      FIXTURE.mkdir(mode=0o770, parents=True, exist_ok=True)
      MARKERS.mkdir(mode=0o770, parents=True, exist_ok=True)
      count_path = FIXTURE / "main-runs"
      count = int(count_path.read_text(encoding="utf-8")) if count_path.exists() else 0
      count_path.write_text(str(count + 1) + "\n", encoding="utf-8")
      context = FIXTURE / "main-context.json"
      if not context.exists():
          context.write_text(
              json.dumps(
                  {
                      "generation": os.environ.get("GC_CITY_GENERATION", "${generation}"),
                      "state_schema": "1",
                      "worktree": str(FIXTURE),
                      "next_action": "continue",
                  },
                  sort_keys=True,
              )
              + "\n",
              encoding="utf-8",
          )

      listeners = []
      for port in (18372, 13307):
          listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
          listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
          listener.bind(("127.0.0.1", port))
          listener.listen(8)
          listener.setblocking(False)
          listeners.append(listener)

      boundary = {
          "projection-read": attempt_read("/run/gascity-contributor/host/host-canary"),
          "projection-write": attempt_write("/run/gascity-contributor/host/host-canary"),
          "unrelated-host-write": attempt_write("/etc/gascity-test/unrelated-host-write"),
          "root-canary-read": attempt_read("/root/gascity-test-secret"),
          "discord-source-read": attempt_read("/etc/gascity-test/discord-token"),
          "github-source-read": attempt_read("/etc/gascity-test/github-key"),
          "buildbuddy-source-read": attempt_read("/etc/gascity-test/buildbuddy-key"),
          "daemon-socket-read": attempt_read("/nix/var/nix/daemon-socket/socket"),
          "systemd-socket-read": attempt_read("/run/systemd/private"),
          "store-visible": pathlib.Path("/nix/store").is_dir(),
          "store-executable": os.access(str(PACKAGE / "bin/gc"), os.X_OK),
          "managed-city-link": (pathlib.Path("/var/lib/gascity-contributor/managed/city").is_symlink()),
          "managed-pack-link": (pathlib.Path("/var/lib/gascity-contributor/managed/pack").is_symlink()),
          "loopback-supervisor": True,
          "loopback-dolt": True,
      }
      (MARKERS / "main-boundary.json").write_text(
          json.dumps(boundary, sort_keys=True) + "\n", encoding="utf-8"
      )
      (MARKERS / "main-ready").write_text("ready\n", encoding="utf-8")
      try:
          while not stopping:
              if listeners:
                  readable, _writable, _errors = select.select(listeners, [], [], 0.2)
                  for listener in readable:
                      try:
                          client, _address = listener.accept()
                          client.close()
                      except OSError:
                          pass
              else:
                  time.sleep(0.2)
      finally:
          for listener in listeners:
              listener.close()
  '';
  };

  launcherProbe = pkgs.writeTextFile {
    name = "gascity-host-launcher-probe";
    executable = true;
    text = ''
      #!${contributorPython}
      import json
      import pathlib
      import socket
      import sys

      generation = sys.argv[1]
      run_id = sys.argv[2] if len(sys.argv) > 2 else "second-run"
      expected = sys.argv[3] if len(sys.argv) > 3 else ""
      worktree = pathlib.Path("/var/lib/gascity-contributor/state/worktrees") / run_id
      worktree.mkdir(mode=0o770, parents=True, exist_ok=True)
      connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
      connection.settimeout(3)
      connection.connect("/run/gascity-agent/agent.sock")
      request = {
          "protocol": "gascity-agent/1",
          "operation": "launch",
          "profile": "code-luna",
          "tool_policy": "coding",
          "run_id": run_id,
          "bead_id": "probe-bead",
          "generation": generation,
          "state_schema": "1",
          "worktree": str(worktree),
          "state_root": "/var/lib/gascity-contributor/state/agent-state",
          "fds": [],
          "require_ready": True,
      }
      connection.sendall((json.dumps(request, separators=(",", ":")) + "\n").encode("utf-8"))
      data = bytearray()
      while b"\n" not in data:
          chunk = connection.recv(4096)
          if not chunk:
              break
          data.extend(chunk)
      connection.close()
      response = json.loads(bytes(data).split(b"\n", 1)[0])
      print(json.dumps(response, sort_keys=True))
      if response.get("ok") is True:
          raise SystemExit("probe unexpectedly launched a second run")
      if expected and expected not in str(response.get("error", "")):
          raise SystemExit("probe failed for an unexpected reason")
  '';
  };

  proxyFixture = pkgs.writeTextFile {
    name = "gascity-host-fdproxy-fixture";
    executable = true;
    text = ''
      #!${contributorPython}
      import pathlib
      import socket
      import subprocess
      import sys
      import time

      proxy = "${contributorScripts}/fdproxy.py"
      channel = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
      channel.connect("/run/gascity-contributor/egress.sock")
      descriptor = channel.fileno()
      process = subprocess.Popen(
          [
              sys.executable,
              proxy,
              "--channel-fd",
              str(descriptor),
              "--listen",
              "127.0.0.1:18999",
          ],
          pass_fds=(descriptor,),
          close_fds=True,
      )
      try:
          client = None
          for _attempt in range(50):
              try:
                  client = socket.create_connection(("127.0.0.1", 18999), timeout=1)
                  break
              except OSError:
                  time.sleep(0.05)
          if client is None:
              raise SystemExit("fixture fdproxy did not listen")
          with client:
              client.sendall(
                  b"CONNECT api.github.com:443 HTTP/1.1\r\n"
                  b"Host: api.github.com\r\n\r\n"
              )
              response = client.recv(4096)
              if b"200 Connection Established" not in response:
                  raise SystemExit("allowlisted fixture egress was not accepted")
              client.sendall(b"GET /fixture HTTP/1.1\r\nHost: api.github.com\r\n\r\n")
              body = client.recv(4096)
              if b"fixture egress api.github.com" not in body:
                  raise SystemExit("fixture egress response was not relayed")
          denied = socket.create_connection(("127.0.0.1", 18999), timeout=1)
          with denied:
              denied.sendall(
                  b"CONNECT 169.254.169.254:80 HTTP/1.1\r\n"
                  b"Host: 169.254.169.254\r\n\r\n"
              )
              response = denied.recv(4096)
              if b"403 Forbidden" not in response:
                  raise SystemExit("private fixture egress was not denied")
      finally:
          channel.close()
          process.terminate()
          try:
              process.wait(timeout=3)
          except subprocess.TimeoutExpired:
              process.kill()
              process.wait()
  '';
  };

  quotaFixture = pkgs.writeTextFile {
    name = "gascity-host-quota-fixture";
    executable = true;
    text = ''
      #!${contributorPython}
      import importlib.util
      import pathlib

      path = pathlib.Path("${contributorScripts}/service-activation.py")
      spec = importlib.util.spec_from_file_location("activation", path)
      module = importlib.util.module_from_spec(spec)
      spec.loader.exec_module(module)
      module._mount_has_project_quota = lambda _path: False
      try:
          module.require_project_quota("/var/lib/gascity-contributor")
      except module.BoundaryError:
          print("project quota fail-closed")
      else:
          raise SystemExit("project quota fixture unexpectedly passed")
  '';
  };

  freeSpaceFixture = pkgs.writeTextFile {
    name = "gascity-host-free-space-fixture";
    executable = true;
    text = ''
      #!${contributorPython}
      import importlib.util
      import pathlib
      import types

      path = pathlib.Path("${contributorScripts}/service-activation.py")
      spec = importlib.util.spec_from_file_location("activation", path)
      module = importlib.util.module_from_spec(spec)
      spec.loader.exec_module(module)
      module.os.statvfs = lambda _path: types.SimpleNamespace(f_bavail=0, f_frsize=4096)
      try:
          module.check_free_space("/var/lib/gascity-contributor", 1)
      except module.BoundaryError:
          print("free-space reserve fail-closed")
      else:
          raise SystemExit("free-space fixture unexpectedly passed")
  '';
  };

  taskFixture = pkgs.writeTextFile {
    name = "gascity-host-task-limit-fixture";
    executable = true;
    text = ''
      #!${contributorPython}
      import subprocess

      children = []
      try:
          for _index in range(256):
              children.append(subprocess.Popen(["sleep", "30"]))
      except OSError:
          pass
      finally:
          for child in children:
              child.terminate()
          for child in children:
              child.wait()
      if len(children) >= 256:
          raise SystemExit("task fixture was not bounded by the contributor slice")
      print("task fixture bounded at", len(children))
  '';
  };
in
pkgs.testers.runNixOSTest {
  name = "gas-city-contributor";

  nodes.machine = { config, pkgs, ... }: {
    imports = [ self.nixosModules.gasCityContributor ];

    virtualisation = {
      memorySize = 4096;
      diskSize = 8192;
      cores = 2;
    };

    boot.kernelParams = [ "audit=0" ];
    system.stateVersion = "25.11";

    users.users = {
      alice = {
        isNormalUser = true;
        uid = 1000;
      };
      bob = {
        isNormalUser = true;
        uid = 1001;
      };
    };

    environment.etc."gascity-fixtures" = {
      source = fixtureSource;
    };

    systemd.tmpfiles.rules = [
      "d /var/lib/gascity-contributor/state/fixture 0770 gascity-agent gascity-contributor -"
      "d /run/gascity-contributor/test 0770 root gascity-contributor -"
    ];

    system.activationScripts.gasCityTestSources = {
      deps = [ "users" ];
      text = ''
        install -d -m 0755 /etc/gascity-test
        install -m 0600 /dev/null /etc/gascity-test/copilot-token
        install -m 0600 /dev/null /etc/gascity-test/discord-token
        install -m 0600 /dev/null /etc/gascity-test/github-key
        install -m 0600 /dev/null /etc/gascity-test/buildbuddy-key
        install -m 0644 /dev/null /etc/gascity-test/host-canary
        printf '%s\n' fixture-copilot-token > /etc/gascity-test/copilot-token
        printf '%s\n' fixture-discord-token > /etc/gascity-test/discord-token
        printf '%s\n' fixture-github-private-key > /etc/gascity-test/github-key
        printf '%s\n' fixture-buildbuddy-key > /etc/gascity-test/buildbuddy-key
        printf '%s\n' host-projection-canary > /etc/gascity-test/host-canary
      '';
    };

    system.activationScripts.gasCityRootCanary = {
      deps = [ "users" ];
      text = ''
        install -m 0600 /dev/null /root/gascity-test-secret
        printf '%s\n' root-secret > /root/gascity-test-secret
      '';
    };

    environment.systemPackages = [
      pkgs.iproute2
      pkgs.procps
      pkgs.jq
      contributor
    ];

    services.gasCityContributor = {
      enable = true;
      repository.githubSlug = "acme/project";
      repository.baseBranch = "main";
      repository.rigName = "d2b";
      operators.users = [ "alice" ];

      credentials = {
        copilotTokenFile = "/etc/gascity-test/copilot-token";
        discordBotTokenFile = "/etc/gascity-test/discord-token";
        githubPrivateKeyFile = "/etc/gascity-test/github-key";
        buildBuddyApiKeyFile = "/etc/gascity-test/buildbuddy-key";
      };

      github.appId = "7";
      github.installationId = "42";
      discord.applicationId = "11";
      discord.guildId = "111";
      discord.channelId = "222";
      discord.operatorUserIds = [ "333" ];

      hostReadOnlyPaths = [ "/etc/gascity-test/host-canary" ];
      network.allowedDomains = [ "fixture.example.com" ];

      resources = {
        cpuQuotaPercent = 37;
        memoryHighPercent = 18;
        memoryMaxPercent = 24;
        memorySwapMaxBytes = 0;
        tasksMax = 128;
        maxConcurrentAgents = 1;
        maxActiveRuns = 1;
        maxHeavyChecks = 1;
        nixMaxJobs = 1;
        nixBuildCores = 2;
      };

      storage = {
        totalQuotaBytes = 134217728;
        stateQuotaBytes = 33554432;
        cacheQuotaBytes = 16777216;
        publisherQuotaBytes = 8388608;
        discordQuotaBytes = 4194304;
        checkQuotaBytes = 8388608;
        minFreeBytes = 1048576;
      };

      ports = {
        supervisor = 18372;
        dolt = 13307;
      };

      check.enable = true;
      buildBuddy.enable = true;
    };

    systemd.services."gas-city-contributor".serviceConfig = {
      ExecStart = lib.mkForce fakeMain;
      ExecStartPre = lib.mkAfter [ "${credentialProbe} main none" ];
      Environment = lib.mkAfter [ "GC_PROJECT_QUOTA_SUPPORTED=1" ];
    };

    systemd.services.gascity-agent = {
      serviceConfig = {
        ExecStart = lib.mkForce fakeAgent;
        ExecStartPre = [ "${credentialProbe} agent copilot-token" ];
        Environment = lib.mkAfter [
          "GC_TEST_MODE=1"
          "GC_FIXTURE_GENERATION=${generation}"
        ];
      };
    };

    systemd.services.gascity-egress.serviceConfig = {
      ExecStartPre = [ "${credentialProbe} egress none" ];
      ExecStart = lib.mkForce fakeEgress;
    };
    systemd.services.gascity-discord.serviceConfig.ExecStartPre =
      [ "${credentialProbe} discord discord-bot-token" ];
    systemd.services.gascity-discord.serviceConfig.ExecStart =
      lib.mkForce
        "${fakeSidecar} /run/gascity-contributor/discord.sock gascity-discord-channel discord";
    systemd.services.gascity-publisher.serviceConfig.ExecStartPre =
      [ "${credentialProbe} publisher github-app-private-key" ];
    systemd.services.gascity-publisher.serviceConfig.ExecStart =
      lib.mkForce
        "${fakeSidecar} /run/gascity-contributor/publisher.sock gascity-publisher-channel";
    systemd.services.gascity-check.serviceConfig.ExecStartPre =
      [ "${credentialProbe} check none" ];
    systemd.services.gascity-buildbuddy-proxy.serviceConfig.ExecStartPre =
      [ "${credentialProbe} buildbuddy buildbuddy-api-key" ];
    systemd.services.gascity-buildbuddy-proxy.serviceConfig.ExecStart =
      lib.mkForce fakeSidecar;
  };

  testScript = ''
    import json

    package = "${contributor}"
    python = "${contributorPython}"
    generation = "${generation}"
    auth = "${relayAuth}"
    fake_agent = "${fakeAgent}"
    launcher_probe = "${launcherProbe}"
    proxy_fixture = "${proxyFixture}"

    start_all()

    # Copy the repository-relative fixture layout into a disposable tree.  The
    # fixture modules deliberately resolve their package scripts from that
    # layout, so this exercises the exact packaged scripts without importing
    # anything from the test runner's host checkout.
    machine.succeed(
        "rm -rf /tmp/gascity-fixtures && "
        "mkdir -p /tmp/gascity-fixtures/tests/fixtures/gas-city && "
        "cp -rL /etc/gascity-fixtures/. "
        "/tmp/gascity-fixtures/tests/fixtures/gas-city/ && "
        "mkdir -p /tmp/gascity-fixtures/nix /tmp/gascity-fixtures/tests/nix && "
        "ln -s ${contributor}/share/gas-city-contributor "
        "/tmp/gascity-fixtures/nix/gas-city-contributor && "
        "ln -s ../../nix/gas-city-contributor "
        "/tmp/gascity-fixtures/tests/nix/gas-city-contributor && "
        "ln -s ${contributor}/share/gas-city-contributor/copilot "
        "/tmp/gascity-fixtures/copilot"
    )

    for unit in [
        "gascity-egress.service",
        "gascity-agent.service",
        "gascity-discord.service",
        "gascity-publisher.service",
        "gascity-free-space-monitor.service",
        "gascity-check.service",
        "gascity-buildbuddy-proxy.service",
        "gas-city-contributor.service",
    ]:
        machine.wait_for_unit(unit)

    for path in [
        "/run/gascity-contributor/egress.sock",
        "/run/gascity-contributor/agent.sock",
        "/run/gascity-contributor/discord.sock",
        "/run/gascity-contributor/publisher.sock",
        "/run/gascity-contributor/readiness.json",
        "/run/gascity-contributor/test/main-ready",
        "/var/lib/gascity-contributor/state/fixture/acp-current.pid",
    ]:
        machine.wait_for_file(path)

    # Static identities and the single contributor slice are live, not merely
    # rendered option values.
    expected_uids = {
        "gascity": 45100,
        "gascity-agent": 45101,
        "gascity-discord": 45102,
        "gascity-publisher": 45103,
        "gascity-egress": 45104,
        "gascity-check": 45105,
        "gascity-buildbuddy-proxy": 45106,
    }
    for name, uid in expected_uids.items():
        machine.succeed(f"test \"$(id -u {name})\" = {uid}")
    for unit in [
        "gas-city-contributor.service",
        "gascity-agent.service",
        "gascity-discord.service",
        "gascity-publisher.service",
        "gascity-egress.service",
        "gascity-free-space-monitor.service",
        "gascity-check.service",
        "gascity-buildbuddy-proxy.service",
    ]:
        machine.succeed(
            f"test \"$(systemctl show -P Slice {unit})\" = gascity-contributor.slice"
        )
        machine.succeed(
            f"test \"$(systemctl show -P KillMode {unit})\" = control-group"
        )
        main_pid = machine.succeed(f"systemctl show -P MainPID {unit}").strip()
        machine.succeed(
            "test \"$(awk '/^Uid:/ {print $2}' "
            f"/proc/{main_pid}/status)\" = \"$(id -u "
            f"$(systemctl show -P User {unit}))\""
        )

    # Requires/PartOf edges are the lifecycle contract, and a real stop must
    # tear down the same contributor process tree.
    requires = machine.succeed(
        "systemctl show -P Requires gas-city-contributor.service"
    )
    for required in [
        "gascity-agent.service",
        "gascity-discord.service",
        "gascity-publisher.service",
        "gascity-egress.service",
        "gascity-free-space-monitor.service",
        "gascity-check.service",
        "gascity-buildbuddy-proxy.service",
    ]:
        assert required in requires, f"missing Requires edge: {required}"
    for unit in [
        "gascity-agent.service",
        "gascity-discord.service",
        "gascity-publisher.service",
        "gascity-egress.service",
        "gascity-free-space-monitor.service",
        "gascity-check.service",
        "gascity-buildbuddy-proxy.service",
    ]:
        part_of = machine.succeed(f"systemctl show -P PartOf {unit}")
        assert "gas-city-contributor.service" in part_of

    child_pid = machine.succeed(
        "cat /var/lib/gascity-contributor/state/fixture/acp-current.pid"
    ).strip()
    agent_cgroup = machine.succeed(
        "systemctl show -P ControlGroup gascity-agent.service"
    ).strip()
    child_cgroup = machine.succeed(
        f"awk -F: '$1 == 0 {{print $3}}' /proc/{child_pid}/cgroup"
    ).strip()
    assert child_cgroup == agent_cgroup
    assert child_cgroup.startswith(
        "/gascity.slice/gascity-contributor.slice/"
    )
    machine.succeed(
        f"test \"$(readlink /proc/{child_pid}/ns/net)\" != \"$(readlink /proc/1/ns/net)\""
    )
    machine.succeed("systemctl show -P PrivateNetwork gascity-agent.service | grep -qx yes")
    machine.succeed("! pgrep -x tmux")

    # Every configured resource and restart property is visible in systemd's
    # rendered unit, while a bounded task fixture reaches the slice limit.
    slice_unit = machine.succeed("systemctl cat gascity-contributor.slice")
    for setting in [
        "CPUQuota=37%",
        "MemoryHigh=18%",
        "MemoryMax=24%",
        "MemorySwapMax=0",
        "TasksMax=128",
    ]:
        assert setting in slice_unit, f"resource setting was not rendered: {setting}"
    for unit in [
        "gas-city-contributor.service",
        "gascity-agent.service",
        "gascity-discord.service",
        "gascity-publisher.service",
    ]:
        machine.succeed(f"systemctl show -P Restart {unit} | grep -qx on-failure")
        machine.succeed(f"systemctl show -P RestartUSec {unit} | grep -Eq '2s|2000000'")
    machine.succeed(
        "systemd-run --quiet --wait --collect "
        "--unit=gascity-task-fixture --slice=gascity-contributor.slice "
        "${taskFixture}"
    )
    machine.succeed(
        f"grep -q -- '--max-agents.*1' {fake_agent} && "
        f"grep -q -- '--max-active-runs.*1' {fake_agent}"
    )
    check_exec = machine.succeed("systemctl cat gascity-check.service")
    assert "--max-heavy-checks 1" in check_exec
    assert "/nix/var/nix/daemon-socket/socket" in check_exec
    check_pid = machine.succeed(
        "systemctl show -P MainPID gascity-check.service"
    ).strip()
    check_env = machine.succeed(f"xargs -0 -n1 </proc/{check_pid}/environ")
    assert "NIX_REMOTE=local?root=/var/lib/gascity-check/nix-root" in check_env
    assert "max-jobs = 1" in check_exec and "cores = 2" in check_exec

    # The real launcher lease rejects a second active run, and the compatible
    # generation has already produced the first child.
    machine.succeed(
        f"runuser -u gascity-agent -- {launcher_probe} {generation} "
        "second-run 'concurrency cap'"
    )
    machine.succeed(
        f"runuser -u gascity-agent -- {launcher_probe} incompatible-generation "
        "probe-run stale"
    )

    # Credential projections are service-local.  Source files remain
    # unreadable, and the check service additionally cannot see the host Nix
    # daemon socket.
    for role, expected in [
        ("main", "none"),
        ("agent", "copilot-token"),
        ("discord", "discord-bot-token"),
        ("publisher", "github-app-private-key"),
        ("egress", "none"),
        ("check", "none"),
        ("buildbuddy", "buildbuddy-api-key"),
    ]:
        marker = f"/run/gascity-contributor/test/credentials-{role}"
        machine.wait_for_file(marker)
        text = machine.succeed(f"cat {marker}")
        assert f"expected={expected}" in text
        for credential in [
            "copilot-token",
            "discord-bot-token",
            "github-app-private-key",
            "buildbuddy-api-key",
        ]:
            if credential == expected:
                assert f"{credential}=visible" in text
            else:
                assert f"{credential}=hidden" in text
        assert "copilot-token=readable" not in text
        assert "discord-token=readable" not in text
        assert "github-key=readable" not in text
        assert "buildbuddy-key=readable" not in text
    assert "nix-daemon=hidden" in machine.succeed(
        "cat /run/gascity-contributor/test/credentials-check"
    )

    boundary = json.loads(
        machine.succeed("cat /run/gascity-contributor/test/main-boundary.json")
    )
    for key in [
        "projection-read",
        "store-visible",
        "store-executable",
        "managed-city-link",
        "managed-pack-link",
        "loopback-supervisor",
        "loopback-dolt",
    ]:
        assert boundary[key] is True, f"managed positive boundary failed: {key}"
    for key in [
        "projection-write",
        "unrelated-host-write",
        "root-canary-read",
        "discord-source-read",
        "github-source-read",
        "buildbuddy-source-read",
        "daemon-socket-read",
        "systemd-socket-read",
    ]:
        assert boundary[key] is False, f"host boundary was readable/writable: {key}"
    machine.succeed("test ! -e /etc/gascity-test/unrelated-host-write")
    machine.succeed(
        f"test -x {package}/bin/gc && test -d /var/lib/gascity-check/nix-root"
    )

    # The declared loopback owners can connect, while another local uid and
    # the egress identity cannot reach the supervisor/Dolt fixtures.  The
    # egress peer accepts only the fake allowlisted channel and rejects
    # private/link-local/metadata destinations without DNS or external traffic.
    owner_probe = (
        f"{python} -c "
        "'import socket; "
        "s=socket.create_connection((\"127.0.0.1\",18372),1); "
        "s.close()'"
    )
    machine.succeed(f"runuser -u gascity -- {owner_probe}")
    machine.succeed(f"! runuser -u alice -- {owner_probe}")
    public_owner_probe = (
        f"{python} -c "
        "'import socket; "
        "s=socket.socket(socket.AF_UNIX); "
        "s.connect(\"/run/gascity-contributor/agent.sock\"); "
        "assert s.recv(64) == b\"fixture-agent/1\\\\n\"; s.close()'"
    )
    machine.succeed(f"runuser -u gascity -- {public_owner_probe}")
    machine.succeed(f"! runuser -u alice -- {public_owner_probe}")
    for host, port in [
        ("127.0.0.1", 18372),
        ("169.254.169.254", 80),
        ("100.100.100.200", 80),
    ]:
        direct_probe = (
            f"{python} -c "
            f"'import socket; socket.create_connection((\"{host}\",{port}),0.4)'"
        )
        machine.succeed(f"! runuser -u gascity-egress -- {direct_probe}")
    machine.succeed(
        f"runuser -u gascity-agent -- env GC_FDPROXY_AUTH={auth} {proxy_fixture}"
    )
    machine.wait_until_succeeds(
        "test -s /run/gascity-contributor/test/egress-allow"
    )
    for uid in ["45102", "45103", "45105", "45106"]:
        machine.succeed(
            f"grep -qx '{uid}' /run/gascity-contributor/test/egress-peer"
        )
    machine.succeed(
        "grep -q '169.254.169.254:80' "
        "/run/gascity-contributor/test/egress-denied"
    )

    listener_lines = machine.succeed("ss -H -ltn").splitlines()
    for line in listener_lines:
        if any(f":{port}" in line for port in ["18372", "13307"]):
            assert (
                "127.0.0.1:" in line or "[::1]:" in line
            ), f"contributor listener is not loopback-only: {line}"
    unix_listeners = set(
        machine.succeed(
            "find /run/gascity-contributor /run/gascity-agent "
            "-type s -printf '%p\n'"
        ).splitlines()
    )
    allowed_unix = {
        "/run/gascity-contributor/egress.sock",
        "/run/gascity-contributor/agent.sock",
        "/run/gascity-contributor/discord.sock",
        "/run/gascity-contributor/publisher.sock",
        "/run/gascity-agent/agent.sock",
        "/run/gascity-contributor/buildbuddy/buildbuddy-upstream.sock",
    }
    assert unix_listeners <= allowed_unix, (
        f"undeclared contributor Unix listener(s): {unix_listeners - allowed_unix}"
    )

    # The controlled helper fixtures exercise fail-closed project quota and
    # reserve checks without depending on the VM filesystem's quota feature.
    machine.succeed("${quotaFixture} | grep -qx 'project quota fail-closed'")
    machine.succeed("${freeSpaceFixture} | grep -qx 'free-space reserve fail-closed'")
    machine.wait_for_unit("gascity-free-space-monitor.service")

    # ACP loss gets a fresh child while the same worktree/context survives.
    old_child = child_pid
    machine.succeed(f"kill -KILL {old_child}")
    machine.wait_until_succeeds(
        "test \"$(cat /var/lib/gascity-contributor/state/fixture/acp-launch-count)\" -ge 2"
    )
    machine.succeed(f"test ! -d /proc/{old_child}")
    new_child = machine.succeed(
        "cat /var/lib/gascity-contributor/state/fixture/acp-current.pid"
    ).strip()
    assert new_child != old_child
    machine.succeed(
        "test -s /var/lib/gascity-contributor/state/worktrees/test-run/fixture-progress.txt"
    )
    machine.succeed(
        "grep -q 'host-run' "
        "/var/lib/gascity-contributor/state/worktrees/test-run/durable-context.json"
    )
    machine.succeed(
        "grep -qx 'denied' "
        "/var/lib/gascity-contributor/state/fixture/acp-discord-token"
    )
    machine.succeed(
        "grep -qx 'denied' "
        "/var/lib/gascity-contributor/state/fixture/acp-github-key"
    )
    machine.succeed(
        "grep -qx 'denied' "
        "/var/lib/gascity-contributor/state/fixture/acp-buildbuddy-key"
    )
    machine.succeed(
        "grep -qx 'present' "
        "/var/lib/gascity-contributor/state/fixture/acp-home-settings"
    )
    machine.succeed(
        "grep -qx 'absent' "
        "/var/lib/gascity-contributor/state/fixture/acp-home-token"
    )

    # Closing the authenticated launcher client is the fixture cancellation
    # path.  It stops the exact child and does not create a replacement.
    machine.succeed("touch /run/gascity-contributor/test/cancel")
    machine.wait_for_file("/run/gascity-contributor/test/cancelled")
    machine.wait_until_succeeds(
        "test ! -d /proc/$(cat /var/lib/gascity-contributor/state/fixture/acp-current.pid)"
    )
    machine.succeed(
        "test -s /var/lib/gascity-contributor/state/worktrees/test-run/fixture-progress.txt"
    )
    cancelled_count = int(
        machine.succeed(
            "cat /var/lib/gascity-contributor/state/fixture/acp-launch-count"
        ).strip()
    )
    machine.sleep(1)
    assert int(
        machine.succeed(
            "cat /var/lib/gascity-contributor/state/fixture/acp-launch-count"
        ).strip()
    ) == cancelled_count

    # A service restart is durable and replaces the cancelled ACP process.
    machine.succeed("rm -f /run/gascity-contributor/test/cancel")
    machine.succeed("systemctl restart gascity-agent.service")
    machine.wait_for_unit("gascity-agent.service")
    machine.succeed("systemctl start gas-city-contributor.service")
    machine.wait_for_unit("gas-city-contributor.service")
    machine.wait_until_succeeds(
        "test -s /var/lib/gascity-contributor/state/fixture/acp-current.pid"
    )
    restarted_child = machine.succeed(
        "cat /var/lib/gascity-contributor/state/fixture/acp-current.pid"
    ).strip()
    machine.succeed(f"test -d /proc/{restarted_child}")
    machine.succeed(
        "test -s /var/lib/gascity-contributor/state/worktrees/test-run/fixture-progress.txt"
    )

    # Main-service failure uses Restart=on-failure and keeps the durable
    # context/worktree.  The child in the separately supervised agent service
    # remains bounded to the same contributor slice.
    before_main_runs = int(
        machine.succeed(
            "cat /var/lib/gascity-contributor/state/fixture/main-runs"
        ).strip()
    )
    machine.succeed(
        "systemctl kill --kill-who=main --signal=KILL gas-city-contributor.service"
    )
    machine.wait_until_succeeds(
        "test \"$(systemctl is-active gas-city-contributor.service)\" = active"
    )
    machine.wait_until_succeeds(
        "test \"$(cat /var/lib/gascity-contributor/state/fixture/main-runs)\" -gt "
        f"{before_main_runs}"
    )
    machine.succeed(
        "test -s /var/lib/gascity-contributor/state/worktrees/test-run/durable-context.json"
    )

    # Operator authorization is local and explicit: alice is allowed, bob is
    # not.  The authorized command need not reach a real supervisor in this
    # fixture; sudo policy admission is the boundary under test.
    machine.succeed(
        f"runuser -u alice -- sudo -n -l | grep -F '{package}/bin/gascity-status'"
    )
    machine.succeed(
        f"! runuser -u bob -- sudo -n -u gascity {package}/bin/gascity-status"
    )

    # Reuse the repository's deterministic ACP, Discord, GitHub, and
    # BuildBuddy doubles from inside the VM.  They cover CAS/restart/duplicate/
    # cancellation behavior without a network or provider credential.
    machine.succeed(
        "cd /tmp/gascity-fixtures/tests/fixtures/gas-city && "
        f"{python} acp/run.py"
    )
    machine.succeed(
        f"{python} /tmp/gascity-fixtures/tests/fixtures/gas-city/discord/test_router.py"
    )
    machine.succeed(
        f"{python} /tmp/gascity-fixtures/tests/fixtures/gas-city/github/test_publisher.py"
    )
    machine.succeed(
        f"{python} /tmp/gascity-fixtures/tests/fixtures/gas-city/buildbuddy/run.py"
    )
    machine.succeed(f"{package}/bin/bazel --version | grep -F 'bazel 9.1.1'")

    # Runtime state/configuration is deliberately not a d2b delivery/panel
    # surface.  Immutable fixture sources are outside this generated-state
    # census.
    forbidden = (
        "d2b-panel|d2b-panel-fix|d2b-panel-round|d2b-wave-delivery|"
        "panel-request|panel-attest|merge-eligibility|make-records\\.mjs|"
        "selection-table\\.json|\\.scratch/panel|"
        "packages/xtask/src/delivery|evidence-pinning"
    )
    for root in [
        "/var/lib/gascity-contributor/state",
        "/run/gascity-contributor",
        "/var/lib/gascity-discord",
        "/var/lib/gascity-publisher",
        "/var/lib/gascity-check",
        "/var/lib/gascity-buildbuddy-proxy",
    ]:
        machine.succeed(
            f"! find {root} -type f -print0 | "
            f"xargs -0 -r grep -I -E -i '{forbidden}' 2>/dev/null"
        )

    # PartOf plus KillMode=control-group is verified with the live child, not
    # just systemd metadata.  Stopping the named unit removes every fake child
    # and sidecar, and a subsequent start proves the lifecycle can be resumed.
    final_child = machine.succeed(
        "cat /var/lib/gascity-contributor/state/fixture/acp-current.pid"
    ).strip()
    machine.succeed("systemctl stop gas-city-contributor.service")
    machine.wait_until_succeeds(
        "test \"$(systemctl is-active gas-city-contributor.service || true)\" "
        "= inactive"
    )
    machine.succeed(f"test ! -d /proc/{final_child}")
    for unit in [
        "gascity-agent.service",
        "gascity-discord.service",
        "gascity-publisher.service",
        "gascity-egress.service",
        "gascity-free-space-monitor.service",
        "gascity-check.service",
        "gascity-buildbuddy-proxy.service",
    ]:
        machine.wait_until_succeeds(
            f"test \"$(systemctl is-active {unit} || true)\" = inactive"
        )
    machine.succeed("systemctl start gas-city-contributor.service")
    machine.wait_for_unit("gas-city-contributor.service")
    machine.wait_for_file("/run/gascity-contributor/readiness.json")
    machine.succeed(
        "test -s /var/lib/gascity-contributor/state/worktrees/test-run/durable-context.json"
    )
  '';
}
