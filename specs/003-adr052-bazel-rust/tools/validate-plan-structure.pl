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
my $max_record_ordinal = 999;
my $max_line_number = 9999;

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
    my ($kind, $reason, $record, $line) = @_;
    $record = 1
        if !defined($record)
        || $record !~ /\A[0-9]+\z/
        || $record < 1
        || $record > $max_record_ordinal;
    $line = 1
        if !defined($line)
        || $line !~ /\A[0-9]+\z/
        || $line < 1
        || $line > $max_line_number;
    return {
        kind   => $kind,
        reason => $reason,
        record => $record,
        line   => $line,
    };
}

sub line_number_at {
    my ($text, $offset) = @_;
    $offset = 0 if !defined($offset) || $offset < 0;
    my $prefix = substr($text, 0, $offset);
    return 1 + ($prefix =~ tr/\n/\n/);
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
        . " record=$error->{record} line=$error->{line} reason=$reason\n"
        . "REMEDY $error_code $correction\n"
        . "RERUN $error_code $rerun\n";
}

sub parse_owned_path {
    my ($raw, $files, $errors, $record, $line) = @_;
    my $path = trim($raw);

    if ($path eq '' || $path eq 'none') {
        push @$errors,
            error_record('ownership', 'empty-path', $record, $line);
        return;
    }

    my $backticks = () = $path =~ /`/g;
    if ($backticks == 2 && $path =~ /\A`([^`]*)`\z/) {
        $path = $1;
    } elsif ($backticks != 0 || $path =~ /['"]/) {
        push @$errors,
            error_record('ownership', 'malformed-quoting', $record, $line);
        return;
    }

    if ($path =~ m{\A/}) {
        push @$errors,
            error_record('ownership', 'absolute-path', $record, $line);
        return;
    }
    if ($path =~ m{//}) {
        push @$errors,
            error_record('ownership', 'repeated-separator', $record, $line);
        return;
    }

    my @components = split m{/}, $path, -1;
    if (grep { $_ eq '.' || $_ eq '..' } @components) {
        push @$errors,
            error_record('ownership', 'dot-component', $record, $line);
        return;
    }

    if (
        $path =~ /\b(?:and\s+every|listed\s+in|generated\s+paths?)\b/i
        || $path =~ /[*?\[\]{}]/
        || $path !~ m{\A[A-Za-z0-9_.@+-]+(?:/[A-Za-z0-9_.@+-]+)*\z}
    ) {
        push @$errors,
            error_record(
                'ownership',
                'unresolved-expression',
                $record,
                $line
            );
        return;
    }

    if ($files->{$path}++) {
        push @$errors,
            error_record('ownership', 'duplicate-path', $record, $line);
    }
}

sub census_task_forms {
    my ($text) = @_;
    my @forms;
    my $graph_seen = 0;
    my $line_number = 0;
    my $record_ordinal = 0;
    for my $line (split /\n/, $text, -1) {
        $line_number++;
        $line =~ s/\r\z//;
        if ($line =~ /\A## Dependency graph[ \t]*\z/) {
            $graph_seen = 1;
        }
        next unless $line =~
            /\A[ \t]*(?:>[ \t]*)*(?:[-*+]|\d+[.)]?)[ \t]+\[[ \t]*\]/;
        $record_ordinal++;

        my $canonical =
            $line =~ /\A- \[ \] T[0-9]{3}(?=\s|\z)/;
        my $canonical_marker =
            $line =~ /\A- \[ \] /;
        push @forms, {
            canonical        => $canonical,
            canonical_marker => $canonical_marker,
            after_graph      => $graph_seen,
            record           => $record_ordinal,
            line             => $line_number,
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
                    (
                        $form->{canonical_marker}
                        ? 'malformed-header'
                        : 'noncanonical-task-form'
                    ),
                    $form->{record},
                    $form->{line}
                );
            next;
        }
        if ($form->{after_graph}) {
            push @errors,
                error_record(
                    'parse',
                    'checkbox-outside-task-section',
                    $form->{record},
                    $form->{line}
                );
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
    my $graph_offset = index($text, '## Dependency graph');
    my $adjacency_first_line =
        line_number_at($text, $graph_offset) + 1;

    my @records;
    while (
        $task_text =~
        /(^- \[ \] T[0-9]{3}(?=\s|\z).*?)(?=^- \[ \] T[0-9]{3}(?=\s|\z)|\z)/msg
    ) {
        push @records, {
            text  => $1,
            start => $-[1],
        };
    }
    my (@tasks, %seen);
    my $record_ordinal = 0;
    for my $record_match (@records) {
        $record_ordinal++;
        my $record = $record_match->{text};
        my $record_line =
            line_number_at($task_text, $record_match->{start});
        if (
            $record !~
            /\A-\x20\[\x20\]\x20(T[0-9]{3})\s+
              \[owner:\s*([^\]\r\n]+)\]\s+
              \[files:\s*(.*?)\]\s+
              \[depends:\s*([^\]\r\n]+)\](?=\s|\z)(.*)\z/sx
        ) {
            push @errors,
                error_record(
                    'parse',
                    'malformed-record',
                    $record_ordinal,
                    $record_line
                );
            next;
        }

        my ($id, $owner, $raw_files, $raw_depends, $remainder) =
            ($1, $2, $3, $4, $5);
        if ($remainder =~ /\[\s*(?:owner|files|depends)\s*:/i) {
            push @errors,
                error_record(
                    'parse',
                    'repeated-metadata-field',
                    $record_ordinal,
                    $record_line
                );
            next;
        }
        if ($seen{$id}++) {
            push @errors,
                error_record(
                    'task_id',
                    'duplicate-task',
                    $record_ordinal,
                    $record_line
                );
        }

        $owner = trim($owner);
        if ($owner !~ /\A[a-z0-9-]+\z/) {
            push @errors,
                error_record(
                    'owner',
                    'malformed',
                    $record_ordinal,
                    $record_line
                );
        }

        my %files;
        my @file_items = split /,/, $raw_files, -1;
        my $files_none = @file_items == 1 && trim($file_items[0]) eq 'none';
        if ($files_none) {
            @file_items = ();
        }
        if (!$files_none && !@file_items) {
            push @errors,
                error_record(
                    'ownership',
                    'empty-path',
                    $record_ordinal,
                    $record_line
                );
        }
        parse_owned_path(
            $_,
            \%files,
            \@errors,
            $record_ordinal,
            $record_line
        ) for @file_items;

        my (@depends, %depends_seen);
        if (trim($raw_depends) ne 'none') {
            for my $raw (split /,/, $raw_depends, -1) {
                my $dependency = trim($raw);
                if ($dependency !~ /\AT[0-9]{3}\z/) {
                    push @errors,
                        error_record(
                            'dependency',
                            'malformed',
                            $record_ordinal,
                            $record_line
                        );
                    next;
                }
                if ($depends_seen{$dependency}++) {
                    push @errors,
                        error_record(
                            'dependency',
                            'duplicate',
                            $record_ordinal,
                            $record_line
                        );
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
            record  => $record_ordinal,
            line    => $record_line,
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
            my $task = $by_id{$id};
            push @errors,
                error_record(
                    'cycle',
                    'detected',
                    $task->{record},
                    $task->{line}
                );
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
                push @errors,
                    error_record(
                        'dependency',
                        'missing',
                        $task->{record},
                        $task->{line}
                    );
                next;
            }
            if ($by_id{$dependency}->{order} >= $task->{order}) {
                push @errors,
                    error_record(
                        'dependency',
                        'not-earlier',
                        $task->{record},
                        $task->{line}
                    );
            }
        }
    }
    return (\@errors, scalar @tasks) if @errors;

    my (%rows, %row_seen, %row_location);
    while ($adjacency_text =~ /^(T[0-9]{3})\s+<-\s+(.+)$/mg) {
        my ($id, $raw) = ($1, $2);
        my $row_line =
            $adjacency_first_line
            + line_number_at($adjacency_text, $-[0])
            - 1;
        my $row_record =
            exists($by_id{$id}) ? $by_id{$id}->{record} : 1;
        if ($row_seen{$id}++) {
            push @errors,
                error_record(
                    'adjacency',
                    'duplicate-row',
                    $row_record,
                    $row_line
                );
        }
        my (%dependencies, %adjacency_seen);
        if (trim($raw) ne 'none') {
            for my $value (split /,/, $raw, -1) {
                my $dependency = trim($value);
                if ($dependency !~ /\AT[0-9]{3}\z/) {
                    push @errors,
                        error_record(
                            'adjacency',
                            'malformed',
                            $row_record,
                            $row_line
                        );
                    next;
                }
                if ($adjacency_seen{$dependency}++) {
                    push @errors,
                        error_record(
                            'adjacency',
                            'duplicate-dependency',
                            $row_record,
                            $row_line
                        );
                    next;
                }
                $dependencies{$dependency} = 1;
            }
        }
        $rows{$id} = \%dependencies;
        $row_location{$id} = {
            record => $row_record,
            line   => $row_line,
        };
    }
    for my $id (sort keys %by_id) {
        if (!exists $rows{$id}) {
            push @errors,
                error_record(
                    'adjacency',
                    'missing-row',
                    $by_id{$id}->{record},
                    $by_id{$id}->{line}
                );
            next;
        }
        my $inline = join ',', sort keys %{$graph{$id}};
        my $row = join ',', sort keys %{$rows{$id}};
        if ($inline ne $row) {
            push @errors,
                error_record(
                    'adjacency',
                    'mismatch',
                    $row_location{$id}->{record},
                    $row_location{$id}->{line}
                );
        }
    }
    for my $id (sort keys %rows) {
        if (!exists $by_id{$id}) {
            push @errors,
                error_record(
                    'adjacency',
                    'extra-row',
                    $row_location{$id}->{record},
                    $row_location{$id}->{line}
                );
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
                push @errors,
                    error_record(
                        'conflict',
                        'detected',
                        $right->{record},
                        $right->{line}
                    );
            }
        }
    }

    return (\@errors, scalar @tasks);
}

my %expected_stderr = (
    'malformed-header.md' => q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tools/validator-fixtures/malformed-header.md record=1 line=6 reason=malformed-header
REMEDY D2B-SPEC003-PLAN-TASK-ID Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'star-list.md' => q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tools/validator-fixtures/star-list.md record=1 line=7 reason=noncanonical-task-form
REMEDY D2B-SPEC003-PLAN-TASK-ID Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'plus-list.md' => q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tools/validator-fixtures/plus-list.md record=1 line=7 reason=noncanonical-task-form
REMEDY D2B-SPEC003-PLAN-TASK-ID Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'ordered-dot-list.md' => q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tools/validator-fixtures/ordered-dot-list.md record=1 line=7 reason=noncanonical-task-form
REMEDY D2B-SPEC003-PLAN-TASK-ID Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'ordered-paren-list.md' => q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tools/validator-fixtures/ordered-paren-list.md record=1 line=7 reason=noncanonical-task-form
REMEDY D2B-SPEC003-PLAN-TASK-ID Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'ordered-list.md' => q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tools/validator-fixtures/ordered-list.md record=1 line=7 reason=noncanonical-task-form
REMEDY D2B-SPEC003-PLAN-TASK-ID Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'indentation.md' => q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tools/validator-fixtures/indentation.md record=1 line=7 reason=noncanonical-task-form
REMEDY D2B-SPEC003-PLAN-TASK-ID Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'tab-indentation.md' => q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tools/validator-fixtures/tab-indentation.md record=1 line=7 reason=noncanonical-task-form
REMEDY D2B-SPEC003-PLAN-TASK-ID Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'blockquote.md' => q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tools/validator-fixtures/blockquote.md record=1 line=7 reason=noncanonical-task-form
REMEDY D2B-SPEC003-PLAN-TASK-ID Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'nested-blockquote.md' => q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tools/validator-fixtures/nested-blockquote.md record=1 line=7 reason=noncanonical-task-form
REMEDY D2B-SPEC003-PLAN-TASK-ID Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'dot-alias.md' => q|FAIL D2B-SPEC003-PLAN-OWNERSHIP source=specs/003-adr052-bazel-rust/tools/validator-fixtures/dot-alias.md record=1 line=7 reason=dot-component
REMEDY D2B-SPEC003-PLAN-OWNERSHIP Replace ownership with unique literal normalized repository-relative paths or none.
RERUN D2B-SPEC003-PLAN-OWNERSHIP perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'dotdot-alias.md' => q|FAIL D2B-SPEC003-PLAN-OWNERSHIP source=specs/003-adr052-bazel-rust/tools/validator-fixtures/dotdot-alias.md record=1 line=7 reason=dot-component
REMEDY D2B-SPEC003-PLAN-OWNERSHIP Replace ownership with unique literal normalized repository-relative paths or none.
RERUN D2B-SPEC003-PLAN-OWNERSHIP perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'absolute-path.md' => q|FAIL D2B-SPEC003-PLAN-OWNERSHIP source=specs/003-adr052-bazel-rust/tools/validator-fixtures/absolute-path.md record=1 line=7 reason=absolute-path
REMEDY D2B-SPEC003-PLAN-OWNERSHIP Replace ownership with unique literal normalized repository-relative paths or none.
RERUN D2B-SPEC003-PLAN-OWNERSHIP perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'repeated-separator.md' => q|FAIL D2B-SPEC003-PLAN-OWNERSHIP source=specs/003-adr052-bazel-rust/tools/validator-fixtures/repeated-separator.md record=1 line=7 reason=repeated-separator
REMEDY D2B-SPEC003-PLAN-OWNERSHIP Replace ownership with unique literal normalized repository-relative paths or none.
RERUN D2B-SPEC003-PLAN-OWNERSHIP perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'malformed-quoting.md' => q|FAIL D2B-SPEC003-PLAN-OWNERSHIP source=specs/003-adr052-bazel-rust/tools/validator-fixtures/malformed-quoting.md record=1 line=7 reason=malformed-quoting
REMEDY D2B-SPEC003-PLAN-OWNERSHIP Replace ownership with unique literal normalized repository-relative paths or none.
RERUN D2B-SPEC003-PLAN-OWNERSHIP perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'duplicate-path.md' => q|FAIL D2B-SPEC003-PLAN-OWNERSHIP source=specs/003-adr052-bazel-rust/tools/validator-fixtures/duplicate-path.md record=1 line=7 reason=duplicate-path
REMEDY D2B-SPEC003-PLAN-OWNERSHIP Replace ownership with unique literal normalized repository-relative paths or none.
RERUN D2B-SPEC003-PLAN-OWNERSHIP perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'empty-path.md' => q|FAIL D2B-SPEC003-PLAN-OWNERSHIP source=specs/003-adr052-bazel-rust/tools/validator-fixtures/empty-path.md record=1 line=7 reason=empty-path
REMEDY D2B-SPEC003-PLAN-OWNERSHIP Replace ownership with unique literal normalized repository-relative paths or none.
RERUN D2B-SPEC003-PLAN-OWNERSHIP perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'dynamic-ownership.md' => q|FAIL D2B-SPEC003-PLAN-OWNERSHIP source=specs/003-adr052-bazel-rust/tools/validator-fixtures/dynamic-ownership.md record=1 line=7 reason=unresolved-expression
REMEDY D2B-SPEC003-PLAN-OWNERSHIP Replace ownership with unique literal normalized repository-relative paths or none.
RERUN D2B-SPEC003-PLAN-OWNERSHIP perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'malformed-owner.md' => q|FAIL D2B-SPEC003-PLAN-OWNER source=specs/003-adr052-bazel-rust/tools/validator-fixtures/malformed-owner.md record=1 line=7 reason=malformed
REMEDY D2B-SPEC003-PLAN-OWNER Replace the owner with one literal lowercase scope identifier.
RERUN D2B-SPEC003-PLAN-OWNER perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'parser-omission.md' => q|FAIL D2B-SPEC003-PLAN-PARSE source=specs/003-adr052-bazel-rust/tools/validator-fixtures/parser-omission.md record=1 line=7 reason=malformed-record
REMEDY D2B-SPEC003-PLAN-PARSE Rewrite every unchecked task record with owner, files, and depends fields in canonical order.
RERUN D2B-SPEC003-PLAN-PARSE perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'repeated-metadata-field.md' => q|FAIL D2B-SPEC003-PLAN-PARSE source=specs/003-adr052-bazel-rust/tools/validator-fixtures/repeated-metadata-field.md record=1 line=7 reason=repeated-metadata-field
REMEDY D2B-SPEC003-PLAN-PARSE Rewrite every unchecked task record with owner, files, and depends fields in canonical order.
RERUN D2B-SPEC003-PLAN-PARSE perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'task-after-graph.md' => q|FAIL D2B-SPEC003-PLAN-PARSE source=specs/003-adr052-bazel-rust/tools/validator-fixtures/task-after-graph.md record=2 line=16 reason=checkbox-outside-task-section
REMEDY D2B-SPEC003-PLAN-PARSE Move every unchecked task record before the canonical ## Dependency graph section.
RERUN D2B-SPEC003-PLAN-PARSE perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'empty.md' => q|FAIL D2B-SPEC003-PLAN-PARSE source=specs/003-adr052-bazel-rust/tools/validator-fixtures/empty.md record=1 line=1 reason=no-tasks
REMEDY D2B-SPEC003-PLAN-PARSE Provide at least one canonical unchecked task record before the dependency graph.
RERUN D2B-SPEC003-PLAN-PARSE perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'whole-task-omission.md' => q|FAIL D2B-SPEC003-PLAN-CENSUS source=specs/003-adr052-bazel-rust/tools/validator-fixtures/whole-task-omission.md record=1 line=1 reason=mismatch
REMEDY D2B-SPEC003-PLAN-CENSUS Declare one independent task-ID census with exactly the canonical task IDs.
RERUN D2B-SPEC003-PLAN-CENSUS perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'census-missing.md' => q|FAIL D2B-SPEC003-PLAN-CENSUS source=specs/003-adr052-bazel-rust/tools/validator-fixtures/census-missing.md record=1 line=1 reason=missing-declaration
REMEDY D2B-SPEC003-PLAN-CENSUS Declare one independent task-ID census with exactly the canonical task IDs.
RERUN D2B-SPEC003-PLAN-CENSUS perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'census-duplicate.md' => q|FAIL D2B-SPEC003-PLAN-CENSUS source=specs/003-adr052-bazel-rust/tools/validator-fixtures/census-duplicate.md record=1 line=1 reason=duplicate-declaration
REMEDY D2B-SPEC003-PLAN-CENSUS Declare one independent task-ID census with exactly the canonical task IDs.
RERUN D2B-SPEC003-PLAN-CENSUS perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'census-malformed.md' => q|FAIL D2B-SPEC003-PLAN-CENSUS source=specs/003-adr052-bazel-rust/tools/validator-fixtures/census-malformed.md record=1 line=1 reason=malformed-declaration
REMEDY D2B-SPEC003-PLAN-CENSUS Declare one independent task-ID census with exactly the canonical task IDs.
RERUN D2B-SPEC003-PLAN-CENSUS perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'census-duplicate-id.md' => q|FAIL D2B-SPEC003-PLAN-CENSUS source=specs/003-adr052-bazel-rust/tools/validator-fixtures/census-duplicate-id.md record=1 line=1 reason=malformed-declaration
REMEDY D2B-SPEC003-PLAN-CENSUS Declare one independent task-ID census with exactly the canonical task IDs.
RERUN D2B-SPEC003-PLAN-CENSUS perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'task-duplicate.md' => q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tools/validator-fixtures/task-duplicate.md record=2 line=8 reason=duplicate-task
REMEDY D2B-SPEC003-PLAN-TASK-ID Assign every task one unique canonical TNNN identifier and update its dependency row.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'dependency-failure.md' => q|FAIL D2B-SPEC003-PLAN-DEPENDENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/dependency-failure.md record=2 line=9 reason=missing
REMEDY D2B-SPEC003-PLAN-DEPENDENCY Replace dependencies with unique existing earlier TNNN IDs or none.
RERUN D2B-SPEC003-PLAN-DEPENDENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'dependency-adjacency-mismatch.md' => q|FAIL D2B-SPEC003-PLAN-DEPENDENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/dependency-adjacency-mismatch.md record=1 line=8 reason=not-earlier
REMEDY D2B-SPEC003-PLAN-DEPENDENCY Replace dependencies with unique existing earlier TNNN IDs or none.
RERUN D2B-SPEC003-PLAN-DEPENDENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'dependency-malformed.md' => q|FAIL D2B-SPEC003-PLAN-DEPENDENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/dependency-malformed.md record=1 line=7 reason=malformed
REMEDY D2B-SPEC003-PLAN-DEPENDENCY Replace dependencies with unique existing earlier TNNN IDs or none.
RERUN D2B-SPEC003-PLAN-DEPENDENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'dependency-duplicate.md' => q|FAIL D2B-SPEC003-PLAN-DEPENDENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/dependency-duplicate.md record=2 line=9 reason=duplicate
REMEDY D2B-SPEC003-PLAN-DEPENDENCY Replace dependencies with unique existing earlier TNNN IDs or none.
RERUN D2B-SPEC003-PLAN-DEPENDENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'dependency-not-earlier.md' => q|FAIL D2B-SPEC003-PLAN-DEPENDENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/dependency-not-earlier.md record=1 line=8 reason=not-earlier
REMEDY D2B-SPEC003-PLAN-DEPENDENCY Replace dependencies with unique existing earlier TNNN IDs or none.
RERUN D2B-SPEC003-PLAN-DEPENDENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'adjacency-missing-row.md' => q|FAIL D2B-SPEC003-PLAN-ADJACENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/adjacency-missing-row.md record=1 line=7 reason=missing-row
REMEDY D2B-SPEC003-PLAN-ADJACENCY Make the dependency-graph row exactly equal the task depends field.
RERUN D2B-SPEC003-PLAN-ADJACENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'adjacency-extra-row.md' => q|FAIL D2B-SPEC003-PLAN-ADJACENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/adjacency-extra-row.md record=1 line=14 reason=extra-row
REMEDY D2B-SPEC003-PLAN-ADJACENCY Make the dependency-graph row exactly equal the task depends field.
RERUN D2B-SPEC003-PLAN-ADJACENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'adjacency-duplicate-row.md' => q|FAIL D2B-SPEC003-PLAN-ADJACENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/adjacency-duplicate-row.md record=1 line=14 reason=duplicate-row
REMEDY D2B-SPEC003-PLAN-ADJACENCY Make the dependency-graph row exactly equal the task depends field.
RERUN D2B-SPEC003-PLAN-ADJACENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'adjacency-malformed.md' => q|FAIL D2B-SPEC003-PLAN-ADJACENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/adjacency-malformed.md record=1 line=13 reason=malformed
REMEDY D2B-SPEC003-PLAN-ADJACENCY Make the dependency-graph row exactly equal the task depends field.
RERUN D2B-SPEC003-PLAN-ADJACENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'adjacency-duplicate-dependency.md' => q|FAIL D2B-SPEC003-PLAN-ADJACENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/adjacency-duplicate-dependency.md record=2 line=16 reason=duplicate-dependency
REMEDY D2B-SPEC003-PLAN-ADJACENCY Make the dependency-graph row exactly equal the task depends field.
RERUN D2B-SPEC003-PLAN-ADJACENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'adjacency-mismatch.md' => q|FAIL D2B-SPEC003-PLAN-ADJACENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/adjacency-mismatch.md record=2 line=16 reason=mismatch
REMEDY D2B-SPEC003-PLAN-ADJACENCY Make the dependency-graph row exactly equal the task depends field.
RERUN D2B-SPEC003-PLAN-ADJACENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'cycle.md' => q|FAIL D2B-SPEC003-PLAN-CYCLE source=specs/003-adr052-bazel-rust/tools/validator-fixtures/cycle.md record=1 line=8 reason=detected
REMEDY D2B-SPEC003-PLAN-CYCLE Remove a dependency edge so the task graph is acyclic.
RERUN D2B-SPEC003-PLAN-CYCLE perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'concurrent-conflict.md' => q|FAIL D2B-SPEC003-PLAN-CONFLICT source=specs/003-adr052-bazel-rust/tools/validator-fixtures/concurrent-conflict.md record=2 line=9 reason=detected
REMEDY D2B-SPEC003-PLAN-CONFLICT Order the conflicting tasks by dependency or give them disjoint owned paths.
RERUN D2B-SPEC003-PLAN-CONFLICT perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'section-missing.md' => q|FAIL D2B-SPEC003-PLAN-SECTION source=specs/003-adr052-bazel-rust/tools/validator-fixtures/section-missing.md record=1 line=1 reason=missing
REMEDY D2B-SPEC003-PLAN-SECTION Create or retain exactly one canonical ## Dependency graph section.
RERUN D2B-SPEC003-PLAN-SECTION perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'section-duplicate.md' => q|FAIL D2B-SPEC003-PLAN-SECTION source=specs/003-adr052-bazel-rust/tools/validator-fixtures/section-duplicate.md record=1 line=1 reason=duplicate
REMEDY D2B-SPEC003-PLAN-SECTION Create or retain exactly one canonical ## Dependency graph section.
RERUN D2B-SPEC003-PLAN-SECTION perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
);

sub run_plan_entrypoint {
    my (%args) = @_;
    my $reader = $args{reader};
    my $source_key = $args{source_key} // 'tasks';
    my $stdout = $args{stdout};
    my $stderr = $args{stderr};

    my $text = $reader->();
    if (!defined $text) {
        $$stderr .= render_error(
            error_record('read', 'unreadable', 1, 1),
            $source_key
        );
        return 1;
    }
    my ($errors, $task_count) = validate_text($text);
    if (@$errors) {
        $$stderr .= render_error($_, $source_key) for @$errors;
        return 1;
    }

    $$stdout .=
        "PASS: $task_count unique tasks with exact canonical headers and owned paths; "
        . "dependencies exist and precede consumers; adjacency matches; "
        . "graph is acyclic; concurrently ready ownership is disjoint\n";
    return 0;
}

sub run_cli_entrypoint {
    my (%args) = @_;
    my $argv = $args{argv};
    my $stdout = $args{stdout};
    my $stderr = $args{stderr};

    if (@$argv) {
        if (@$argv == 1 && $argv->[0] eq '--self-test') {
            return $args{self_test_runner}->($stdout, $stderr);
        }
        $$stderr .= render_error(
            error_record('parse', 'unsupported-arguments', 1, 1),
            'tasks'
        );
        return 2;
    }

    return run_plan_entrypoint(
        reader     => $args{reader},
        source_key => 'tasks',
        stdout     => $stdout,
        stderr     => $stderr,
    );
}

sub run_self_tests {
    my ($self_stdout, $self_stderr) = @_;
    my $fixture_dir = File::Spec->catdir($Bin, 'validator-fixtures');
    my $positive =
        read_repository_file(File::Spec->catfile($fixture_dir, 'positive.md'));
    if (!defined $positive) {
        $$self_stderr .= render_error(
            error_record('read', 'unreadable', 1, 1),
            'positive.md'
        );
        return 1;
    }
    my ($positive_stdout, $positive_stderr) = ('', '');
    my $positive_status = run_plan_entrypoint(
        reader     => sub { return $positive; },
        source_key => 'positive.md',
        stdout     => \$positive_stdout,
        stderr     => \$positive_stderr,
    );
    my $positive_expected =
        "PASS: 2 unique tasks with exact canonical headers and owned paths; "
        . "dependencies exist and precede consumers; adjacency matches; "
        . "graph is acyclic; concurrently ready ownership is disjoint\n";
    if (
        $positive_status != 0
        || $positive_stdout ne $positive_expected
        || $positive_stderr ne ''
    ) {
        $$self_stderr .= render_error(
            error_record(
                'parse',
                'unexpected-positive-failure',
                1,
                1
            ),
            'positive.md'
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
            $$self_stderr .= render_error(
                error_record('read', 'unreadable', 1, 1),
                $name
            );
            return 1;
        }
        my ($case_stdout, $case_stderr) = ('', '');
        my $case_status = run_plan_entrypoint(
            reader     => sub { return $text; },
            source_key => $name,
            stdout     => \$case_stdout,
            stderr     => \$case_stderr,
        );
        my $expected = $expected_stderr{$name};
        if (
            $case_status != 1
            || $case_stdout ne ''
            || $case_stderr ne $expected
        ) {
            $$self_stderr .= render_error(
                error_record(
                    'parse',
                    'expected-failure-missing',
                    1,
                    1
                ),
                $name
            );
            return 1;
        }
    }

    my $unsupported_expected = q|FAIL D2B-SPEC003-PLAN-PARSE source=specs/003-adr052-bazel-rust/tasks.md record=1 line=1 reason=unsupported-arguments
REMEDY D2B-SPEC003-PLAN-PARSE Invoke the validator with no argument or with only --self-test.
RERUN D2B-SPEC003-PLAN-PARSE perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|;
    my ($unsupported_stdout, $unsupported_stderr) = ('', '');
    my $unsupported_status = run_cli_entrypoint(
        argv             => ['--unsupported'],
        reader           => sub { die 'unsupported arguments must not read'; },
        self_test_runner => sub { die 'unsupported arguments must not self-test'; },
        stdout           => \$unsupported_stdout,
        stderr           => \$unsupported_stderr,
    );
    if (
        $unsupported_status != 2
        || $unsupported_stdout ne ''
        || $unsupported_stderr ne $unsupported_expected
    ) {
        $$self_stderr .= render_error(
            error_record('parse', 'diagnostic-contract', 1, 1),
            'tasks'
        );
        return 1;
    }

    my $read_expected = q|FAIL D2B-SPEC003-PLAN-READ source=specs/003-adr052-bazel-rust/tasks.md record=1 line=1 reason=unreadable
REMEDY D2B-SPEC003-PLAN-READ Restore the repository-relative source and make it readable.
RERUN D2B-SPEC003-PLAN-READ perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|;
    my ($read_stdout, $read_stderr) = ('', '');
    my $read_status = run_cli_entrypoint(
        argv             => [],
        reader           => sub { return undef; },
        self_test_runner => sub { die 'no self-test expected'; },
        stdout           => \$read_stdout,
        stderr           => \$read_stderr,
    );
    if (
        $read_status != 1
        || $read_stdout ne ''
        || $read_stderr ne $read_expected
    ) {
        $$self_stderr .= render_error(
            error_record('parse', 'diagnostic-contract', 1, 1),
            'tasks'
        );
        return 1;
    }

    for my $kind (sort keys %code) {
        if (!defined($remedy{$kind}) || $remedy{$kind} eq '') {
            $$self_stderr .= render_error(
                error_record('parse', 'diagnostic-contract', 1, 1),
                'tasks'
            );
            return 1;
        }
    }

    $$self_stdout .=
        'PASS: 47 validator self-tests; positive fixture accepted; '
        . '44 independent negative fixtures cover noncanonical unchecked-list forms, census declarations, '
        . 'task parsing, ownership, dependency, adjacency, section, cycle, '
        . 'and conflict fixtures rejected; full stderr byte-matched against '
        . 'independent literals; bounded record/line locators, unreadable-source '
        . "status 1, and unsupported-argument status 2 verified\n";
    return 0;
}

sub main {
    my ($stdout, $stderr) = ('', '');
    my $filesystem_tasks_path =
        File::Spec->catfile($Bin, '..', 'tasks.md');
    my @argv = @ARGV;
    my $status = run_cli_entrypoint(
        argv             => \@argv,
        reader           => sub {
            return read_repository_file($filesystem_tasks_path);
        },
        self_test_runner => sub {
            return run_self_tests(@_);
        },
        stdout           => \$stdout,
        stderr           => \$stderr,
    );
    print STDOUT $stdout;
    print STDERR $stderr;
    return $status;
}

exit main() unless caller;
1;
