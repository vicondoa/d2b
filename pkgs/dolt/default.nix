{ pkgs }:

pkgs.buildGoModule rec {
  pname = "dolt";
  version = "2.1.7";

  src = pkgs.fetchFromGitHub {
    owner = "dolthub";
    repo = "dolt";
    tag = "v${version}";
    hash = "sha256-ZMK0XiVaSZObr23mQ3OKA5t8wDV8l8SN2Rhh3VjJo1w=";
  };

  modRoot = "./go";
  subPackages = [ "cmd/dolt" ];
  vendorHash = "sha256-l0SHq3WTajqGTE5sV6RgLgVLS+i7AhAxfJkJmAvv2ok=";
  proxyVendor = true;
  doCheck = false;

  # Dolt's embedded SQL engine uses CGO-backed ICU support.
  env.CGO_ENABLED = "1";
  env.GOTOOLCHAIN = "local";
  buildInputs = [ pkgs.icu ];

  meta = {
    description = "Relational database with version control and a Git-like CLI";
    homepage = "https://www.dolthub.com/";
    license = pkgs.lib.licenses.asl20;
    mainProgram = "dolt";
  };

  passthru = {
    sourceRepository = "dolthub/dolt";
  };
}
