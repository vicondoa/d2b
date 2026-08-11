{ pkgs }:

let
  # This is the exact github.com/steveyegge/beads pseudo-version selected by
  # Gas City's pinned go.mod, including the conditional-write implementation.
  revision = "bf97b73749ac3ef2fca2365b54537ac041ad4293";
in
pkgs.buildGoModule rec {
  pname = "beads";
  version = "1.1.1-unstable-20260805";

  src = pkgs.fetchFromGitHub {
    owner = "steveyegge";
    repo = "beads";
    rev = revision;
    hash = "sha256-qFOqdzcfIHcKmYB5+WMpoUGWfFGyzt4kLGnleWnGG8s=";
  };

  subPackages = [ "cmd/bd" ];
  tags = [ "gms_pure_go" ];
  vendorHash = "sha256-CW+ba1KYpmBZ1UXHCr2B/EHOr8LDi494BuEDGHABLbk=";
  proxyVendor = true;
  doCheck = false;

  # Embedded Dolt requires CGO.  The gms_pure_go tag avoids the optional ICU
  # regex path while retaining the storage backend used by Gas City.
  env.CGO_ENABLED = "1";
  env.GOTOOLCHAIN = "local";

  ldflags = [
    "-X main.Version=${version}"
    "-X main.Build=${revision}"
    "-X main.Commit=${revision}"
  ];

  meta = {
    description = "Issue tracker designed for AI-supervised coding workflows";
    homepage = "https://github.com/steveyegge/beads";
    license = pkgs.lib.licenses.mit;
    mainProgram = "bd";
  };

  passthru = {
    inherit revision;
    sourceRepository = "steveyegge/beads";
  };
}
