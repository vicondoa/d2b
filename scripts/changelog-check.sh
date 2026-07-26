#!/usr/bin/env bash
# scripts/changelog-check.sh - fail-closed changelog policy gate for PR CI.
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

# Deletions count as changes: removing a Rust module, a shell script, or a
# Makefile rule is a behaviour change that needs a release note just as much as
# an edit. `--diff-filter=d` (lowercase) keeps everything EXCEPT entries that
# are only broken-pair noise, so additions, copies, modifications, renames,
# type changes, and deletions are all considered.
changed_files=$(git diff --name-only --diff-filter=ACMRTD "$merge_base..HEAD")
code_changed=0
changelog_changed=0

# A change needs a release note unless it touches only prose or a data/binary
# asset. Rather than enumerate every executable extension - which silently
# missed whole surfaces such as `.patch` and `.proto` - the default is "code",
# and only an explicit prose/data allowlist is exempt. A deleted module, a
# Makefile behaviour change, a patch, or a protocol definition all require a
# note; Markdown, LICENSE/COPYING text, and binary assets do not.
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
    *.md|*.markdown|*.txt|*.rst|*.adoc \
    |LICENSE|LICENSE.*|COPYING|COPYING.* \
    |*.png|*.jpg|*.jpeg|*.gif|*.svg|*.webp|*.ico|*.pdf \
    |*.woff|*.woff2|*.ttf|*.otf)
      # Prose, documentation, or a data/binary asset: exempt from the note
      # requirement.
      ;;
    *)
      # Every other path is an executable or configuration surface - Rust, Nix,
      # shell, Make, TOML/JSON/YAML, a `.patch`, a `.proto`, or any unrecognized
      # extension. Fail closed: an unknown surface needs a note rather than
      # slipping through.
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
use Encode ();

# Structural validation of changelog.d/ fragments. This is the SECOND parser of
# the fragment format; the canonical one is the Rust assembler
# (`cargo xtask changelog-fold --check`, packages/xtask/src/changelog.rs). Two
# parsers exist on purpose: the changelog CI job is intentionally toolchain-free
# (git + bash + perl only) so it stays fast and runnable on every pull request
# without provisioning a Rust toolchain, while the Rust parser is what actually
# rewrites CHANGELOG.md at merge time. Because a fragment that this gate accepts
# but the Rust fold rejects would break the merge, the two MUST agree on
# discovery, file types, encoding, and structure. That equivalence is pinned by
# the cross-language parity tests in
# packages/d2b-contract-tests/tests/policy_changelog_gate.rs and the Rust-side
# discovery tests in packages/xtask/src/changelog.rs. Keep all three in sync.
my @sections = qw(Added Changed Deprecated Removed Fixed Security);
my %known = map { $_ => 1 } @sections;

my @errors;

# Discovery mirrors the Rust `load_fragments`: read every directory entry rather
# than globbing '*.md', skip only README.md, and reject a non-regular file (a
# symlink, a directory, a fifo) BEFORE the name check, then reject any entry
# whose name does not end in '.md'. A bare glob would silently ignore a symlink
# fragment or a mis-named file that the Rust fold refuses.
my @fragments;
if (opendir(my $dh, 'changelog.d')) {
    my @names = sort grep { $_ ne '.' && $_ ne '..' && $_ ne 'README.md' } readdir($dh);
    closedir $dh;
    for my $name (@names) {
        my $path = "changelog.d/$name";
        if (-l $path || !-f _) {
            push @errors,
              "$path: not a regular file; the fragment directory holds one '.md' file per branch";
            next;
        }
        if ($name !~ /\.md\z/) {
            push @errors, "$path: fragments must be named '<branch>.md'";
            next;
        }
        push @fragments, $path;
    }
}

for my $path (@fragments) {
    open my $fh, '<:raw', $path or die "open $path: $!";
    my $bytes = do { local $/; <$fh> };
    close $fh;
    $bytes = '' unless defined $bytes;

    # The Rust fold reads fragments with `read_to_string`, which fails on
    # invalid UTF-8. Reject the same inputs here instead of parsing mojibake.
    my $text = eval { Encode::decode('UTF-8', $bytes, Encode::FB_CROAK | Encode::LEAVE_SRC) };
    if (!defined $text) {
        push @errors, "$path: is not valid UTF-8";
        next;
    }
    my @lines = split /\n/, $text, -1;
    pop @lines if @lines && $lines[-1] eq '';
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
        if ($body[0] !~ /^- /) {
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
