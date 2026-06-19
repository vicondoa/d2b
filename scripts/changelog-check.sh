#!/usr/bin/env bash
# scripts/changelog-check.sh — fail-closed changelog policy gate for PR CI.

set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=${ROOT:-$(dirname "$HERE")}

cd "$ROOT"

base_ref=${GITHUB_BASE_REF:-main}
if git rev-parse --verify --quiet "origin/$base_ref" >/dev/null; then
  merge_base=$(git merge-base HEAD "origin/$base_ref")
else
  echo "WARN: origin/$base_ref not found; falling back to HEAD^ for diff scope" >&2
  merge_base=$(git rev-parse HEAD^)
fi

changed_files=$(git diff --name-only --diff-filter=ACMR "$merge_base..HEAD")
code_changed=0
changelog_changed=0

while IFS= read -r path; do
  [ -n "$path" ] || continue
  case "$path" in
    CHANGELOG.md)
      changelog_changed=1
      ;;
    *.rs|*.nix|Cargo.toml|*/Cargo.toml|Cargo.lock|*/Cargo.lock)
      code_changed=1
      ;;
  esac
done <<<"$changed_files"

if [ "$code_changed" -eq 1 ] && [ "$changelog_changed" -ne 1 ]; then
  echo "FAIL: code changed ($merge_base..HEAD) but CHANGELOG.md was not updated." >&2
  echo "      Add release notes under '## [Unreleased]' before merging." >&2
  exit 1
fi

perl - <<'PERL'
use strict;
use warnings;
use Time::Piece;

open my $fh, '<', 'CHANGELOG.md' or die "open CHANGELOG.md: $!";
my @lines = <$fh>;
chomp @lines;

my @errors;
my @unreleased_lines;
my @release_headers;
my %seen_versions;

for my $idx (0 .. $#lines) {
    my $line = $lines[$idx];
    next unless $line =~ /^## /;

    if ($line !~ /^## \[([^\]]+)\](?: - (\d{4}-\d{2}-\d{2}))?$/) {
        push @errors,
          "line " . ($idx + 1) . ": invalid release header '$line' "
          . "(expected '## [Unreleased]' or '## [X.Y.Z] - YYYY-MM-DD')";
        next;
    }

    my ($label, $date_text) = ($1, $2);

    if ($label eq 'Unreleased') {
        push @errors, "line " . ($idx + 1) . ": 'Unreleased' must not carry a release date"
          if defined $date_text;
        push @unreleased_lines, $idx + 1;
        next;
    }

    if (!defined $date_text) {
        push @errors, "line " . ($idx + 1) . ": release header missing ISO date";
        next;
    }

    if ($label !~ /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/) {
        push @errors, "line " . ($idx + 1) . ": version '$label' is not valid semver (X.Y.Z)";
        next;
    }

    my $parsed_date = eval { Time::Piece->strptime($date_text, '%Y-%m-%d')->strftime('%Y-%m-%d') };
    if (!defined $parsed_date || $parsed_date ne $date_text) {
        push @errors, "line " . ($idx + 1) . ": date '$date_text' is not a valid ISO 8601 calendar date";
        next;
    }

    if (exists $seen_versions{$label}) {
        push @errors,
          "line " . ($idx + 1) . ": duplicate release header for '$label' "
          . "(already seen on line $seen_versions{$label})";
        next;
    }

    $seen_versions{$label} = $idx + 1;
    my @parts = split /\./, $label;
    push @release_headers, [ $idx + 1, $label, \@parts ];
}

if (!@unreleased_lines) {
    push @errors, "missing required '## [Unreleased]' section";
} elsif (@unreleased_lines > 1) {
    push @errors, "duplicate '## [Unreleased]' headers at lines " . join(', ', @unreleased_lines);
}

if (@unreleased_lines && @release_headers && $unreleased_lines[0] > $release_headers[0]->[0]) {
    push @errors, "'## [Unreleased]' must appear before the first numbered release";
}

for my $i (1 .. $#release_headers) {
    my ($prev_line, $prev_label, $prev_parts) = @{ $release_headers[$i - 1] };
    my ($line, $label, $parts) = @{ $release_headers[$i] };
    if (
        $parts->[0] > $prev_parts->[0]
        || ($parts->[0] == $prev_parts->[0] && $parts->[1] > $prev_parts->[1])
        || ($parts->[0] == $prev_parts->[0] && $parts->[1] == $prev_parts->[1] && $parts->[2] >= $prev_parts->[2])
    ) {
        push @errors,
          "line $line: release $label is out of order; expected descending versions below $prev_label";
    }
}

if (@errors) {
    warn "FAIL: CHANGELOG.md validation failed:\n";
    warn "  - $_\n" for @errors;
    exit 1;
}

print "PASS: CHANGELOG.md policy checks passed.\n";
PERL
