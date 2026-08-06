#!/usr/bin/env perl
use strict;
use warnings;
use FindBin qw($Bin);
use File::Spec;

my $tool_path = 'specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl';
my $tasks_path = 'specs/003-adr052-bazel-rust/tasks.md';
my $rerun =
    "perl $tool_path --self-test && perl $tool_path";
my $fixture_root =
    'specs/003-adr052-bazel-rust/tools/validator-fixtures/';

my %code = (
    read       => 'D2B-SPEC003-PLAN-READ',
    section    => 'D2B-SPEC003-PLAN-SECTION',
    census     => 'D2B-SPEC003-PLAN-CENSUS',
    parse      => 'D2B-SPEC003-PLAN-PARSE',
    task_id    => 'D2B-SPEC003-PLAN-TASK-ID',
    owner      => 'D2B-SPEC003-PLAN-OWNER',
    ownership  => 'D2B-SPEC003-PLAN-OWNERSHIP',
    dependency => 'D2B-SPEC003-PLAN-DEPENDENCY',
    adjacency  => 'D2B-SPEC003-PLAN-ADJACENCY',
    cycle      => 'D2B-SPEC003-PLAN-CYCLE',
    conflict   => 'D2B-SPEC003-PLAN-CONFLICT',
);

my %remedy = (
    read =>
        'Restore the repository-relative source and make it readable.',
    section =>
        'Create or retain exactly one canonical ## Dependency graph section.',
    census =>
        'Declare one independent task-ID census with exactly the canonical task IDs.',
    parse =>
        'Rewrite every unchecked task record with owner, files, and depends fields in canonical order.',
    task_id =>
        'Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.',
    owner =>
        'Replace the owner with one literal lowercase scope identifier.',
    ownership =>
        'Replace ownership with unique literal normalized repository-relative paths or none.',
    dependency =>
        'Replace dependencies with unique existing earlier TNNN IDs or none.',
    adjacency =>
        'Make the dependency-graph row exactly equal the task depends field.',
    cycle =>
        'Remove a dependency edge so the task graph is acyclic.',
    conflict =>
        'Order the conflicting tasks by dependency or give them disjoint owned paths.',
);

my %specific_remedy = (
    'parse:checkbox-outside-task-section' =>
        'Move every unchecked task record before the canonical ## Dependency graph section.',
    'parse:no-tasks' =>
        'Provide at least one canonical unchecked task record before the dependency graph.',
    'parse:unsupported-arguments' =>
        'Invoke the validator with no argument or with only --self-test.',
    'task_id:duplicate-task' =>
        'Assign every task one unique canonical TNNN identifier and update its dependency row.',
);

my %known_reason = (
    read => { map { $_ => 1 } qw(unreadable) },
    section => { map { $_ => 1 } qw(missing duplicate) },
    census => {
        map { $_ => 1 }
            qw(missing-declaration duplicate-declaration malformed-declaration mismatch)
    },
    parse => {
        map { $_ => 1 }
            qw(
                checkbox-outside-task-section
                malformed-record
                repeated-metadata-field
                record-census
                no-tasks
                unexpected-positive-failure
                expected-failure-missing
                diagnostic-contract
                unsupported-arguments
                self-test-contract
            )
    },
    task_id => {
        map { $_ => 1 }
            qw(malformed-header noncanonical-task-form duplicate-task)
    },
    owner => { map { $_ => 1 } qw(malformed) },
    ownership => {
        map { $_ => 1 }
            qw(
                empty-path
                malformed-quoting
                absolute-path
                repeated-separator
                dot-component
                unresolved-expression
                duplicate-path
            )
    },
    dependency => {
        map { $_ => 1 }
            qw(malformed duplicate missing not-earlier)
    },
    adjacency => {
        map { $_ => 1 }
            qw(
                duplicate-row
                malformed
                duplicate-dependency
                missing-row
                extra-row
                mismatch
            )
    },
    cycle => { map { $_ => 1 } qw(detected) },
    conflict => { map { $_ => 1 } qw(detected) },
);

my @fixture_names = qw(
    positive.md
    malformed-header.md
    star-list.md
    plus-list.md
    ordered-dot-list.md
    ordered-paren-list.md
    ordered-list.md
    indentation.md
    tab-indentation.md
    blockquote.md
    nested-blockquote.md
    dot-alias.md
    dotdot-alias.md
    absolute-path.md
    repeated-separator.md
    malformed-quoting.md
    duplicate-path.md
    empty-path.md
    dynamic-ownership.md
    malformed-owner.md
    parser-omission.md
    repeated-metadata-field.md
    task-after-graph.md
    empty.md
    whole-task-omission.md
    census-missing.md
    census-duplicate.md
    census-malformed.md
    census-duplicate-id.md
    task-duplicate.md
    dependency-failure.md
    dependency-adjacency-mismatch.md
    dependency-malformed.md
    dependency-duplicate.md
    dependency-not-earlier.md
    adjacency-missing-row.md
    adjacency-extra-row.md
    adjacency-duplicate-row.md
    adjacency-malformed.md
    adjacency-duplicate-dependency.md
    adjacency-mismatch.md
    cycle.md
    concurrent-conflict.md
    section-missing.md
    section-duplicate.md
);
my %fixture_source = map { $_ => $fixture_root . $_ } @fixture_names;

