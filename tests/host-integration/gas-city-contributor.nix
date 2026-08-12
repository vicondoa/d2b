# Type 10 runNixOSTest: the Gas City contributor module under real systemd.
#
# The module and ACP launcher are production paths under test.  The test
# package changes only the external Copilot executable to the deterministic ACP
# fixture; the supervisor and provider sidecars remain narrow local doubles.
{ pkgs, self }:

let
  inherit (pkgs) lib;
  contributor = self.packages.${pkgs.system}.gas-city-contributor;
  contributorPython = "${contributor}/bin/python3";
  contributorScripts = "${contributor}/share/gas-city-contributor/pack/scripts";
  relayAuth = builtins.hashString "sha256" "gascity-fdproxy:acme/project:d2b";
  fixtureSource = ../fixtures/gas-city;

  fakeCopilot = pkgs.writeTextFile {
    name = "gascity-host-fake-copilot";
    destination = "/bin/copilot";
    executable = true;
    text = ''
      #!${contributorPython}
      import json
      import os
      import pathlib
      import sys

      def _readable(path):
          try:
              pathlib.Path(path).read_text(encoding="utf-8")
          except (OSError, UnicodeError):
              return False
          return True

      def _writable(path):
          try:
              with pathlib.Path(path).open("a", encoding="utf-8") as stream:
                  stream.write("fixture\n")
          except OSError:
              return False
          return True

      def _effort():
          try:
              index = sys.argv.index("--effort")
          except ValueError:
              return None
          return sys.argv[index + 1] if index + 1 < len(sys.argv) else None

      run_id = os.environ.get("GC_RUN_ID")
      if run_id in {"planning-run", "code-run"}:
          profile = os.environ.get("GC_PROFILE_NAME", "")
          check_fd = os.environ.get("GC_CHECK_FD")
          settings_path = pathlib.Path(os.environ["COPILOT_HOME"]) / "settings.json"
          settings = json.loads(settings_path.read_text(encoding="utf-8"))
          namespaces = {}
          for name in ("user", "pid", "net", "ipc", "uts", "mnt"):
              try:
                  namespaces[name] = os.readlink(f"/proc/self/ns/{name}")
              except OSError:
                  namespaces[name] = "missing"
          observation = {
              "run_id": run_id,
              "profile": profile,
              "tool_policy": "coding" if check_fd is not None else "planning",
              "effort": _effort(),
              "settings": settings,
              "model": settings.get("model"),
              "context": settings.get("contextTier"),
              "uid": os.getuid(),
              "gid": os.getgid(),
              "groups": os.getgroups(),
              "namespaces": namespaces,
              "progress_fd": os.environ.get("GC_AGENT_FD") is not None,
              "check_fd": check_fd is not None,
              "check_fd_target": (
                  os.readlink(f"/proc/self/fd/{check_fd}")
                  if check_fd is not None
                  else None
              ),
              "workspace_write": _writable("/workspace/.gascity-workspace-probe"),
              "planning_write": _writable(
                  "/workspace/docs/plans/.gascity-planning-probe"
              ),
              "sidecar_source_read": _readable("/etc/gascity-test/discord-token"),
              "state_source_read": _readable(
                  "/var/lib/gascity-contributor/state/fixture"
              ),
              "check_socket_read": _readable("/run/gascity-check/check.sock"),
              "home_settings_read": _readable("/home/copilot/settings.json"),
          }
          print(json.dumps(observation, sort_keys=True), file=sys.stderr, flush=True)
          worktree = pathlib.Path.cwd()
          if check_fd is not None:
              marker = worktree / f"acp-observation-{run_id}.json"
          else:
              marker = worktree / "docs/plans" / f"acp-observation-{run_id}.json"
          marker.write_text(
              json.dumps(observation, sort_keys=True) + "\n",
              encoding="utf-8",
          )
          os.chmod(marker, 0o660)

      _fake_acp_source = ${builtins.toJSON (builtins.readFile "${fixtureSource}/acp/fake_acp.py")}
      exec(
          compile(_fake_acp_source, "tests/fixtures/gas-city/acp/fake_acp.py", "exec"),
          globals(),
          globals(),
      )
    '';
  };

  testPackage = pkgs.symlinkJoin {
    name = "gascity-contributor-host-test";
    paths = [ fakeCopilot contributor ];
    passthru = {
      runtimeScripts = contributor.passthru.runtimeScripts;
    };
  };
  testPackagePython = "${testPackage}/bin/python3";
  testPackageScripts = "${testPackage}/share/gas-city-contributor/pack/scripts";
  generation =
    builtins.substring 0 32 (builtins.hashString "sha256" (toString testPackage));

  credentialProbe = pkgs.writeShellScript "gascity-host-credential-probe" ''
    set -eu
    role="$1"
    expected="$2"
    case "$role" in
      main) root=/run/gascity-contributor/test ;;
      agent) root=/run/gascity-agent/test ;;
      discord) root=/run/gascity-discord/test ;;
      publisher) root=/run/gascity-publisher/test ;;
      egress) root=/run/gascity-egress/test ;;
      check) root=/run/gascity-check/test ;;
      buildbuddy) root=/run/gascity-buildbuddy/test ;;
      *) exit 2 ;;
    esac
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

  fakeBuildBuddy = pkgs.writeTextFile {
    name = "gascity-host-fake-buildbuddy";
    executable = true;
    text = ''
      #!${contributorPython}
      import pathlib
      import signal
      import socket

      stopping = False

      def stop(_signum, _frame):
          global stopping
          stopping = True

      signal.signal(signal.SIGTERM, stop)
      signal.signal(signal.SIGINT, stop)
      pathlib.Path("/tmp/gascity-buildbuddy-private-marker").write_text(
          "buildbuddy\n", encoding="utf-8"
      )
      listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
      listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 0)
      listener.bind(("127.0.0.1", 19801))
      listener.listen(8)
      listener.settimeout(0.2)
      try:
          while not stopping:
              try:
                  connection, _address = listener.accept()
              except socket.timeout:
                  continue
              connection.close()
      finally:
          listener.close()
    '';
  };

  fakeCheck = pkgs.writeShellScript "gascity-host-fake-check" ''
    set -eu
    printf '%s\n' check > /tmp/gascity-check-private-marker
    exec "$@"
  '';

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

      SOCKET = pathlib.Path("/run/gascity-egress/egress.sock")
      MARKERS = pathlib.Path("/run/gascity-egress/test")
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
      import subprocess
      import threading
      import time

      PACKAGE = pathlib.Path("${testPackage}");
      PYTHON = "${testPackagePython}"
      SCRIPTS = pathlib.Path("${testPackageScripts}")
      RUNTIME = pathlib.Path("/run/gascity-contributor");
      STATE = pathlib.Path("/var/lib/gascity-contributor/state");
      MANAGED = pathlib.Path("/var/lib/gascity-contributor/managed");
      MANAGED_FILES = {
          "city": "city.toml",
          "pack": "pack.toml",
          "copilot": "code-luna/settings.json",
          "buildbuddy": "envoy.yaml.tmpl",
      }
      FIXTURE = STATE / "fixture"
      MARKERS = RUNTIME / "test"
      WORKTREES = STATE / "worktrees"
      AGENT_STATE = STATE / "agent-state"
      TERMINAL_ROOT = pathlib.Path(
          os.environ.get("GC_TERMINAL_STATE_ROOT", str(AGENT_STATE / "terminal"))
      )
      GENERATION = os.environ.get("GC_CITY_GENERATION", "${generation}")
      PUBLIC_SOCKET = os.environ.get(
          "GC_AGENT_LAUNCHER_SOCKET", "/run/gascity-agent/agent.sock"
      )
      EGRESS_SOCKET = os.environ.get(
          "GC_EGRESS_SOCKET", "/run/gascity-egress/egress.sock"
      )
      READINESS = RUNTIME / "readiness.json"
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

      def _managed_readable(name):
          try:
              (MANAGED / name / MANAGED_FILES[name]).read_bytes()
          except OSError:
              return False
          return True

      def _managed_create():
          candidate = MANAGED / ".main-create-probe"
          try:
              descriptor = os.open(
                  candidate,
                  os.O_WRONLY | os.O_CREAT | os.O_EXCL,
                  0o600,
              )
              os.close(descriptor)
          except OSError:
              return False
          try:
              candidate.unlink()
          except OSError:
              pass
          return True

      def _managed_replace():
          source = FIXTURE / ".main-replace-source"
          source.write_text("fixture\n", encoding="utf-8")
          try:
              os.replace(source, MANAGED / "city")
          except OSError:
              source.unlink(missing_ok=True)
              return False
          return True

      def _managed_unlink():
          try:
              os.unlink(MANAGED / ".service-unlink-probe")
          except OSError:
              return False
          return True

      def _managed_parent_replace():
          replacement = MANAGED.parent / (
              f".main-managed-parent-replace-{os.getuid()}-{os.getpid()}"
          )
          try:
              os.replace(MANAGED, replacement)
          except OSError:
              return False
          try:
              os.replace(replacement, MANAGED)
          except OSError:
              return True
          return True

      def stop(_signum, _frame):
          global stopping
          stopping = True

      def append(name, value):
          FIXTURE.mkdir(mode=0o770, parents=True, exist_ok=True)
          with (FIXTURE / name).open("a", encoding="utf-8") as stream:
              stream.write(str(value) + "\n")

      def wait_for(path, attempts=200):
          value = pathlib.Path(path)
          for _attempt in range(attempts):
              if value.exists():
                  return
              if stopping:
                  raise RuntimeError("main is stopping")
              time.sleep(0.05)
          raise RuntimeError(f"timed out waiting for {value}")

      def write_terminal(run_id, bead_id):
          TERMINAL_ROOT.mkdir(mode=0o750, parents=True, exist_ok=True)
          target = TERMINAL_ROOT / f"{run_id}.json"
          target.write_text(
              json.dumps(
                  {
                      "schema": 1,
                      "run_id": run_id,
                      "bead_id": bead_id,
                      "generation": GENERATION,
                      "state_schema": "1",
                      "terminal_status": "complete",
                  },
                  sort_keys=True,
                  separators=(",", ":"),
              )
              + "\n",
              encoding="utf-8",
          )
          os.chmod(target, 0o640)

      def prepare_worktree(name, *, planning):
          worktree = WORKTREES / name
          worktree.mkdir(mode=0o770, parents=True, exist_ok=True)
          if worktree.stat().st_uid == os.geteuid():
              os.chmod(worktree, 0o770)
          plans = worktree / "docs/plans"
          plans.mkdir(mode=0o770, parents=True, exist_ok=True)
          for directory in (worktree / "docs", plans):
              if directory.stat().st_uid == os.geteuid():
                  os.chmod(directory, 0o770)
          if not planning:
              progress = worktree / "fixture-progress.txt"
              if not progress.exists():
                  progress.write_text("assigned\n", encoding="utf-8")
              os.chmod(progress, 0o660)
              context = worktree / "durable-context.json"
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
                              "worktree": str(worktree),
                              "review_state": "working",
                              "retry_counters": {},
                              "next_action": "continue",
                          },
                          sort_keys=True,
                      )
                      + "\n",
                      encoding="utf-8",
                  )
                  os.chmod(context, 0o660)
          return worktree

      def connect_egress():
          channel = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
          channel.settimeout(3)
          channel.connect(EGRESS_SOCKET)
          channel.settimeout(None)
          descriptor = channel.detach()
          os.set_inheritable(descriptor, True)
          return descriptor

      def launch_profile(run_id, bead_id, profile, tool_policy, worktree):
          for marker in (
              FIXTURE / f"{run_id}-ready",
              FIXTURE / f"{run_id}-error",
              FIXTURE / f"{run_id}-session.json",
              worktree / f"acp-observation-{run_id}.json",
              worktree / "docs/plans" / f"acp-observation-{run_id}.json",
          ):
              marker.unlink(missing_ok=True)
          proxy_fd = connect_egress()
          progress_parent, progress_child = socket.socketpair()
          control_parent, control_child = socket.socketpair()
          for descriptor in (
              progress_parent.fileno(),
              progress_child.fileno(),
              control_parent.fileno(),
              control_child.fileno(),
          ):
              os.set_inheritable(descriptor, True)
          environment = dict(os.environ)
          environment.update(
              {
                  "GC_PROXY_FD": str(proxy_fd),
                  "GC_AGENT_FD": str(progress_child.fileno()),
                  "GC_CONTROL_FD": str(control_child.fileno()),
              }
          )
          command = [
              PYTHON,
              str(SCRIPTS / "copilot-profile.py"),
              "--profile",
              profile,
              "--tool-policy",
              tool_policy,
              "--run-id",
              run_id,
              "--bead-id",
              bead_id,
              "--generation",
              GENERATION,
              "--state-schema",
              "1",
              "--worktree",
              str(worktree),
              "--state-root",
              str(AGENT_STATE),
              "--launcher-socket",
              PUBLIC_SOCKET,
              "--readiness-status",
              str(READINESS),
              "--require-ready",
              "--runtime-path",
              str(PACKAGE / "bin"),
              "--runtime-path",
              str(PACKAGE / "share/gas-city-contributor"),
              "--sandbox-script",
              str(SCRIPTS / "agent-sandbox.py"),
              "--fdproxy-script",
              str(SCRIPTS / "fdproxy.py"),
              "--sandbox-python",
              PYTHON,
              "--bwrap-path",
              str(PACKAGE / "bin/bwrap"),
          ]
          process = subprocess.Popen(
              command,
              stdin=subprocess.PIPE,
              stdout=subprocess.PIPE,
              stderr=subprocess.PIPE,
              cwd=str(worktree),
              env=environment,
              pass_fds=(
                  proxy_fd,
                  progress_child.fileno(),
                  control_child.fileno(),
              ),
              close_fds=True,
          )
          progress_child.close()
          control_child.close()
          return {
              "process": process,
              "control": control_parent,
              "progress": progress_parent,
              "run_id": run_id,
              "bead_id": bead_id,
              "worktree": worktree,
          }

      def frame(value):
          return (json.dumps(value, separators=(",", ":")) + "\n").encode("utf-8")

      def read_response(stream, request_id, responses):
          while True:
              raw = stream.readline()
              if not raw:
                  raise RuntimeError("profile relay closed during ACP session")
              value = json.loads(raw)
              responses.append(value)
              if isinstance(value, dict) and value.get("id") == request_id:
                  return value

      def process_diagnostic(process):
          try:
              process.wait(timeout=3)
          except subprocess.TimeoutExpired:
              return ""
          try:
              return process.stderr.read().decode("utf-8", "replace").strip()
          except (AttributeError, OSError):
              return ""

      def response_values(values, key):
          found = []
          for value in values:
              if isinstance(value, dict):
                  candidate = value.get(key)
                  if isinstance(candidate, str):
                      found.append(candidate)
                  for nested in value.values():
                      found.extend(response_values([nested], key))
              elif isinstance(value, list):
                  found.extend(response_values(value, key))
          return found

      def drive_profile(session, planning):
          process = session["process"]
          responses = []
          try:
              process.stdin.write(
                  frame(
                      {
                          "jsonrpc": "2.0",
                          "id": 1,
                          "method": "initialize",
                          "params": {
                              "protocolVersion": 1,
                              "clientCapabilities": {},
                              "clientInfo": {
                                  "name": "gascity-host-test",
                                  "version": "1",
                              },
                          },
                      }
                  )
              )
              process.stdin.flush()
              read_response(process.stdout, 1, responses)
              process.stdin.write(
                  frame(
                      {
                          "jsonrpc": "2.0",
                          "id": 2,
                          "method": "session/new",
                          "params": {
                              "cwd": "/workspace",
                              "mcpServers": [],
                          },
                      }
                  )
              )
              process.stdin.flush()
              session_response = read_response(process.stdout, 2, responses)
              session_id = session_response["result"]["sessionId"]
              process.stdin.write(
                  frame(
                      {
                          "jsonrpc": "2.0",
                          "id": 3,
                          "method": "session/prompt",
                          "params": {
                              "sessionId": session_id,
                              "prompt": [
                                  {
                                      "type": "text",
                                      "text": "Run the deterministic host integration session.",
                                  }
                              ],
                          },
                      }
                  )
              )
              process.stdin.flush()
              read_response(process.stdout, 3, responses)
              (FIXTURE / f"{session['run_id']}-session.json").write_text(
                  json.dumps(
                      {
                          "models": response_values(responses, "currentModelId"),
                          "contexts": response_values(responses, "contextTier"),
                      },
                      sort_keys=True,
                  )
                  + "\n",
                  encoding="utf-8",
              )
              (FIXTURE / f"{session['run_id']}-ready").write_text(
                  "ready\n", encoding="utf-8"
              )
              if planning:
                  write_terminal(session["run_id"], session["bead_id"])
                  process.stdin.close()
                  process.wait(timeout=10)
              else:
                  process.stdin.flush()
          except (BrokenPipeError, OSError, KeyError, RuntimeError, ValueError) as error:
              detail = process_diagnostic(process)
              message = str(error)
              if detail:
                  message += f": {detail}"
              (FIXTURE / f"{session['run_id']}-error").write_text(
                  message + "\n", encoding="utf-8"
              )
              try:
                  process.terminate()
              except OSError:
                  pass

      def wait_session_ready(session):
          ready = FIXTURE / f"{session['run_id']}-ready"
          error = FIXTURE / f"{session['run_id']}-error"
          for _attempt in range(200):
              if ready.exists():
                  return
              if error.exists():
                  raise RuntimeError(error.read_text(encoding="utf-8"))
              if session["process"].poll() is not None:
                  detail = process_diagnostic(session["process"])
                  message = "profile process exited before ACP readiness"
                  if detail:
                      message += f": {detail}"
                  raise RuntimeError(message)
              if stopping:
                  raise RuntimeError("main is stopping")
              time.sleep(0.05)
          raise RuntimeError(f"timed out waiting for {ready}")

      def close_session(session, operation):
          process = session["process"]
          if process.poll() is None:
              if operation in {"cancel", "drain"}:
                  write_terminal(session["run_id"], session["bead_id"])
                  try:
                      session["control"].sendall(
                          frame({"run_id": session["run_id"], "op": operation})
                      )
                      session["control"].settimeout(3)
                      (FIXTURE / f"{session['run_id']}-control").write_bytes(
                          session["control"].recv(4096)
                      )
                  except OSError:
                      pass
              try:
                  process.wait(timeout=10)
              except subprocess.TimeoutExpired:
                  process.kill()
                  process.wait()
          session["control"].close()
          session["progress"].close()

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
          "managed-city-read": _managed_readable("city"),
          "managed-pack-read": _managed_readable("pack"),
          "managed-copilot-read": _managed_readable("copilot"),
          "managed-buildbuddy-read": _managed_readable("buildbuddy"),
          "managed-create": _managed_create(),
          "managed-replace": _managed_replace(),
          "managed-unlink": _managed_unlink(),
          "managed-parent-replace": _managed_parent_replace(),
          "loopback-supervisor": True,
          "loopback-dolt": True,
      }
      (MARKERS / "main-boundary.json").write_text(
          json.dumps(boundary, sort_keys=True) + "\n", encoding="utf-8"
      )
      planning_worktree = prepare_worktree("planning-run", planning=True)
      code_worktree = prepare_worktree("test-run", planning=False)
      wait_for(PUBLIC_SOCKET)
      planning = launch_profile(
          "planning-run",
          "planning-bead",
          "review-sol",
          "planning",
          planning_worktree,
      )
      planning_thread = threading.Thread(
          target=drive_profile,
          args=(planning, True),
          daemon=True,
      )
      planning_thread.start()
      wait_session_ready(planning)
      planning_thread.join(timeout=10)
      if planning["process"].returncode not in (0, None):
          raise RuntimeError("planning profile failed")
      close_session(planning, "drain")

      active = None
      launch_count = 0

      def launch_code():
          global launch_count
          launch_count += 1
          session = launch_profile(
              "code-run",
              "host-bead",
              "code-luna",
              "coding",
              code_worktree,
          )
          thread = threading.Thread(
              target=drive_profile,
              args=(session, False),
              daemon=True,
          )
          thread.start()
          wait_session_ready(session)
          observation = json.loads(
              (code_worktree / "acp-observation-code-run.json").read_text(
                  encoding="utf-8"
              )
          )
          (FIXTURE / "acp-launch-count").write_text(
              str(launch_count) + "\n", encoding="utf-8"
          )
          return session

      active = launch_code()
      (MARKERS / "main-ready").write_text("ready\n", encoding="utf-8")
      try:
          while not stopping:
              if pathlib.Path("/run/gascity-contributor/test/cancel").exists():
                  if active is not None:
                      close_session(active, "cancel")
                      active = None
                  append("cancelled", "yes")
                  while not stopping:
                      time.sleep(0.1)
                  break
              if active is None:
                  time.sleep(0.1)
                  continue
              if active["process"].poll() is not None:
                  close_session(active, "drain")
                  active = None
                  if not stopping:
                      time.sleep(0.2)
                      active = launch_code()
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
          if active is not None:
              close_session(active, "drain")
          for listener in listeners:
              listener.close()
  '';
  };

  managedAccessProbe = pkgs.writeTextFile {
    name = "gascity-host-managed-access-probe";
    executable = true;
    text = ''
      #!${contributorPython}
      import errno
      import os
      import pathlib

      root = pathlib.Path("/var/lib/gascity-contributor/managed")
      parent = root.parent
      files = {
          "city": "city.toml",
          "pack": "pack.toml",
          "copilot": "code-luna/settings.json",
          "buildbuddy": "envoy.yaml.tmpl",
      }

      for name, relative in files.items():
          try:
              (root / name / relative).read_bytes()
          except OSError as error:
              raise SystemExit(
                  f"managed asset {name} was not readable: {error}"
              )

      def expect_denied(label, operation):
          try:
              operation()
          except OSError as error:
              if error.errno not in (errno.EACCES, errno.EPERM):
                  raise SystemExit(
                      f"managed asset {label} failed for an unexpected reason: "
                      f"{error}"
                  )
          else:
              raise SystemExit(f"managed asset {label} was writable")

      def create_entry():
          path = root / ".service-create-probe"
          descriptor = os.open(
              path,
              os.O_WRONLY | os.O_CREAT | os.O_EXCL,
              0o600,
          )
          os.close(descriptor)

      def replace_entry():
          source = pathlib.Path(
              f"/tmp/gascity-managed-replace-{os.getuid()}-{os.getpid()}"
          )
          source.write_text("fixture\n", encoding="utf-8")
          try:
              os.replace(source, root / "city")
          except OSError:
              source.unlink(missing_ok=True)
              raise

      expect_denied("create", create_entry)
      expect_denied("replace", replace_entry)
      expect_denied(
          "unlink",
          lambda: os.unlink(root / ".service-unlink-probe"),
      )

      def expect_parent_rename_denied(source):
          name = source.name
          replacement = parent / (
              f".service-parent-replace-{name}-{os.getuid()}-{os.getpid()}"
          )
          if source.parent != parent:
              replacement = source.parent / replacement.name
          try:
              os.replace(source, replacement)
          except OSError as error:
              if error.errno not in (errno.EACCES, errno.EPERM):
                  raise SystemExit(
                      f"managed parent {name} rename failed unexpectedly: "
                      f"{error}"
                  )
              return
          try:
              os.replace(replacement, source)
          except OSError as error:
              raise SystemExit(
                  f"managed parent {name} rename could not be restored: {error}"
              )
          raise SystemExit(f"managed parent {name} rename was writable")

      for source in (
          parent / "managed",
          parent / "state",
          parent / "home",
          parent / "gc",
          parent / "cache",
      ):
          expect_parent_rename_denied(source)
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
      worktree = pathlib.Path(
          "/var/lib/gascity-contributor/state/worktrees"
      ) / run_id
      worktree.mkdir(mode=0o770, parents=True, exist_ok=True)
      connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
      connection.settimeout(3)
      connection.connect("/run/gascity-agent/private.sock")
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
      port = int(sys.argv[1]) if len(sys.argv) > 1 else 18999
      channel = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
      channel.connect("/run/gascity-egress/egress.sock")
      descriptor = channel.fileno()
      process = subprocess.Popen(
          [
              sys.executable,
              proxy,
              "--channel-fd",
              str(descriptor),
              "--listen",
              f"127.0.0.1:{port}",
          ],
          pass_fds=(descriptor,),
          close_fds=True,
      )
      try:
          client = None
          for _attempt in range(50):
              try:
                  client = socket.create_connection(("127.0.0.1", port), timeout=1)
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
          denied = socket.create_connection(("127.0.0.1", port), timeout=1)
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
      import sys

      path = pathlib.Path("${contributorScripts}/service-activation.py")
      spec = importlib.util.spec_from_file_location("activation", path)
      module = importlib.util.module_from_spec(spec)
      sys.modules[spec.name] = module
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
      import sys
      import types

      path = pathlib.Path("${contributorScripts}/service-activation.py")
      spec = importlib.util.spec_from_file_location("activation", path)
      module = importlib.util.module_from_spec(spec)
      sys.modules[spec.name] = module
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

  reserveBreachFixture = pkgs.writeTextFile {
    name = "gascity-host-reserve-breach-fixture";
    executable = true;
    text = ''
      #!${contributorPython}
      import importlib.util
      import pathlib
      import sys
      import time

      path = pathlib.Path("${contributorScripts}/service-activation.py")
      spec = importlib.util.spec_from_file_location("activation", path)
      module = importlib.util.module_from_spec(spec)
      sys.modules[spec.name] = module
      spec.loader.exec_module(module)
      trigger = pathlib.Path("/run/gascity-contributor/test/reserve-breach")
      status = pathlib.Path("/run/gascity-contributor/readiness.json")
      while not trigger.exists():
          time.sleep(0.05)
      (trigger.parent / "reserve-breach-observed").write_text(
          "available=4096\n",
          encoding="utf-8",
      )
      module.publish_reserve_breach(
          status,
          generation="${generation}",
          state_schema="1",
      )
      raise SystemExit(1)
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
    imports = [
      (import ../../nixos-modules/gas-city-contributor {
        packageFor = _: testPackage;
      })
    ];

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
    environment.etc."gascity-module-fixtures" = {
      source = ../../nixos-modules/gas-city-contributor;
    };

    systemd.tmpfiles.rules = [
      "d /var/lib/gascity-fixture-scratch 0700 root root -"
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
        cp /etc/gascity-fixtures/github/fixture-private-key.pem \
          /etc/gascity-test/github-key
        printf '%s\n' fixture-buildbuddy-key > /etc/gascity-test/buildbuddy-key
        printf '%s\n' host-projection-canary > /etc/gascity-test/host-canary
        install -d -m 0750 -o gascity -g gascity-contributor \
          /var/lib/gascity-contributor/state/agent-state/terminal
        for path in \
          /var/lib/gascity-contributor/state/worktrees/test-run \
          /var/lib/gascity-contributor/state/worktrees/test-run/docs \
          /var/lib/gascity-contributor/state/worktrees/test-run/docs/plans \
          /var/lib/gascity-contributor/state/worktrees/planning-run \
          /var/lib/gascity-contributor/state/worktrees/planning-run/docs \
          /var/lib/gascity-contributor/state/worktrees/planning-run/docs/plans; do
          install -d -m 0770 "$path"
          chown gascity-agent:gascity-contributor "$path"
          chmod 0770 "$path"
        done
        install -d -m 0700 -o gascity -g gascity \
          /var/lib/gascity-contributor/home \
          /var/lib/gascity-contributor/gc
        printf '%s\n' legacy-home > /var/lib/gascity-contributor/home/legacy-marker
        printf '%s\n' legacy-gc > /var/lib/gascity-contributor/gc/legacy-marker
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
      pkgs.util-linux
      pkgs.jq
      testPackage
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
        checkTimeoutSeconds = 7;
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
      SupplementaryGroups = lib.mkAfter [ "gascity-egress-channel" ];
      Environment = lib.mkAfter [ "GC_PROJECT_QUOTA_SUPPORTED=1" ];
    };

    systemd.services.gascity-agent.serviceConfig.ExecStartPre =
      [ "${credentialProbe} agent copilot-token" ];

    systemd.services.gascity-egress.serviceConfig = {
      ExecStartPre = [ "${credentialProbe} egress none" ];
      ExecStart = lib.mkForce fakeEgress;
    };
    systemd.services.gascity-free-space-monitor.serviceConfig.ExecStart =
      lib.mkForce reserveBreachFixture;
    systemd.services.gascity-discord.serviceConfig.ExecStartPre =
      [ "${credentialProbe} discord discord-bot-token" ];
    systemd.services.gascity-discord.serviceConfig.ExecStart =
      lib.mkForce
        "${fakeSidecar} /run/gascity-discord/discord.sock gascity-discord-channel discord";
    systemd.services.gascity-publisher.serviceConfig.ExecStartPre =
      [ "${credentialProbe} publisher github-app-private-key" ];
    systemd.services.gascity-publisher.serviceConfig.ExecStart =
      lib.mkForce
        "${fakeSidecar} /run/gascity-publisher/publisher.sock gascity-publisher-channel";
    systemd.services.gascity-check.serviceConfig.ExecStartPre =
      [ "${credentialProbe} check none" ];
    # Keep the production PrivateTmp boundary while giving this fixture a
    # disconnected writable tmpfs for its live marker under ProtectSystem.
    systemd.services.gascity-check.serviceConfig.PrivateTmp =
      lib.mkForce "disconnected";
    systemd.services.gascity-check.serviceConfig.ExecStart = lib.mkForce (
      "${fakeCheck} ${testPackagePython} ${testPackageScripts}/check-runner.py"
      + " server"
      + " --store-root /var/lib/gascity-check/nix-root"
      + " --output-root /var/lib/gascity-check/output"
      + " --proxy http://127.0.0.1:3128"
      + " --egress-socket /run/gascity-egress/egress.sock"
      + " --egress-server-uid 45104"
      + " --socket /run/gascity-check/check.sock"
      + " --allowed-uid 45100"
      + " --check-auth-token-env GC_CHECK_AUTH"
      + " --approved-check 'build-artifact-valid=.gc/scripts/checks/build-artifact-valid.sh'"
      + " --max-jobs 1"
      + " --build-cores 2"
      + " --max-heavy-checks 1"
      + " --timeout-seconds 7"
      + " --term-grace 2"
      + " --kill-grace 1"
    );
    systemd.services.gascity-buildbuddy-proxy.serviceConfig.ExecStartPre =
      [ "${credentialProbe} buildbuddy buildbuddy-api-key" ];
    systemd.services.gascity-buildbuddy-proxy.serviceConfig.PrivateTmp =
      lib.mkForce "disconnected";
    systemd.services.gascity-buildbuddy-proxy.serviceConfig.ExecStart =
      lib.mkForce fakeBuildBuddy;
    # The production proxy is replaced with a fixture sidecar above so the
    # broader host test stays credential- and network-free.  Start the real
    # packaged proxy separately with the production unit's effective syscall
    # filter, and fail rather than restart if Envoy is killed during startup.
    systemd.services.gascity-buildbuddy-envoy-syscall-smoke = {
      serviceConfig =
        config.systemd.services.gascity-buildbuddy-proxy.serviceConfig
        // {
          ExecStartPre = [ ];
          RuntimeDirectory = "gascity-buildbuddy-envoy-smoke";
          ReadWritePaths = [ "/run/gascity-buildbuddy-envoy-smoke" ];
          SystemCallFilter =
            config.systemd.services.gascity-buildbuddy-proxy.serviceConfig.SystemCallFilter;
          ExecStart = "${testPackagePython} ${testPackageScripts}/buildbuddy-proxy.py"
            + " serve"
            + " --template ${testPackage}/share/gas-city-contributor/buildbuddy/envoy.yaml.tmpl"
            + " --credential %d/buildbuddy-api-key"
            + " --envoy ${testPackage}/bin/envoy"
            + " --listen 127.0.0.1:19802"
            + " --runtime-dir /run/gascity-buildbuddy-envoy-smoke"
            + " --egress-socket /run/gascity-egress/egress.sock"
            + " --ca ${testPackage}/etc/ssl/certs/ca-bundle.crt";
          Restart = "no";
        };
    };
  };

  testScript = ''
    import json

    package = "${testPackage}"
    python = "${testPackagePython}"
    fdproxy = "${testPackage.passthru.runtimeScripts}/bin/gascity-fdproxy"
    envoy = "${testPackage}/bin/envoy"
    ca_bundle = "${testPackage}/etc/ssl/certs/ca-bundle.crt"
    generation = "${generation}"
    auth = "${relayAuth}"
    launcher_probe = "${launcherProbe}"
    proxy_fixture = "${proxyFixture}"

    start_all()
    machine.succeed(
        "install -m 0440 -o root -g gascity-contributor /dev/null "
        "/var/lib/gascity-contributor/managed/.service-unlink-probe"
    )

    # Copy the repository-relative fixture layout into a disposable tree.  The
    # fixture modules deliberately resolve their package scripts from that
    # layout, so this exercises the exact packaged scripts without importing
    # anything from the test runner's host checkout.
    machine.succeed(
        "rm -rf /tmp/gascity-fixtures && "
        "mkdir -p /tmp/gascity-fixtures/tests/fixtures/gas-city && "
        "cp -rL /etc/gascity-fixtures/. "
        "/tmp/gascity-fixtures/tests/fixtures/gas-city/ && "
        "mkdir -p /tmp/gascity-fixtures/nixos-modules/gas-city-contributor && "
        "cp -rL /etc/gascity-module-fixtures/. "
        "/tmp/gascity-fixtures/nixos-modules/gas-city-contributor/ && "
        "mkdir -p /tmp/gascity-fixtures/nix /tmp/gascity-fixtures/tests/nix && "
        "ln -s ${testPackage}/share/gas-city-contributor "
        "/tmp/gascity-fixtures/nix/gas-city-contributor && "
        "ln -s ../../nix/gas-city-contributor "
        "/tmp/gascity-fixtures/tests/nix/gas-city-contributor && "
        "ln -s ${testPackage}/share/gas-city-contributor/copilot "
        "/tmp/gascity-fixtures/copilot"
    )

    for unit in [
        "gascity-egress.service",
        "gascity-agent.service",
        "gascity-discord.service",
        "gascity-publisher.service",
        "gascity-free-space-monitor.service",
        "gascity-check.service",
        "gascity-buildbuddy-netns.service",
        "gascity-buildbuddy-proxy.service",
        "gas-city-contributor.service",
    ]:
        machine.wait_for_unit(unit)

    for path in [
        "/run/gascity-egress/egress.sock",
        "/run/gascity-agent/agent.sock",
        "/run/gascity-discord/discord.sock",
        "/run/gascity-publisher/publisher.sock",
        "/run/gascity-contributor/readiness.json",
        "/run/gascity-contributor/test/main-ready",
        "/var/lib/gascity-contributor/state/fixture/code-run-session.json",
        "/var/lib/gascity-contributor/state/worktrees/test-run/acp-observation-code-run.json",
        "/var/lib/gascity-contributor/state/worktrees/planning-run/docs/plans/acp-observation-planning-run.json",
    ]:
        machine.wait_for_file(path)

    machine.succeed(
        "systemctl start gascity-buildbuddy-envoy-syscall-smoke.service"
    )
    machine.wait_for_unit("gascity-buildbuddy-envoy-syscall-smoke.service")
    machine.succeed("sleep 2")
    machine.succeed(
        "test \"$(systemctl is-active gascity-buildbuddy-envoy-syscall-smoke.service)\" "
        "= active"
    )
    machine.succeed(
        "systemctl stop gascity-buildbuddy-envoy-syscall-smoke.service"
    )

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
        "gascity-buildbuddy-netns.service",
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
        "gascity-buildbuddy-netns.service",
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
        "gascity-buildbuddy-netns.service",
        "gascity-buildbuddy-proxy.service",
    ]:
        part_of = machine.succeed(f"systemctl show -P PartOf {unit}")
        assert "gas-city-contributor.service" in part_of

    # BuildBuddy and the uncredentialed check runner share one private network
    # namespace through a credential-free holder.  The loopback listeners must
    # stay inside it, never appearing in the host namespace or from either
    # service identity outside systemd.
    joins_namespace = machine.succeed(
        "systemctl show -P JoinsNamespaceOf gascity-check.service"
    ).strip()
    proxy_joins_namespace = machine.succeed(
        "systemctl show -P JoinsNamespaceOf gascity-buildbuddy-proxy.service"
    ).strip()
    assert "gascity-buildbuddy-netns.service" in joins_namespace
    assert "gascity-buildbuddy-netns.service" in proxy_joins_namespace
    assert "gascity-buildbuddy-proxy.service" not in joins_namespace
    machine.succeed(
        "systemctl show -P Requires gascity-check.service "
        "| tr ' ' '\\n' | grep -qx gascity-buildbuddy-netns.service"
    )
    machine.succeed(
        "systemctl show -P After gascity-check.service "
        "| tr ' ' '\\n' | grep -qx gascity-buildbuddy-netns.service"
    )
    machine.succeed(
        "systemctl show -P Requires gascity-buildbuddy-proxy.service "
        "| tr ' ' '\\n' | grep -qx gascity-buildbuddy-netns.service"
    )
    machine.succeed(
        "systemctl show -P After gascity-buildbuddy-proxy.service "
        "| tr ' ' '\\n' | grep -qx gascity-buildbuddy-netns.service"
    )
    machine.succeed(
        "systemctl show -P Requires gas-city-contributor.service "
        "| tr ' ' '\\n' | grep -qx gascity-buildbuddy-netns.service"
    )
    for property_name in [
        "PrivateTmp",
        "PrivateDevices",
        "PrivateIPC",
        "PrivateUsers",
        "PrivateMounts",
    ]:
        machine.succeed(
            f"systemctl show -P {property_name} gascity-buildbuddy-netns.service "
            "| grep -qx no"
        )
    machine.succeed(
        "systemctl show -P PrivateNetwork gascity-buildbuddy-netns.service "
        "| grep -qx yes"
    )
    machine.succeed(
        "systemctl show -P User gascity-buildbuddy-netns.service "
        "| grep -qx gascity-egress"
    )
    for setting in [
        "LoadCredential",
        "StateDirectory",
        "CacheDirectory",
        "RuntimeDirectory",
        "ReadWritePaths",
    ]:
        machine.succeed(
            f"! systemctl cat gascity-buildbuddy-netns.service "
            f"| grep -q '^{setting}='"
        )
    holder_pid = machine.succeed(
        "systemctl show -P MainPID gascity-buildbuddy-netns.service"
    ).strip()
    machine.succeed(
        "systemctl show -P PrivateNetwork gascity-check.service | grep -qx yes"
    )
    machine.succeed(
        "systemctl show -P PrivateNetwork gascity-buildbuddy-proxy.service "
        "| grep -qx yes"
    )
    check_pid = machine.succeed(
        "systemctl show -P MainPID gascity-check.service"
    ).strip()
    proxy_pid = machine.succeed(
        "systemctl show -P MainPID gascity-buildbuddy-proxy.service"
    ).strip()
    namespace_inodes = {
        "holder": machine.succeed(
            f"stat -Lc '%i' /proc/{holder_pid}/ns/net"
        ).strip(),
        "check": machine.succeed(
            f"stat -Lc '%i' /proc/{check_pid}/ns/net"
        ).strip(),
        "proxy": machine.succeed(
            f"stat -Lc '%i' /proc/{proxy_pid}/ns/net"
        ).strip(),
    }
    host_namespace_inode = machine.succeed(
        "stat -Lc '%i' /proc/1/ns/net"
    ).strip()
    assert len(set(namespace_inodes.values())) == 1
    assert namespace_inodes["holder"] != host_namespace_inode
    mount_namespace_inodes = {
        "holder": machine.succeed(
            f"stat -Lc '%i' /proc/{holder_pid}/ns/mnt"
        ).strip(),
        "check": machine.succeed(
            f"stat -Lc '%i' /proc/{check_pid}/ns/mnt"
        ).strip(),
        "proxy": machine.succeed(
            f"stat -Lc '%i' /proc/{proxy_pid}/ns/mnt"
        ).strip(),
    }
    assert len(set(mount_namespace_inodes.values())) == 3
    holder_network_sockets = machine.succeed(
        f"nsenter -t {holder_pid} -n ss -H -tnp"
    )
    assert f"pid={holder_pid}," not in holder_network_sockets
    shared_listeners = machine.succeed(
        f"nsenter -t {holder_pid} -n ss -H -ltnp"
    )
    for listener in ["127.0.0.1:3128", "127.0.0.1:19801"]:
        assert listener in shared_listeners, (
            f"shared network namespace is missing listener {listener}: "
            f"{shared_listeners}"
        )
    host_listeners = machine.succeed("ss -H -ltn")
    for listener in ["127.0.0.1:3128", "127.0.0.1:19801"]:
        assert listener not in host_listeners, (
            f"private listener is host-visible: {listener}: {host_listeners}"
        )
    for identity in ["gascity-check", "gascity-buildbuddy-proxy"]:
        for port in [3128, 19801]:
            machine.succeed(
                f"! runuser -u {identity} -- {python} -c "
                f"'import socket; "
                f"socket.create_connection((\"127.0.0.1\",{port}),0.4)'"
            )

    check_tmp_marker = "/tmp/gascity-check-private-marker"
    proxy_tmp_marker = "/tmp/gascity-buildbuddy-private-marker"
    machine.succeed(
        f"nsenter -t {check_pid} -m -- test -s {check_tmp_marker}"
    )
    machine.succeed(
        f"nsenter -t {proxy_pid} -m -- test ! -e {check_tmp_marker}"
    )
    machine.succeed(
        f"nsenter -t {proxy_pid} -m -- test -s {proxy_tmp_marker}"
    )
    machine.succeed(
        f"nsenter -t {check_pid} -m -- test ! -e {proxy_tmp_marker}"
    )
    machine.succeed(
        f"test ! -e {check_tmp_marker} && test ! -e {proxy_tmp_marker}"
    )

    def active_child_pid():
        command = (
            "for pid in $(pgrep -f 'gascity-host-fake-copilot' || true); do "
            "  comm=$(cat /proc/$pid/comm 2>/dev/null || true); "
            "  case \"$comm\" in python*) "
            "    run_id=$(tr '\\0' '\\n' </proc/$pid/environ 2>/dev/null "
            "      | sed -n 's/^GC_RUN_ID=//p'); "
            "    test \"$run_id\" = code-run || continue; "
            "    cgroup=$(awk -F: '$1 == 0 {print $3}' /proc/$pid/cgroup); "
            "    expected=$(systemctl show -P ControlGroup gascity-agent.service); "
            "    if test \"$cgroup\" = \"$expected\"; then echo \"$pid\"; exit 0; fi ;; "
            "  esac; "
            "done; exit 1"
        )
        machine.wait_until_succeeds(command)
        pid = machine.succeed(command).strip()
        machine.succeed(
            "printf '%s\\n' "
            f"{pid} > /var/lib/gascity-contributor/state/fixture/acp-current.pid"
        )
        return pid

    child_pid = active_child_pid()
    coding_observation = json.loads(
        machine.succeed(
            "cat /var/lib/gascity-contributor/state/worktrees/test-run/"
            "acp-observation-code-run.json"
        )
    )
    planning_observation = json.loads(
        machine.succeed(
            "cat /var/lib/gascity-contributor/state/worktrees/planning-run/"
            "docs/plans/acp-observation-planning-run.json"
        )
    )
    assert coding_observation["profile"] == "code-luna"
    assert coding_observation["tool_policy"] == "coding"
    assert coding_observation["settings"] == {
        "model": "gpt-5.6-luna",
        "contextTier": "default",
    }
    assert coding_observation["model"] == "gpt-5.6-luna"
    assert coding_observation["context"] == "default"
    assert coding_observation["effort"] == "max"
    assert coding_observation["check_fd"] is True
    assert coding_observation["progress_fd"] is True
    assert coding_observation["workspace_write"] is True
    assert coding_observation["planning_write"] is True
    assert coding_observation["sidecar_source_read"] is False
    assert coding_observation["state_source_read"] is False
    assert coding_observation["check_socket_read"] is False
    assert coding_observation["home_settings_read"] is True
    assert coding_observation["check_fd_target"].startswith("socket:")
    assert set(coding_observation["namespaces"]) == {
        "user",
        "pid",
        "net",
        "ipc",
        "uts",
        "mnt",
    }
    for namespace, value in coding_observation["namespaces"].items():
        machine.succeed(
            f"test '{value}' != \"$(readlink /proc/1/ns/{namespace})\""
        )
    assert planning_observation["profile"] == "review-sol"
    assert planning_observation["tool_policy"] == "planning"
    assert planning_observation["settings"] == {
        "model": "gpt-5.6-sol",
        "contextTier": "long_context",
    }
    assert planning_observation["model"] == "gpt-5.6-sol"
    assert planning_observation["context"] == "long_context"
    assert planning_observation["effort"] == "xhigh"
    assert planning_observation["check_fd"] is False
    assert planning_observation["progress_fd"] is True
    assert planning_observation["workspace_write"] is False
    assert planning_observation["planning_write"] is True
    assert planning_observation["sidecar_source_read"] is False
    assert planning_observation["state_source_read"] is False
    assert planning_observation["check_socket_read"] is False
    assert planning_observation["home_settings_read"] is True
    coding_session = json.loads(
        machine.succeed(
            "cat /var/lib/gascity-contributor/state/fixture/code-run-session.json"
        )
    )
    planning_session = json.loads(
        machine.succeed(
            "cat /var/lib/gascity-contributor/state/fixture/planning-run-session.json"
        )
    )
    assert set(coding_session["models"]) == {"gpt-5.6-luna"}
    assert coding_session["contexts"] == []
    assert set(planning_session["models"]) == {"gpt-5.6-sol"}
    assert planning_session["contexts"] == []
    machine.wait_for_file(
        "/nix/var/nix/gcroots/gascity-contributor/code-run/metadata.json"
    )
    gc_metadata = json.loads(
        machine.succeed(
            "cat /nix/var/nix/gcroots/gascity-contributor/code-run/metadata.json"
        )
    )
    assert gc_metadata["run_id"] == "code-run"
    assert gc_metadata["bead_id"] == "host-bead"
    assert set(gc_metadata["targets"]) == {
        "package",
        "city",
        "pack",
        "profiles",
        "instructions",
    }
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
    agent_exec = machine.succeed("systemctl cat gascity-agent.service")
    assert "gascity-agent-start" in agent_exec
    check_exec = machine.succeed("systemctl cat gascity-check.service")
    assert "--max-heavy-checks 1" in check_exec
    assert "--timeout-seconds 7" in check_exec
    assert "--socket /run/gascity-check/check.sock" in check_exec
    assert (
        "--approved-check 'build-artifact-valid=.gc/scripts/checks/build-artifact-valid.sh'"
        in check_exec
    )
    assert "/nix/var/nix/daemon-socket/socket" in check_exec
    check_pid = machine.succeed(
        "systemctl show -P MainPID gascity-check.service"
    ).strip()
    check_env = machine.succeed(f"xargs -0 -n1 </proc/{check_pid}/environ")
    assert "NIX_REMOTE=local?root=/var/lib/gascity-check/nix-root" in check_env
    assert "max-jobs = 1" in check_exec and "cores = 2" in check_exec
    machine.succeed("test -S /run/gascity-check/check.sock")
    machine.succeed(
        "systemctl show -P SupplementaryGroups gas-city-contributor.service "
        "| grep -qw gascity-check-channel"
    )

    # The real launcher lease rejects a second active run, and the compatible
    # generation has already produced the first child.
    machine.succeed(
        f"runuser -u gascity-agent -- {launcher_probe} {generation} "
        "second-run 'concurrency cap'"
    )
    machine.succeed(
        f"runuser -u gascity-agent -- {launcher_probe} incompatible-generation "
        "probe-run generation"
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
        marker_root = {
            "main": "/run/gascity-contributor/test",
            "agent": "/run/gascity-agent/test",
            "discord": "/run/gascity-discord/test",
            "publisher": "/run/gascity-publisher/test",
            "egress": "/run/gascity-egress/test",
            "check": "/run/gascity-check/test",
            "buildbuddy": "/run/gascity-buildbuddy/test",
        }[role]
        marker = f"{marker_root}/credentials-{role}"
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
        "cat /run/gascity-check/test/credentials-check"
    )

    boundary = json.loads(
        machine.succeed("cat /run/gascity-contributor/test/main-boundary.json")
    )
    parent_stat = machine.succeed(
        "stat -c '%u %g %a' /var/lib/gascity-contributor"
    ).strip().split()
    assert parent_stat[0] == "0"
    assert parent_stat[1] == machine.succeed(
        "getent group gascity-contributor | cut -d: -f3"
    ).strip()
    assert parent_stat[2] == "750"
    machine.succeed(
        "test -d /var/lib/gascity-contributor/state/home && "
        "test -d /var/lib/gascity-contributor/state/gc && "
        "test -s /var/lib/gascity-contributor/home/legacy-marker && "
        "test -s /var/lib/gascity-contributor/gc/legacy-marker"
    )
    main_unit = machine.succeed("systemctl cat gas-city-contributor.service")
    assert "StateDirectory=gascity-contributor/state" in main_unit
    assert "StateDirectoryQuota=33554432" in main_unit
    assert "CacheDirectory=gascity-contributor" in main_unit
    read_write_paths = [
        line.split("=", 1)[1]
        for line in main_unit.splitlines()
        if line.startswith("ReadWritePaths=")
    ]
    assert read_write_paths == [
        "/var/lib/gascity-contributor/state",
        "/run/gascity-contributor",
    ]
    managed_stat = machine.succeed(
        "stat -c '%u %g %a' /var/lib/gascity-contributor/managed"
    ).strip().split()
    assert managed_stat[0] == "0"
    assert managed_stat[1] == machine.succeed(
        "getent group gascity-contributor | cut -d: -f3"
    ).strip()
    assert managed_stat[2] == "750"
    for key in [
        "projection-read",
        "store-visible",
        "store-executable",
        "managed-city-link",
        "managed-pack-link",
        "managed-city-read",
        "managed-pack-read",
        "managed-copilot-read",
        "managed-buildbuddy-read",
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
        "managed-create",
        "managed-replace",
        "managed-unlink",
        "managed-parent-replace",
    ]:
        assert boundary[key] is False, f"host boundary was readable/writable: {key}"
    for identity in [
        "gascity",
        "gascity-agent",
        "gascity-discord",
        "gascity-publisher",
        "gascity-egress",
        "gascity-check",
        "gascity-buildbuddy-proxy",
    ]:
        machine.succeed(
            f"runuser -u {identity} -- ${managedAccessProbe}"
        )
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
        "s.connect(\"/run/gascity-agent/agent.sock\"); "
        "s.close()'"
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
    for index, identity in enumerate([
        "gascity-agent",
        "gascity-discord",
        "gascity-publisher",
        "gascity-check",
        "gascity-buildbuddy-proxy",
    ]):
        machine.succeed(
            f"runuser -u {identity} -- env GC_FDPROXY_AUTH={auth} "
            f"{proxy_fixture} {18999 + index}"
        )
    machine.wait_until_succeeds(
        "test -s /run/gascity-egress/test/egress-allow"
    )
    for uid in ["45102", "45103", "45105", "45106"]:
        machine.succeed(
            f"grep -qx '{uid}' /run/gascity-egress/test/egress-peer"
        )
    machine.succeed(
        "grep -q '169.254.169.254:80' "
        "/run/gascity-egress/test/egress-denied"
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
            "/run/gascity-egress /run/gascity-discord /run/gascity-publisher "
            "/run/gascity-check /run/gascity-buildbuddy "
            "-type s -printf '%p\n'"
        ).splitlines()
    )
    allowed_unix = {
        "/run/gascity-agent/agent.sock",
        "/run/gascity-agent/private.sock",
        "/run/gascity-egress/egress.sock",
        "/run/gascity-discord/discord.sock",
        "/run/gascity-publisher/publisher.sock",
        "/run/gascity-check/check.sock",
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
        "test \"$(cat /var/lib/gascity-contributor/state/fixture/acp-launch-count)\" -ge 2",
        timeout=30,
    )
    machine.succeed(f"test ! -d /proc/{old_child}")
    new_child = active_child_pid()
    assert new_child != old_child
    new_coding_observation = json.loads(
        machine.succeed(
            "cat /var/lib/gascity-contributor/state/worktrees/test-run/"
            "acp-observation-code-run.json"
        )
    )
    assert new_coding_observation["uid"] == 45101
    assert new_coding_observation["check_fd"] is True
    assert new_coding_observation["tool_policy"] == "coding"
    machine.succeed(
        "test -s /var/lib/gascity-contributor/state/worktrees/test-run/fixture-progress.txt"
    )
    machine.succeed(
        "grep -q 'host-run' "
        "/var/lib/gascity-contributor/state/worktrees/test-run/durable-context.json"
    )
    assert new_coding_observation["sidecar_source_read"] is False
    assert new_coding_observation["state_source_read"] is False
    assert new_coding_observation["home_settings_read"] is True

    # The real control attachment drives cancellation through the authenticated
    # public relay and launcher-owned process group.
    cancelled_child = new_child
    machine.succeed("touch /run/gascity-contributor/test/cancel")
    machine.wait_for_file(
        "/var/lib/gascity-contributor/state/fixture/cancelled"
    )
    machine.succeed(
        "grep -q '\"op\":\"cancel\"' "
        "/var/lib/gascity-contributor/state/fixture/code-run-control"
    )
    machine.wait_until_succeeds(
        "test ! -d /proc/$(cat /var/lib/gascity-contributor/state/fixture/acp-current.pid)"
    )
    machine.wait_until_succeeds(
        "test ! -e /nix/var/nix/gcroots/gascity-contributor/code-run"
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
    machine.succeed(
        "rm -f /var/lib/gascity-contributor/state/fixture/acp-current.pid"
    )
    machine.succeed("systemctl restart gascity-agent.service")
    machine.wait_for_unit("gascity-agent.service")
    machine.succeed("systemctl start gas-city-contributor.service")
    machine.wait_for_unit("gas-city-contributor.service")
    restarted_child = active_child_pid()
    assert restarted_child != cancelled_child
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
        "test -d /var/lib/gascity-fixture-scratch && "
        "test \"$(stat -c '%u %g %a' /var/lib/gascity-fixture-scratch)\" "
        "= '0 0 700'"
    )
    machine.succeed(
        f"cd /tmp/gascity-fixtures/tests/fixtures/gas-city && "
        f"GAS_CITY_FDPROXY={fdproxy} "
        "GC_MANAGED_ASSET_SCRATCH_ROOT=/var/lib/gascity-fixture-scratch "
        f"{python} acp/run.py"
    )
    machine.succeed(
        f"{python} /tmp/gascity-fixtures/tests/fixtures/gas-city/discord/test_router.py"
    )
    machine.succeed(
        f"{python} /tmp/gascity-fixtures/tests/fixtures/gas-city/github/test_publisher.py"
    )
    machine.succeed(
        "cd /tmp/gascity-fixtures/tests/fixtures/gas-city && "
        f"PATH=/definitely-not-a-command-path "
        "GC_TEST_OPENSSL=${testPackage}/bin/openssl "
        f"{python} github/test_publisher.py "
        "PublisherFixture.test_github_api_signs_jwt_with_packaged_openssl_under_restricted_path"
    )
    machine.succeed(
        f"GAS_CITY_ENVOY={envoy} GAS_CITY_CA_BUNDLE={ca_bundle} "
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
            f"! grep -r -I -E -i '{forbidden}' {root} 2>/dev/null"
        )

    # PartOf plus KillMode=control-group is verified with the live child, not
    # just systemd metadata.  Stopping the named unit removes every fake child
    # and sidecar, and a subsequent start proves the lifecycle can be resumed.
    final_child = active_child_pid()
    machine.succeed("systemctl stop gas-city-contributor.service")
    machine.wait_until_succeeds(
        "test \"$(systemctl is-active gas-city-contributor.service || true)\" "
        "= inactive"
    )
    machine.wait_until_succeeds(f"test ! -d /proc/{final_child}")
    for unit in [
        "gascity-agent.service",
        "gascity-discord.service",
        "gascity-publisher.service",
        "gascity-egress.service",
        "gascity-free-space-monitor.service",
        "gascity-check.service",
        "gascity-buildbuddy-netns.service",
        "gascity-buildbuddy-proxy.service",
    ]:
        machine.wait_until_succeeds(
            f"test \"$(systemctl is-active {unit} || true)\" != active"
        )
    machine.succeed("systemctl start gas-city-contributor.service")
    machine.wait_for_unit("gas-city-contributor.service")
    machine.wait_for_file("/run/gascity-contributor/readiness.json")
    machine.succeed(
        "test -s /var/lib/gascity-contributor/state/worktrees/test-run/durable-context.json"
    )

    # A reserve breach publishes the fail-closed readiness state first, then
    # systemd dependency propagation stops the active ACP writer and check
    # runner.  The fixture records a synthetic positive free-space amount, so
    # this proves the stop happens before the filesystem reaches zero.
    breach_child = machine.succeed(
        "cat /var/lib/gascity-contributor/state/fixture/acp-current.pid"
    ).strip()
    breach_check = machine.succeed(
        "systemctl show -P MainPID gascity-check.service"
    ).strip()
    machine.succeed("touch /run/gascity-contributor/test/reserve-breach")
    machine.wait_for_file("/run/gascity-contributor/test/reserve-breach-observed")
    machine.succeed(
        "grep -qx 'available=4096' "
        "/run/gascity-contributor/test/reserve-breach-observed"
    )
    machine.wait_until_succeeds(
        "test \"$(systemctl is-active gascity-free-space-monitor.service || true)\" "
        "= failed"
    )
    machine.wait_until_succeeds(
        f"test ! -d /proc/{breach_child} && ! -d /proc/{breach_check}"
    )
    for unit in [
        "gas-city-contributor.service",
        "gascity-agent.service",
        "gascity-discord.service",
        "gascity-publisher.service",
        "gascity-egress.service",
        "gascity-check.service",
        "gascity-buildbuddy-netns.service",
        "gascity-buildbuddy-proxy.service",
    ]:
        machine.wait_until_succeeds(
            f"test \"$(systemctl is-active {unit} || true)\" != active"
        )
    blocked_submit_status, blocked_submit_output = machine.execute(
        "runuser -u alice -- sudo -n -u gascity "
        f"{package}/bin/gascity-submit 2>&1 <<'EOF'\n"
        '{"run_id":"blocked-run","bead_id":"blocked-bead",'
        '"summary":"blocked","base_branch":"v3","repository":"acme/project"}\n'
        "EOF"
    )
    assert blocked_submit_status != 0
    assert "free-space-reserve" in blocked_submit_output
  '';
}
