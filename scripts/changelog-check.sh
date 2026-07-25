#!/usr/bin/env bash
# scripts/changelog-check.sh — fail-closed changelog policy gate for PR CI.
#
# A code change ships release notes either as a CHANGELOG.md entry or as a
# changelog.d/ fragment (see changelog.d/README.md). Neither is a failure.
# Every fragment present is structurally validated here, so a malformed
# fragment fails on the PR that introduced it instead of at fold time.

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
    changelog.d/README.md)
      # Documents the fragment mechanism; carries no release notes.
      ;;
    changelog.d/*.md)
      changelog_changed=1
      ;;
    *.rs|*.nix|Cargo.toml|*/Cargo.toml|Cargo.lock|*/Cargo.lock)
      code_changed=1
      ;;
  esac
done <<<"$changed_files"

if [ "$code_changed" -eq 1 ] && [ "$changelog_changed" -ne 1 ]; then
  echo "FAIL: code changed ($merge_base..HEAD) but no release notes were added." >&2
  echo "      Add an entry under '## [Unreleased]' in CHANGELOG.md, or add a" >&2
  echo "      changelog.d/<branch>.md fragment (see changelog.d/README.md)." >&2
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

perl - <<'PERL'
use strict;
use warnings;

# Structural validation of changelog.d/ fragments. Mirrors the fail-closed
# rules the assembler (`cargo xtask changelog-fold`) enforces, so a fragment
# that would abort the fold fails on the pull request instead.
my @sections = qw(Added Changed Deprecated Removed Fixed Security);
my %known = map { $_ => 1 } @sections;

my @fragments = sort grep { $_ ne 'changelog.d/README.md' } glob('changelog.d/*.md');
my @errors;

for my $path (@fragments) {
    open my $fh, '<', $path or die "open $path: $!";
    my @lines = <$fh>;
    close $fh;
    chomp @lines;

    my $current;
    my $swallow = 0;
    my $saw_heading = 0;
    my $before = scalar @errors;
    my %seen;
    my @order;
    my %entries;
    my $stray_reported = 0;

    for my $idx (0 .. $#lines) {
        my $line = $lines[$idx];
        $line =~ s/\s+$//;
        my $lineno = $idx + 1;

        if ($line =~ /^#/) {
            if ($line !~ /^### (.+)$/) {
                push @errors, "$path:$lineno: expected a '### <Section>' heading, found '$line'";
                next;
            }
            my $title = $1;
            $title =~ s/^\s+|\s+$//g;
            $saw_heading = 1;
            if (!$known{$title}) {
                push @errors,
                  "$path:$lineno: unknown section '$title'; expected one of "
                  . join(', ', @sections);
                ($current, $swallow) = (undef, 1);
                next;
            }
            if (exists $seen{$title}) {
                push @errors,
                  "$path:$lineno: duplicate '### $title' heading (first seen on line $seen{$title})";
                ($current, $swallow) = (undef, 1);
                next;
            }
            $seen{$title} = $lineno;
            push @order, $title;
            $entries{$title} = [];
            ($current, $swallow) = ($title, 0);
            next;
        }

        if (defined $current) {
            push @{ $entries{$current} }, $line;
        } elsif (!$swallow && $line =~ /\S/ && !$stray_reported) {
            $stray_reported = 1;
            push @errors, "$path:$lineno: content before the first '### <Section>' heading";
        }
    }

    if (!$saw_heading && scalar(@errors) == $before) {
        push @errors,
          "$path: no '### <Section>' heading; a fragment must carry at least one entry";
    }

    for my $title (@order) {
        my @body = @{ $entries{$title} };
        shift @body while @body && $body[0] !~ /\S/;
        pop @body while @body && $body[-1] !~ /\S/;
        if (!@body) {
            push @errors, "$path:$seen{$title}: section '### $title' has no entries";
            next;
        }
        if ($body[0] !~ /^[-*] /) {
            push @errors, "$path:$seen{$title}: section '### $title' must start with a '- ' bullet";
        }
    }
}

if (@errors) {
    warn "FAIL: changelog.d/ fragment validation failed:\n";
    warn "  - $_\n" for @errors;
    warn "  See changelog.d/README.md for the fragment format.\n";
    exit 1;
}

printf "PASS: %d changelog.d/ fragment(s) validated.\n", scalar @fragments;
PERL