sub read_repository_file {
    my ($path) = @_;
    open my $fh, '<', $path or return;
    local $/;
    my $text = <$fh>;
    close $fh or return;
    return $text;
}

sub trim {
    my ($value) = @_;
    $value =~ s/^\s+|\s+$//g;
    return $value;
}

sub error_record {
    my ($kind, $reason) = @_;
    return {
        kind   => $kind,
        reason => $reason,
    };
}

sub diagnostic_source {
    my ($source_key) = @_;
    return $tasks_path if !defined($source_key) || $source_key eq 'tasks';
    return $fixture_source{$source_key} if exists $fixture_source{$source_key};
    return $tasks_path;
}

sub render_error {
    my ($error, $source_key) = @_;
    my $kind = $error->{kind};
    my $error_code = $code{$kind} // $code{parse};
    my $reason =
        exists($known_reason{$kind}) && exists($known_reason{$kind}->{$error->{reason}})
        ? $error->{reason}
        : 'validation-failure';
    my $correction =
        $specific_remedy{"$kind:$reason"} // $remedy{$kind}
        // 'Correct the repository-relative planning artifact.';
    return "FAIL $error_code source=" . diagnostic_source($source_key)
        . " reason=$reason\n"
        . "REMEDY $error_code $correction\n"
        . "RERUN $error_code $rerun\n";
}

