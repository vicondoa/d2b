{ envMeta, pkgs, lib, ... }:

let
  m = envMeta;
  mdns = m.externalNetwork.mdns;
  avahiEnabled = mdns.enable || mdns.dnsmasqLocal.enable;
  bridgeEnabled = mdns.dnsmasqLocal.enable;
  bridgePort = mdns.dnsmasqLocal.port;

  resolverPython = pkgs.python3.withPackages (ps: [
    ps.dbus-python
  ]);

  resolver = pkgs.writeTextFile {
    name = "d2b-mdns-local-resolver";
    executable = true;
    destination = "/bin/d2b-mdns-local-resolver";
    text = ''
      #!${resolverPython}/bin/python3
      import argparse
      import ipaddress
      import socketserver
      import struct
      import threading

      import dbus

      AVAHI_IF_UNSPEC = -1
      AVAHI_PROTO_UNSPEC = -1
      AVAHI_PROTO_INET = 0
      AVAHI_LOOKUP_NO_FLAGS = 0
      TYPE_A = 1
      CLASS_IN = 1

      def decode_qname(packet, offset):
          labels = []
          jumped = False
          end = offset
          seen = set()
          while True:
              if offset >= len(packet):
                  raise ValueError("truncated qname")
              length = packet[offset]
              if length & 0xC0 == 0xC0:
                  if offset + 1 >= len(packet):
                      raise ValueError("truncated compression pointer")
                  pointer = ((length & 0x3F) << 8) | packet[offset + 1]
                  if pointer in seen:
                      raise ValueError("compression loop")
                  seen.add(pointer)
                  if not jumped:
                      end = offset + 2
                  offset = pointer
                  jumped = True
                  continue
              if length == 0:
                  if not jumped:
                      end = offset + 1
                  return ".".join(labels), end
              offset += 1
              if offset + length > len(packet):
                  raise ValueError("truncated label")
              labels.append(packet[offset:offset + length].decode("ascii").lower())
              offset += length

      def question(packet):
          if len(packet) < 12:
              raise ValueError("short dns packet")
          qdcount = struct.unpack("!H", packet[4:6])[0]
          if qdcount != 1:
              raise ValueError("expected one question")
          name, end = decode_qname(packet, 12)
          if end + 4 > len(packet):
              raise ValueError("short question")
          qtype, qclass = struct.unpack("!HH", packet[end:end + 4])
          return name, qtype, qclass, packet[12:end + 4]

      def avahi_server():
          bus = dbus.SystemBus()
          obj = bus.get_object("org.freedesktop.Avahi", "/")
          return dbus.Interface(obj, "org.freedesktop.Avahi.Server")

      class Resolver:
          def __init__(self):
              self._lock = threading.Lock()
              self._server = avahi_server()

          def resolve_a(self, name):
              with self._lock:
                  try:
                      result = self._server.ResolveHostName(
                          AVAHI_IF_UNSPEC,
                          AVAHI_PROTO_UNSPEC,
                          name,
                          AVAHI_PROTO_INET,
                          AVAHI_LOOKUP_NO_FLAGS,
                      )
                  except dbus.DBusException:
                      return None
              return ipaddress.IPv4Address(str(result[4])).packed

      resolver = Resolver()

      def build_response(packet):
          txid = packet[:2]
          flags_in = struct.unpack("!H", packet[2:4])[0]
          rd = flags_in & 0x0100
          try:
              name, qtype, qclass, qwire = question(packet)
          except ValueError:
              return txid + struct.pack("!HHHHH", 0x8000 | rd | 1, 0, 0, 0, 0)

          flags = 0x8000 | 0x0400 | rd
          answer = b""
          ancount = 0
          rcode = 0

          if not name.endswith(".local") or qclass != CLASS_IN:
              rcode = 3
          elif qtype == TYPE_A:
              address = resolver.resolve_a(name)
              if address is None:
                  rcode = 3
              else:
                  answer = (
                      b"\xc0\x0c"
                      + struct.pack("!HHIH", TYPE_A, CLASS_IN, 120, len(address))
                      + address
                  )
                  ancount = 1

          header = txid + struct.pack("!HHHHH", flags | rcode, 1, ancount, 0, 0)
          return header + qwire + answer

      class UDPHandler(socketserver.BaseRequestHandler):
          def handle(self):
              response = build_response(self.request[0])
              self.request[1].sendto(response, self.client_address)

      class TCPHandler(socketserver.BaseRequestHandler):
          def handle(self):
              length = self.request.recv(2)
              if len(length) != 2:
                  return
              size = struct.unpack("!H", length)[0]
              data = b""
              while len(data) < size:
                  chunk = self.request.recv(size - len(data))
                  if not chunk:
                      return
                  data += chunk
              response = build_response(data)
              self.request.sendall(struct.pack("!H", len(response)) + response)

      class ThreadingUDPServer(socketserver.ThreadingMixIn, socketserver.UDPServer):
          allow_reuse_address = True
          daemon_threads = True

      class ThreadingTCPServer(socketserver.ThreadingMixIn, socketserver.TCPServer):
          allow_reuse_address = True
          daemon_threads = True

      def main():
          parser = argparse.ArgumentParser()
          parser.add_argument("--port", type=int, required=True)
          args = parser.parse_args()
          udp = ThreadingUDPServer(("127.0.0.1", args.port), UDPHandler)
          tcp = ThreadingTCPServer(("127.0.0.1", args.port), TCPHandler)
          threading.Thread(target=tcp.serve_forever, daemon=True).start()
          udp.serve_forever()

      if __name__ == "__main__":
          main()
    '';
  };
in
{
  config = lib.mkIf avahiEnabled {
    services.avahi = {
      enable = true;
      reflector = mdns.enable && mdns.reflector.enable;
      openFirewall = false;
      allowInterfaces = [ "external0" "eth1" ];
      nssmdns4 = false;
      nssmdns6 = false;
    };

    services.dnsmasq.settings.server = lib.mkIf bridgeEnabled (
      lib.mkAfter [ "/local/127.0.0.1#${toString bridgePort}" ]
    );

    systemd.services.d2b-mdns-local-resolver = lib.mkIf bridgeEnabled {
      description = "d2b net-VM .local DNS bridge to Avahi";
      wantedBy = [ "multi-user.target" ];
      after = [ "avahi-daemon.service" "dbus.service" ];
      requires = [ "avahi-daemon.service" "dbus.service" ];
      serviceConfig = {
        ExecStart = "${resolver}/bin/d2b-mdns-local-resolver --port ${toString bridgePort}";
        Restart = "on-failure";
        RestartSec = "1s";
        DynamicUser = true;
        NoNewPrivileges = true;
        PrivateNetwork = false;
        PrivateDevices = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectControlGroups = true;
        RestrictAddressFamilies = "AF_UNIX AF_INET";
        RestrictNamespaces = true;
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        SystemCallFilter = "@system-service";
        SystemCallArchitectures = "native";
        CapabilityBoundingSet = "";
        AmbientCapabilities = "";
      };
    };
  };
}
