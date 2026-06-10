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

      def fwd(src, dst):
          while True:
              try:
                  data = os.read(src, 65536)
              except OSError:
                  break
              if not data:
                  break
              try:
                  os.write(dst, data)
              except OSError:
                  break

      def main():
          if len(sys.argv) != 3:
              sys.stderr.write("usage: nixling-ch-vsock-connect <base-socket> <port>\n")
              sys.exit(2)
          base, port = sys.argv[1], sys.argv[2]
          try:
              sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
              sock.connect(base)
          except OSError as e:
              sys.stderr.write(f"nixling-ch-vsock-connect: cannot connect {base}: {e}\n")
              sys.exit(1)
          sock.sendall(f"CONNECT {port}\n".encode())
          # Read CH's OK line byte-by-byte so we don't slurp payload bytes.
          reply = b""
          while not reply.endswith(b"\n"):
              chunk = sock.recv(1)
              if not chunk:
                  sys.stderr.write("nixling-ch-vsock-connect: EOF before OK from CH\n")
                  sys.exit(1)
              reply += chunk
              if len(reply) > 128:
                  sys.stderr.write("nixling-ch-vsock-connect: CH CONNECT reply too long\n")
                  sys.exit(1)
          if not reply.startswith(b"OK ") or not reply.endswith(b"\n"):
              sys.stderr.write(f"nixling-ch-vsock-connect: CH refused: {reply.decode(errors='replace').strip()}\n")
              sys.exit(1)
          local_port = reply[3:-1]
          if not local_port.isdigit() or int(local_port) > 65535:
              sys.stderr.write(f"nixling-ch-vsock-connect: malformed CH OK: {reply.decode(errors='replace').strip()}\n")
              sys.exit(1)
          # The ACK value is CH's local-port acknowledgement, not a buffer
          # size or flow-control input. Forward the post-OK stream as-is.
          # Two-way pipe between stdio and the UDS.
          sock_fd = sock.fileno()
          t = threading.Thread(target=fwd, args=(0, sock_fd), daemon=True)
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