sub parse_owned_path {
    my ($raw, $files, $errors) = @_;
    my $path = trim($raw);

    if ($path eq '' || $path eq 'none') {
        push @$errors, error_record('ownership', 'empty-path');
        return;
    }

    my $backticks = () = $path =~ /`/g;
    if ($backticks == 2 && $path =~ /\A`([^`]*)`\z/) {
        $path = $1;
    } elsif ($backticks != 0 || $path =~ /['"]/) {
        push @$errors, error_record('ownership', 'malformed-quoting');
        return;
    }

    if ($path =~ m{\A/}) {
        push @$errors, error_record('ownership', 'absolute-path');
        return;
    }
    if ($path =~ m{//}) {
        push @$errors, error_record('ownership', 'repeated-separator');
        return;
    }

    my @components = split m{/}, $path, -1;
    if (grep { $_ eq '.' || $_ eq '..' } @components) {
        push @$errors, error_record('ownership', 'dot-component');
        return;
    }

    if (
        $path =~ /\b(?:and\s+every|listed\s+in|generated\s+paths?)\b/i
        || $path =~ /[*?\[\]{}]/
        || $path !~ m{\A[A-Za-z0-9_.@+-]+(?:/[A-Za-z0-9_.@+-]+)*\z}
    ) {
        push @$errors, error_record('ownership', 'unresolved-expression');
        return;
    }

    if ($files->{$path}++) {
        push @$errors, error_record('ownership', 'duplicate-path');
    }
}

sub census_task_forms {
    my ($text) = @_;
    my @forms;
    my $graph_seen = 0;
    for my $line (split /\n/, $text, -1) {
        $line =~ s/\r\z//;
        if ($line =~ /\A## Dependency graph[ \t]*\z/) {
            $graph_seen = 1;
        }
        next unless $line =~
            /\A[ \t]*(?:>[ \t]*)*(?:[-*+]|\d+[.)]?)[ \t]+\[[ \t]*\]/;

        my $canonical =
            $line =~ /\A- \[ \] T[0-9]{3}(?=\s|\z)/;
        my $canonical_marker =
            $line =~ /\A- \[ \] /;
        push @forms, {
            canonical        => $canonical,
            canonical_marker => $canonical_marker,
            after_graph      => $graph_seen,
        };
    }
    return \@forms;
}

sub parse_census_declaration {
    my ($text) = @_;
    my $begin = '<!-- D2B-SPEC003-PLAN-TASK-CENSUS:BEGIN -->';
    my $end = '<!-- D2B-SPEC003-PLAN-TASK-CENSUS:END -->';
    my @begins = $text =~ /^\Q$begin\E[ \t]*\r?$/mg;
    my @ends = $text =~ /^\Q$end\E[ \t]*\r?$/mg;
    my @mentions =
        $text =~ /^.*D2B-SPEC003-PLAN-TASK-CENSUS.*$/mg;

    if (@mentions > @begins + @ends) {
        return (undef, error_record('census', 'malformed-declaration'));
    }

    if (!@begins && !@ends) {
        return (undef, error_record('census', 'missing-declaration'));
    }
    if (@begins > 1 || @ends > 1) {
        return (undef, error_record('census', 'duplicate-declaration'));
    }
    if (@begins != 1 || @ends != 1) {
        return (undef, error_record('census', 'malformed-declaration'));
    }

    my @blocks;
    while (
        $text =~
        /^\Q$begin\E[ \t]*\r?\n(.*?)^\Q$end\E[ \t]*\r?$/msg
    ) {
        push @blocks, $1;
    }
    if (@blocks != 1) {
        return (undef, error_record('census', 'malformed-declaration'));
    }

    my $body = $blocks[0];
    $body =~ s/\r\n/\n/g;
    $body =~ s/\r/\n/g;
    my @ids = split /\n/, $body, -1;
    pop @ids if @ids && $ids[-1] eq '';
    for my $id (@ids) {
        return (undef, error_record('census', 'malformed-declaration'))
            unless $id =~ /\AT[0-9]{3}\z/;
    }
    my %seen;
    for my $id (@ids) {
        return (undef, error_record('census', 'malformed-declaration'))
            if $seen{$id}++;
    }
    return (\@ids, undef);
}

sub same_id_census {
    my ($left, $right) = @_;
    return join("\n", sort @$left) eq join("\n", sort @$right);
}

sub validate_text {
    my ($text) = @_;
    my @errors;

    # This is deliberately the first validation phase. The broad census
    # sees every unchecked Markdown list form before canonical parsing can
    # ignore a marker, indentation, blockquote, or ordered-list variant.
    my $forms = census_task_forms($text);
    my @canonical_ids;
    for my $form (@$forms) {
        if (!$form->{canonical}) {
            push @errors,
                error_record(
                    'task_id',
                    $form->{canonical_marker}
                    ? 'malformed-header'
                    : 'noncanonical-task-form'
                );
            next;
        }
        if ($form->{after_graph}) {
            push @errors,
                error_record('parse', 'checkbox-outside-task-section');
            next;
        }
        push @canonical_ids, 1;
    }
    return (\@errors, 0) if @errors;

    my ($expected_ids, $census_error) = parse_census_declaration($text);
    return ([$census_error], 0) if defined $census_error;

    my @sections = $text =~ /^## Dependency graph[ \t]*$/mg;
    if (@sections != 1) {
        my $reason = @sections ? 'duplicate' : 'missing';
        return ([error_record('section', $reason)], 0);
    }
    my ($task_text, $adjacency_text) =
        split /^## Dependency graph[ \t]*$/m, $text, 2;
    $task_text //= '';
    $adjacency_text //= '';

    my @records =
        $task_text =~
        /(^- \[ \] T[0-9]{3}(?=\s|\z).*?)(?=^- \[ \] T[0-9]{3}(?=\s|\z)|\z)/msg;
    my (@tasks, %seen);
    for my $record (@records) {
        if (
            $record !~
            /\A-\x20\[\x20\]\x20(T[0-9]{3})\s+
              \[owner:\s*([^\]\r\n]+)\]\s+
              \[files:\s*(.*?)\]\s+
              \[depends:\s*([^\]\r\n]+)\](?=\s|\z)(.*)\z/sx
        ) {
            push @errors, error_record('parse', 'malformed-record');
            next;
        }

        my ($id, $owner, $raw_files, $raw_depends, $remainder) =
            ($1, $2, $3, $4, $5);
        if ($remainder =~ /\[\s*(?:owner|files|depends)\s*:/i) {
            push @errors,
                error_record('parse', 'repeated-metadata-field');
            next;
        }
        if ($seen{$id}++) {
            push @errors, error_record('task_id', 'duplicate-task');
        }

        $owner = trim($owner);
        if ($owner !~ /\A[a-z0-9-]+\z/) {
            push @errors, error_record('owner', 'malformed');
        }

        my %files;
        my @file_items = split /,/, $raw_files, -1;
        my $files_none = @file_items == 1 && trim($file_items[0]) eq 'none';
        if ($files_none) {
            @file_items = ();
        }
        if (!$files_none && !@file_items) {
            push @errors, error_record('ownership', 'empty-path');
        }
        parse_owned_path($_, \%files, \@errors) for @file_items;

        my (@depends, %depends_seen);
        if (trim($raw_depends) ne 'none') {
            for my $raw (split /,/, $raw_depends, -1) {
                my $dependency = trim($raw);
                if ($dependency !~ /\AT[0-9]{3}\z/) {
                    push @errors, error_record('dependency', 'malformed');
                    next;
                }
                if ($depends_seen{$dependency}++) {
                    push @errors, error_record('dependency', 'duplicate');
                    next;
                }
                push @depends, $dependency;
            }
        }

        push @tasks, {
            id      => $id,
            owner   => $owner,
            files   => \%files,
            depends => \@depends,
            order   => scalar @tasks,
        };
    }

    if (@records != @canonical_ids) {
        push @errors, error_record('parse', 'record-census');
    }
    return (\@errors, scalar @tasks) if @errors;
    if (!@tasks) {
        push @errors, error_record('parse', 'no-tasks');
        return (\@errors, 0);
    }
    my @actual_ids = map { $_->{id} } @tasks;
    if (!same_id_census(\@actual_ids, $expected_ids)) {
        push @errors, error_record('census', 'mismatch');
        return (\@errors, scalar @tasks);
    }

    my %by_id = map { $_->{id} => $_ } @tasks;
    my %graph = map {
        $_->{id} => { map { $_ => 1 } @{$_->{depends}} }
    } @tasks;

    my (%visiting, %visited);
    my $visit;
    $visit = sub {
        my ($id) = @_;
        return if $visited{$id} || !exists $graph{$id};
        if ($visiting{$id}) {
            push @errors, error_record('cycle', 'detected');
            return;
        }
        $visiting{$id} = 1;
        $visit->($_) for sort keys %{$graph{$id}};
        delete $visiting{$id};
        $visited{$id} = 1;
    };
    $visit->($_) for sort keys %graph;
    return (\@errors, scalar @tasks) if @errors;

    for my $task (@tasks) {
        for my $dependency (@{$task->{depends}}) {
            if (!exists $by_id{$dependency}) {
                push @errors, error_record('dependency', 'missing');
                next;
            }
            if ($by_id{$dependency}->{order} >= $task->{order}) {
                push @errors, error_record('dependency', 'not-earlier');
            }
        }
    }
    return (\@errors, scalar @tasks) if @errors;

    my (%rows, %row_seen);
    while ($adjacency_text =~ /^(T[0-9]{3})\s+<-\s+(.+)$/mg) {
        my ($id, $raw) = ($1, $2);
        if ($row_seen{$id}++) {
            push @errors, error_record('adjacency', 'duplicate-row');
        }
        my (%dependencies, %adjacency_seen);
        if (trim($raw) ne 'none') {
            for my $value (split /,/, $raw, -1) {
                my $dependency = trim($value);
                if ($dependency !~ /\AT[0-9]{3}\z/) {
                    push @errors, error_record('adjacency', 'malformed');
                    next;
                }
                if ($adjacency_seen{$dependency}++) {
                    push @errors,
                        error_record('adjacency', 'duplicate-dependency');
                    next;
                }
                $dependencies{$dependency} = 1;
            }
        }
        $rows{$id} = \%dependencies;
    }
    for my $id (sort keys %by_id) {
        if (!exists $rows{$id}) {
            push @errors, error_record('adjacency', 'missing-row');
            next;
        }
        my $inline = join ',', sort keys %{$graph{$id}};
        my $row = join ',', sort keys %{$rows{$id}};
        if ($inline ne $row) {
            push @errors, error_record('adjacency', 'mismatch');
        }
    }
    for my $id (sort keys %rows) {
        if (!exists $by_id{$id}) {
            push @errors, error_record('adjacency', 'extra-row');
        }
    }
    return (\@errors, scalar @tasks) if @errors;

    my %ancestors;
    for my $id (keys %graph) {
        my @stack = keys %{$graph{$id}};
        my %found;
        while (@stack) {
            my $dependency = pop @stack;
            next if $found{$dependency}++;
            push @stack, keys %{$graph{$dependency}};
        }
        $ancestors{$id} = \%found;
    }

    for my $left_index (0 .. $#tasks) {
        my $left = $tasks[$left_index];
        next unless keys %{$left->{files}};
        for my $right_index ($left_index + 1 .. $#tasks) {
            my $right = $tasks[$right_index];
            next unless keys %{$right->{files}};
            next if $left->{owner} eq $right->{owner};
            next if $ancestors{$left->{id}}->{$right->{id}};
            next if $ancestors{$right->{id}}->{$left->{id}};
            my @overlap =
                grep { $right->{files}->{$_} } sort keys %{$left->{files}};
            if (@overlap) {
                push @errors, error_record('conflict', 'detected');
            }
        }
    }

    return (\@errors, scalar @tasks);
}

my %expected_stderr = (
    'malformed-header.md' => q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tools/validator-fixtures/malformed-header.md reason=malformed-header
REMEDY D2B-SPEC003-PLAN-TASK-ID Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'star-list.md' => q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tools/validator-fixtures/star-list.md reason=noncanonical-task-form
REMEDY D2B-SPEC003-PLAN-TASK-ID Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'plus-list.md' => q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tools/validator-fixtures/plus-list.md reason=noncanonical-task-form
REMEDY D2B-SPEC003-PLAN-TASK-ID Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'ordered-dot-list.md' => q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tools/validator-fixtures/ordered-dot-list.md reason=noncanonical-task-form
REMEDY D2B-SPEC003-PLAN-TASK-ID Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'ordered-paren-list.md' => q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tools/validator-fixtures/ordered-paren-list.md reason=noncanonical-task-form
REMEDY D2B-SPEC003-PLAN-TASK-ID Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'ordered-list.md' => q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tools/validator-fixtures/ordered-list.md reason=noncanonical-task-form
REMEDY D2B-SPEC003-PLAN-TASK-ID Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'indentation.md' => q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tools/validator-fixtures/indentation.md reason=noncanonical-task-form
REMEDY D2B-SPEC003-PLAN-TASK-ID Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'tab-indentation.md' => q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tools/validator-fixtures/tab-indentation.md reason=noncanonical-task-form
REMEDY D2B-SPEC003-PLAN-TASK-ID Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'blockquote.md' => q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tools/validator-fixtures/blockquote.md reason=noncanonical-task-form
REMEDY D2B-SPEC003-PLAN-TASK-ID Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'nested-blockquote.md' => q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tools/validator-fixtures/nested-blockquote.md reason=noncanonical-task-form
REMEDY D2B-SPEC003-PLAN-TASK-ID Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'dot-alias.md' => q|FAIL D2B-SPEC003-PLAN-OWNERSHIP source=specs/003-adr052-bazel-rust/tools/validator-fixtures/dot-alias.md reason=dot-component
REMEDY D2B-SPEC003-PLAN-OWNERSHIP Replace ownership with unique literal normalized repository-relative paths or none.
RERUN D2B-SPEC003-PLAN-OWNERSHIP perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'dotdot-alias.md' => q|FAIL D2B-SPEC003-PLAN-OWNERSHIP source=specs/003-adr052-bazel-rust/tools/validator-fixtures/dotdot-alias.md reason=dot-component
REMEDY D2B-SPEC003-PLAN-OWNERSHIP Replace ownership with unique literal normalized repository-relative paths or none.
RERUN D2B-SPEC003-PLAN-OWNERSHIP perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'absolute-path.md' => q|FAIL D2B-SPEC003-PLAN-OWNERSHIP source=specs/003-adr052-bazel-rust/tools/validator-fixtures/absolute-path.md reason=absolute-path
REMEDY D2B-SPEC003-PLAN-OWNERSHIP Replace ownership with unique literal normalized repository-relative paths or none.
RERUN D2B-SPEC003-PLAN-OWNERSHIP perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'repeated-separator.md' => q|FAIL D2B-SPEC003-PLAN-OWNERSHIP source=specs/003-adr052-bazel-rust/tools/validator-fixtures/repeated-separator.md reason=repeated-separator
REMEDY D2B-SPEC003-PLAN-OWNERSHIP Replace ownership with unique literal normalized repository-relative paths or none.
RERUN D2B-SPEC003-PLAN-OWNERSHIP perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'malformed-quoting.md' => q|FAIL D2B-SPEC003-PLAN-OWNERSHIP source=specs/003-adr052-bazel-rust/tools/validator-fixtures/malformed-quoting.md reason=malformed-quoting
REMEDY D2B-SPEC003-PLAN-OWNERSHIP Replace ownership with unique literal normalized repository-relative paths or none.
RERUN D2B-SPEC003-PLAN-OWNERSHIP perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'duplicate-path.md' => q|FAIL D2B-SPEC003-PLAN-OWNERSHIP source=specs/003-adr052-bazel-rust/tools/validator-fixtures/duplicate-path.md reason=duplicate-path
REMEDY D2B-SPEC003-PLAN-OWNERSHIP Replace ownership with unique literal normalized repository-relative paths or none.
RERUN D2B-SPEC003-PLAN-OWNERSHIP perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'empty-path.md' => q|FAIL D2B-SPEC003-PLAN-OWNERSHIP source=specs/003-adr052-bazel-rust/tools/validator-fixtures/empty-path.md reason=empty-path
REMEDY D2B-SPEC003-PLAN-OWNERSHIP Replace ownership with unique literal normalized repository-relative paths or none.
RERUN D2B-SPEC003-PLAN-OWNERSHIP perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'dynamic-ownership.md' => q|FAIL D2B-SPEC003-PLAN-OWNERSHIP source=specs/003-adr052-bazel-rust/tools/validator-fixtures/dynamic-ownership.md reason=unresolved-expression
REMEDY D2B-SPEC003-PLAN-OWNERSHIP Replace ownership with unique literal normalized repository-relative paths or none.
RERUN D2B-SPEC003-PLAN-OWNERSHIP perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'malformed-owner.md' => q|FAIL D2B-SPEC003-PLAN-OWNER source=specs/003-adr052-bazel-rust/tools/validator-fixtures/malformed-owner.md reason=malformed
REMEDY D2B-SPEC003-PLAN-OWNER Replace the owner with one literal lowercase scope identifier.
RERUN D2B-SPEC003-PLAN-OWNER perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'parser-omission.md' => q|FAIL D2B-SPEC003-PLAN-PARSE source=specs/003-adr052-bazel-rust/tools/validator-fixtures/parser-omission.md reason=malformed-record
REMEDY D2B-SPEC003-PLAN-PARSE Rewrite every unchecked task record with owner, files, and depends fields in canonical order.
RERUN D2B-SPEC003-PLAN-PARSE perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'repeated-metadata-field.md' => q|FAIL D2B-SPEC003-PLAN-PARSE source=specs/003-adr052-bazel-rust/tools/validator-fixtures/repeated-metadata-field.md reason=repeated-metadata-field
REMEDY D2B-SPEC003-PLAN-PARSE Rewrite every unchecked task record with owner, files, and depends fields in canonical order.
RERUN D2B-SPEC003-PLAN-PARSE perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'task-after-graph.md' => q|FAIL D2B-SPEC003-PLAN-PARSE source=specs/003-adr052-bazel-rust/tools/validator-fixtures/task-after-graph.md reason=checkbox-outside-task-section
REMEDY D2B-SPEC003-PLAN-PARSE Move every unchecked task record before the canonical ## Dependency graph section.
RERUN D2B-SPEC003-PLAN-PARSE perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'empty.md' => q|FAIL D2B-SPEC003-PLAN-PARSE source=specs/003-adr052-bazel-rust/tools/validator-fixtures/empty.md reason=no-tasks
REMEDY D2B-SPEC003-PLAN-PARSE Provide at least one canonical unchecked task record before the dependency graph.
RERUN D2B-SPEC003-PLAN-PARSE perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'whole-task-omission.md' => q|FAIL D2B-SPEC003-PLAN-CENSUS source=specs/003-adr052-bazel-rust/tools/validator-fixtures/whole-task-omission.md reason=mismatch
REMEDY D2B-SPEC003-PLAN-CENSUS Declare one independent task-ID census with exactly the canonical task IDs.
RERUN D2B-SPEC003-PLAN-CENSUS perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'census-missing.md' => q|FAIL D2B-SPEC003-PLAN-CENSUS source=specs/003-adr052-bazel-rust/tools/validator-fixtures/census-missing.md reason=missing-declaration
REMEDY D2B-SPEC003-PLAN-CENSUS Declare one independent task-ID census with exactly the canonical task IDs.
RERUN D2B-SPEC003-PLAN-CENSUS perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'census-duplicate.md' => q|FAIL D2B-SPEC003-PLAN-CENSUS source=specs/003-adr052-bazel-rust/tools/validator-fixtures/census-duplicate.md reason=duplicate-declaration
REMEDY D2B-SPEC003-PLAN-CENSUS Declare one independent task-ID census with exactly the canonical task IDs.
RERUN D2B-SPEC003-PLAN-CENSUS perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'census-malformed.md' => q|FAIL D2B-SPEC003-PLAN-CENSUS source=specs/003-adr052-bazel-rust/tools/validator-fixtures/census-malformed.md reason=malformed-declaration
REMEDY D2B-SPEC003-PLAN-CENSUS Declare one independent task-ID census with exactly the canonical task IDs.
RERUN D2B-SPEC003-PLAN-CENSUS perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'census-duplicate-id.md' => q|FAIL D2B-SPEC003-PLAN-CENSUS source=specs/003-adr052-bazel-rust/tools/validator-fixtures/census-duplicate-id.md reason=malformed-declaration
REMEDY D2B-SPEC003-PLAN-CENSUS Declare one independent task-ID census with exactly the canonical task IDs.
RERUN D2B-SPEC003-PLAN-CENSUS perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'task-duplicate.md' => q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tools/validator-fixtures/task-duplicate.md reason=duplicate-task
REMEDY D2B-SPEC003-PLAN-TASK-ID Assign every task one unique canonical TNNN identifier and update its dependency row.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'dependency-failure.md' => q|FAIL D2B-SPEC003-PLAN-DEPENDENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/dependency-failure.md reason=missing
REMEDY D2B-SPEC003-PLAN-DEPENDENCY Replace dependencies with unique existing earlier TNNN IDs or none.
RERUN D2B-SPEC003-PLAN-DEPENDENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'dependency-adjacency-mismatch.md' => q|FAIL D2B-SPEC003-PLAN-DEPENDENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/dependency-adjacency-mismatch.md reason=not-earlier
REMEDY D2B-SPEC003-PLAN-DEPENDENCY Replace dependencies with unique existing earlier TNNN IDs or none.
RERUN D2B-SPEC003-PLAN-DEPENDENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'dependency-malformed.md' => q|FAIL D2B-SPEC003-PLAN-DEPENDENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/dependency-malformed.md reason=malformed
REMEDY D2B-SPEC003-PLAN-DEPENDENCY Replace dependencies with unique existing earlier TNNN IDs or none.
RERUN D2B-SPEC003-PLAN-DEPENDENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'dependency-duplicate.md' => q|FAIL D2B-SPEC003-PLAN-DEPENDENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/dependency-duplicate.md reason=duplicate
REMEDY D2B-SPEC003-PLAN-DEPENDENCY Replace dependencies with unique existing earlier TNNN IDs or none.
RERUN D2B-SPEC003-PLAN-DEPENDENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'dependency-not-earlier.md' => q|FAIL D2B-SPEC003-PLAN-DEPENDENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/dependency-not-earlier.md reason=not-earlier
REMEDY D2B-SPEC003-PLAN-DEPENDENCY Replace dependencies with unique existing earlier TNNN IDs or none.
RERUN D2B-SPEC003-PLAN-DEPENDENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'adjacency-missing-row.md' => q|FAIL D2B-SPEC003-PLAN-ADJACENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/adjacency-missing-row.md reason=missing-row
REMEDY D2B-SPEC003-PLAN-ADJACENCY Make the dependency-graph row exactly equal the task depends field.
RERUN D2B-SPEC003-PLAN-ADJACENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'adjacency-extra-row.md' => q|FAIL D2B-SPEC003-PLAN-ADJACENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/adjacency-extra-row.md reason=extra-row
REMEDY D2B-SPEC003-PLAN-ADJACENCY Make the dependency-graph row exactly equal the task depends field.
RERUN D2B-SPEC003-PLAN-ADJACENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'adjacency-duplicate-row.md' => q|FAIL D2B-SPEC003-PLAN-ADJACENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/adjacency-duplicate-row.md reason=duplicate-row
REMEDY D2B-SPEC003-PLAN-ADJACENCY Make the dependency-graph row exactly equal the task depends field.
RERUN D2B-SPEC003-PLAN-ADJACENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'adjacency-malformed.md' => q|FAIL D2B-SPEC003-PLAN-ADJACENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/adjacency-malformed.md reason=malformed
REMEDY D2B-SPEC003-PLAN-ADJACENCY Make the dependency-graph row exactly equal the task depends field.
RERUN D2B-SPEC003-PLAN-ADJACENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'adjacency-duplicate-dependency.md' => q|FAIL D2B-SPEC003-PLAN-ADJACENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/adjacency-duplicate-dependency.md reason=duplicate-dependency
REMEDY D2B-SPEC003-PLAN-ADJACENCY Make the dependency-graph row exactly equal the task depends field.
RERUN D2B-SPEC003-PLAN-ADJACENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'adjacency-mismatch.md' => q|FAIL D2B-SPEC003-PLAN-ADJACENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/adjacency-mismatch.md reason=mismatch
REMEDY D2B-SPEC003-PLAN-ADJACENCY Make the dependency-graph row exactly equal the task depends field.
RERUN D2B-SPEC003-PLAN-ADJACENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'cycle.md' => q|FAIL D2B-SPEC003-PLAN-CYCLE source=specs/003-adr052-bazel-rust/tools/validator-fixtures/cycle.md reason=detected
REMEDY D2B-SPEC003-PLAN-CYCLE Remove a dependency edge so the task graph is acyclic.
RERUN D2B-SPEC003-PLAN-CYCLE perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'concurrent-conflict.md' => q|FAIL D2B-SPEC003-PLAN-CONFLICT source=specs/003-adr052-bazel-rust/tools/validator-fixtures/concurrent-conflict.md reason=detected
REMEDY D2B-SPEC003-PLAN-CONFLICT Order the conflicting tasks by dependency or give them disjoint owned paths.
RERUN D2B-SPEC003-PLAN-CONFLICT perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'section-missing.md' => q|FAIL D2B-SPEC003-PLAN-SECTION source=specs/003-adr052-bazel-rust/tools/validator-fixtures/section-missing.md reason=missing
REMEDY D2B-SPEC003-PLAN-SECTION Create or retain exactly one canonical ## Dependency graph section.
RERUN D2B-SPEC003-PLAN-SECTION perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'section-duplicate.md' => q|FAIL D2B-SPEC003-PLAN-SECTION source=specs/003-adr052-bazel-rust/tools/validator-fixtures/section-duplicate.md reason=duplicate
REMEDY D2B-SPEC003-PLAN-SECTION Create or retain exactly one canonical ## Dependency graph section.
RERUN D2B-SPEC003-PLAN-SECTION perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
);

sub run_self_tests {
    my $fixture_dir = File::Spec->catdir($Bin, 'validator-fixtures');
    my $positive =
        read_repository_file(File::Spec->catfile($fixture_dir, 'positive.md'));
    if (!defined $positive) {
        print STDERR render_error(
            error_record('read', 'unreadable'), 'positive.md'
        );
        return 1;
    }
    my ($positive_errors) = validate_text($positive);
    if (@$positive_errors) {
        print STDERR render_error(
            error_record('parse', 'unexpected-positive-failure'), 'positive.md'
        );
        return 1;
    }

    my @cases = (
        ['malformed-header',             'task_id',    'malformed-header'],
        ['star-list',                    'task_id',    'noncanonical-task-form'],
        ['plus-list',                    'task_id',    'noncanonical-task-form'],
        ['ordered-dot-list',             'task_id',    'noncanonical-task-form'],
        ['ordered-paren-list',           'task_id',    'noncanonical-task-form'],
        ['ordered-list',                 'task_id',    'noncanonical-task-form'],
        ['indentation',                  'task_id',    'noncanonical-task-form'],
        ['tab-indentation',              'task_id',    'noncanonical-task-form'],
        ['blockquote',                   'task_id',    'noncanonical-task-form'],
        ['nested-blockquote',            'task_id',    'noncanonical-task-form'],
        ['dot-alias',                    'ownership',  'dot-component'],
        ['dotdot-alias',                 'ownership',  'dot-component'],
        ['absolute-path',                'ownership',  'absolute-path'],
        ['repeated-separator',           'ownership',  'repeated-separator'],
        ['malformed-quoting',            'ownership',  'malformed-quoting'],
        ['duplicate-path',               'ownership',  'duplicate-path'],
        ['empty-path',                   'ownership',  'empty-path'],
        ['dynamic-ownership',            'ownership',  'unresolved-expression'],
        ['malformed-owner',              'owner',      'malformed'],
        ['parser-omission',              'parse',      'malformed-record'],
        ['repeated-metadata-field',      'parse',      'repeated-metadata-field'],
        ['task-after-graph',             'parse',      'checkbox-outside-task-section'],
        ['empty',                        'parse',      'no-tasks'],
        ['whole-task-omission',          'census',     'mismatch'],
        ['census-missing',               'census',     'missing-declaration'],
        ['census-duplicate',             'census',     'duplicate-declaration'],
        ['census-malformed',             'census',     'malformed-declaration'],
        ['census-duplicate-id',          'census',     'malformed-declaration'],
        ['task-duplicate',               'task_id',    'duplicate-task'],
        ['dependency-failure',           'dependency', 'missing'],
        ['dependency-adjacency-mismatch', 'dependency', 'not-earlier'],
        ['dependency-malformed',         'dependency', 'malformed'],
        ['dependency-duplicate',         'dependency', 'duplicate'],
        ['dependency-not-earlier',       'dependency', 'not-earlier'],
        ['adjacency-missing-row',        'adjacency',  'missing-row'],
        ['adjacency-extra-row',          'adjacency',  'extra-row'],
        ['adjacency-duplicate-row',      'adjacency',  'duplicate-row'],
        ['adjacency-malformed',          'adjacency',  'malformed'],
        ['adjacency-duplicate-dependency', 'adjacency', 'duplicate-dependency'],
        ['adjacency-mismatch',           'adjacency',  'mismatch'],
        ['cycle',                        'cycle',      'detected'],
        ['concurrent-conflict',          'conflict',   'detected'],
        ['section-missing',              'section',    'missing'],
        ['section-duplicate',            'section',    'duplicate'],
    );

    for my $case (@cases) {
        my ($stem, $expected_kind, $expected_reason) = @$case;
        my $name = "$stem.md";
        my $text =
            read_repository_file(File::Spec->catfile($fixture_dir, $name));
        if (!defined $text) {
            print STDERR render_error(
                error_record('read', 'unreadable'), $name
            );
            return 1;
        }
        my ($errors) = validate_text($text);
        my $actual = join '', map { render_error($_, $name) } @$errors;
        my $expected = $expected_stderr{$name};
        if ($actual ne $expected) {
            print STDERR render_error(
                error_record('parse', 'expected-failure-missing'), $name
            );
            return 1;
        }
    }

    my $unsupported_expected = q|FAIL D2B-SPEC003-PLAN-PARSE source=specs/003-adr052-bazel-rust/tasks.md reason=unsupported-arguments
REMEDY D2B-SPEC003-PLAN-PARSE Invoke the validator with no argument or with only --self-test.
RERUN D2B-SPEC003-PLAN-PARSE perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|;
    my $unsupported_actual =
        render_error(
            error_record('parse', 'unsupported-arguments'),
            'tasks'
        );
    if ($unsupported_actual ne $unsupported_expected) {
        print STDERR render_error(
            error_record('parse', 'diagnostic-contract'), 'tasks'
        );
        return 1;
    }

    my $read_expected = q|FAIL D2B-SPEC003-PLAN-READ source=specs/003-adr052-bazel-rust/tasks.md reason=unreadable
REMEDY D2B-SPEC003-PLAN-READ Restore the repository-relative source and make it readable.
RERUN D2B-SPEC003-PLAN-READ perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|;
    my $read_actual =
        render_error(error_record('read', 'unreadable'), 'tasks');
    if ($read_actual ne $read_expected) {
        print STDERR render_error(
            error_record('parse', 'diagnostic-contract'), 'tasks'
        );
        return 1;
    }

    print 'PASS: 47 validator self-tests; positive fixture accepted; '
        . '44 independent negative fixtures cover noncanonical unchecked-list forms, census declarations, '
        . 'task parsing, ownership, dependency, adjacency, section, cycle, '
        . 'and conflict fixtures rejected; byte-exact fixed diagnostics and '
        . "unsupported-argument rendering verified\n";
    return 0;
}

if (@ARGV) {
    if (@ARGV == 1 && $ARGV[0] eq '--self-test') {
        exit run_self_tests();
    }
    print STDERR render_error(
        error_record('parse', 'unsupported-arguments'), 'tasks'
    );
    exit 2;
}

my $filesystem_tasks_path = File::Spec->catfile($Bin, '..', 'tasks.md');
my $text = read_repository_file($filesystem_tasks_path);
if (!defined $text) {
    print STDERR render_error(
        error_record('read', 'unreadable'), 'tasks'
    );
    exit 1;
}
my ($errors, $task_count) = validate_text($text);
if (@$errors) {
    print STDERR render_error($_, 'tasks') for @$errors;
    exit 1;
}

print "PASS: $task_count unique tasks with exact canonical headers and owned paths; "
    . "dependencies exist and precede consumers; adjacency matches; "
    . "graph is acyclic; concurrently ready ownership is disjoint\n";
