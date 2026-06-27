{ pkgs }:

pkgs.runCommand "d2b-harness-ubuntu-skeleton" {
  src = ./.;
  nativeBuildInputs = [ pkgs.bash pkgs.jq ];
} ''
  mkdir -p $out/harness/ubuntu
  cp $src/*.sh $src/*.json $src/README.md $out/harness/ubuntu/
  chmod +x $out/harness/ubuntu/*.sh

  # Smoke-test that the stub is well-formed JSON.
  bash $out/harness/ubuntu/host-check-stub.sh > $out/host-check-output.json
  jq -e . $out/host-check-output.json > /dev/null
''
