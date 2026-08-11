{ pkgs
, source ? null
}:

let
  revision = "6e0399fb970190a35c3e3d5d272a02becec55ffe";
  commitDate = "2026-08-10T21:41:02Z";
  fetchedSource = pkgs.fetchFromGitHub {
    owner = "gastownhall";
    repo = "gascity";
    rev = revision;
    hash = "sha256-Gdrk4f1ViW1abdCqL3Jm2vvoyHy3Sj6pe6BNeKVUjgY=";
  };
in
pkgs.buildGoModule rec {
  pname = "gascity";
  version = "0-unstable-2026-08-10";

  # The flake supplies the same commit-pinned source input used by the pack
  # closure.  Keep the fetch fallback for direct package consumers.
  src = if source == null then fetchedSource else source;

  subPackages = [ "cmd/gc" ];
  vendorHash = "sha256-05Ch0dn0W8OKZaGFq04VQS7QzLkgo//chz0WBjjefrQ=";
  proxyVendor = true;

  # Gas City embeds the Dolt-backed beads provider.  Keep CGO enabled while
  # selecting beads' pure-Go regex implementation so the binary retains
  # embedded-Dolt support without adding an ICU runtime dependency.
  tags = [ "gms_pure_go" ];
  env.CGO_ENABLED = "1";
  env.GOTOOLCHAIN = "local";

  nativeBuildInputs = [ pkgs.pkg-config ];
  buildInputs = [ pkgs.icu ];

  ldflags = [
    "-X main.version=${version}"
    "-X main.commit=${revision}"
    "-X main.date=${commitDate}"
  ];

  # The source archive has no VCS directory.  Explicitly disable build
  # stamping so the injected revision is the only version identity.
  preBuild = ''
    export GOFLAGS="''${GOFLAGS:-} -buildvcs=false"
  '';

  # Keep the upstream package test surface enabled.  No external-service
  # exclusion is applied here; a sandbox-only exception must be justified by
  # a later, focused failure rather than weakening the default check.
  doCheck = true;

  meta = {
    description = "Gas City supervisor and workflow engine";
    homepage = "https://github.com/gastownhall/gascity";
    license = pkgs.lib.licenses.mit;
    mainProgram = "gc";
  };

  passthru = {
    inherit revision;
  };
}
