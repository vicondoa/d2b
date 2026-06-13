# Helper that bridges stdio to a Cloud-Hypervisor vsock port via the
# CH textual protocol on the base UDS.
#
# Cloud-Hypervisor supports two host-side vsock idioms:
#
#  1. GUEST→HOST (guest initiates):
#     The host program creates `<base>_<port>` as a UNIX-LISTEN.
#     When a guest does `connect(AF_VSOCK, CID=2, port=<port>)`,
#     CH proxies that to a UNIX connect on the LISTENer.
#
#  2. HOST→GUEST (host initiates):
#     The host program connects to the BASE UDS (`<base>`), sends
#     `CONNECT <port>\n`, reads back `OK <local-port>\n`, then
#     bidirectional bytes flow. There is NO per-port file for
#     host-initiated connections — CH does not create
#     `<base>_<port>` as a LISTENer when the guest does VSOCK-LISTEN.
#
# Pre-v0.2.0 the framework's host-bridge and per-VM relay tried to
# `UNIX-CONNECT:<base>_<port>` for the stack-VM side, which is the
# wrong idiom: that file never exists for host→guest, so socat
# bailed with ENOENT and OTLP data never reached the stack VM.
#
# This helper implements the textual protocol. Plug it into socat
# with `EXEC:"nixling-ch-vsock-connect <base> <port>"`.
{ pkgs, ... }:

pkgs.writeShellApplication {
  name = "nixling-ch-vsock-connect";
  runtimeInputs = with pkgs; [ python3 ];
  text = ''
    exec ${pkgs.python3}/bin/python3 -u ${pkgs.writeText "nixling-ch-vsock-connect.py" ''
      """CH host->guest vsock bridge over stdin/stdout.

      Speaks Cloud-Hypervisor's textual vsock protocol:
        - CONNECT to the BASE UDS at sys.argv[1]
        - send "CONNECT <port>\n"
        - read "OK <local-port>\n" reply
        - bidirectional bytes between stdio and the UDS

      Plug into socat with:
        EXEC:"nixling-ch-vsock-connect <base> <port>"
      """
      import os
      import socket
      import sys
      import threading
      import time
      HANDSHAKE_TIMEOUT_SECONDS = 5.0

      def write_all(fd, data):
          view = memoryview(data)
          while view:
              written = os.write(fd, view)
              if written == 0:
                  raise OSError("short write")
              view = view[written:]

      def fwd(src, dst, on_done=None):
          try:
              while True:
                  try:
                      data = os.read(src, 65536)
                  except OSError:
                      break
                  if not data:
                      break
                  try:
                      write_all(dst, data)
                  except OSError:
                      break
          finally:
              if on_done is not None:
                  on_done()

      def main():
          if len(sys.argv) != 3:
              sys.stderr.write("usage: nixling-ch-vsock-connect <base-socket> <port>\n")
              sys.exit(2)
          base, port = sys.argv[1], sys.argv[2]
          deadline = time.monotonic() + HANDSHAKE_TIMEOUT_SECONDS

          def refresh_deadline():
              remaining = deadline - time.monotonic()
              if remaining <= 0:
                  sys.stderr.write("nixling-ch-vsock-connect: connect-timeout\n")
                  sys.exit(1)
              sock.settimeout(remaining)

          try:
              sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
              refresh_deadline()
              sock.connect(base)
          except TimeoutError:
              sys.stderr.write("nixling-ch-vsock-connect: connect-timeout\n")
              sys.exit(1)
          except OSError:
              sys.stderr.write("nixling-ch-vsock-connect: transport-unreachable\n")
              sys.exit(1)
          try:
              refresh_deadline()
              sock.sendall(f"CONNECT {port}\n".encode())
          except TimeoutError:
              sys.stderr.write("nixling-ch-vsock-connect: connect-timeout\n")
              sys.exit(1)
          except OSError:
              sys.stderr.write("nixling-ch-vsock-connect: transport-unreachable\n")
              sys.exit(1)
          # Read CH's OK line byte-by-byte so we don't slurp payload bytes.
          reply = b""
          while not reply.endswith(b"\n"):
              refresh_deadline()
              try:
                  chunk = sock.recv(1)
              except TimeoutError:
                  sys.stderr.write("nixling-ch-vsock-connect: connect-timeout\n")
                  sys.exit(1)
              except OSError:
                  sys.stderr.write("nixling-ch-vsock-connect: transport-unreachable\n")
                  sys.exit(1)
              if not chunk:
                  sys.stderr.write("nixling-ch-vsock-connect: eof-before-ack\n")
                  sys.exit(1)
              reply += chunk
              if len(reply) > 128:
                  sys.stderr.write("nixling-ch-vsock-connect: ack-too-long\n")
                  sys.exit(1)
          if not reply.startswith(b"OK ") or not reply.endswith(b"\n"):
              sys.stderr.write("nixling-ch-vsock-connect: connect-refused\n")
              sys.exit(1)
          local_port = reply[3:-1]
          if not local_port.isdigit() or int(local_port) > 0xffffffff:
              sys.stderr.write("nixling-ch-vsock-connect: malformed-ack\n")
              sys.exit(1)
          # The ACK value is CH's local-port acknowledgement, not a buffer
          # size or flow-control input. Forward the post-OK stream as-is.
          sock.settimeout(None)
          # Two-way pipe between stdio and the UDS.
          sock_fd = sock.fileno()
          def shutdown_write():
              try:
                  sock.shutdown(socket.SHUT_WR)
              except OSError:
                  pass

          t = threading.Thread(target=fwd, args=(0, sock_fd, shutdown_write), daemon=True)
          t.start()
          fwd(sock_fd, 1)
          try:
              sock.shutdown(socket.SHUT_RDWR)
          except OSError:
              pass
          sock.close()

      main()
    ''} "$@"
  '';
}
