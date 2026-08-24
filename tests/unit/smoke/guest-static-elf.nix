{ pkgs, flake, system }:

pkgs.runCommand "d2b-guest-static-elf" {
  nativeBuildInputs = [ pkgs.pkgsStatic.binutils ];
} ''
  readelf=${pkgs.pkgsStatic.binutils.bintools}/bin/readelf
  for bin in \
    ${flake.packages.${system}.d2bd-guest-static}/bin/d2bd \
    ${flake.packages.${system}.d2b-broker-guest-static}/bin/d2b-broker \
    ${flake.packages.${system}.d2b-guest-shell-runner-static}/bin/d2b-guest-shell-runner
  do
    test -x "$bin"
    name="$(basename "$bin")"
    "$readelf" -h "$bin" >/dev/null
    "$readelf" -l "$bin" > "$TMPDIR/$name.program-headers"
    if grep -q 'Requesting program interpreter' "$TMPDIR/$name.program-headers"; then
      echo "$bin: unexpected ELF interpreter" >&2
      cat "$TMPDIR/$name.program-headers" >&2
      exit 1
    fi
    if "$readelf" -d "$bin" > "$TMPDIR/$name.dynamic" 2> "$TMPDIR/$name.dynamic.err"; then
      if grep -q '(NEEDED)' "$TMPDIR/$name.dynamic"; then
        echo "$bin: unexpected dynamic dependency" >&2
        cat "$TMPDIR/$name.dynamic" >&2
        exit 1
      fi
    elif ! grep -qi 'no dynamic section' "$TMPDIR/$name.dynamic.err"; then
      echo "$bin: readelf -d failed unexpectedly" >&2
      cat "$TMPDIR/$name.dynamic.err" >&2
      exit 1
    fi
  done
  mkdir -p "$out"
  echo ok > "$out/guest-static-elf"
''
