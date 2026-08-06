#!/usr/bin/env perl
use strict;
use warnings;
use FindBin qw($Bin);
use File::Spec;

my $tool_path = 'specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl';
my $tasks_path = 'specs/003-adr052-bazel-rust/tasks.md';
my $rerun =
    "perl $tool_path --self-test && perl $tool_path";

my %code = (
    read       => 'D2B-SPEC003-PLAN-READ',
    section    => 'D2B-SPEC003-PLAN-SECTION',
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
    $code{read} =>
        'Restore the repository-relative source and make it readable.',
    $code{section} =>
        'Create or retain exactly one canonical ## Dependency graph section.',
    $code{parse} =>
        'Rewrite every unchecked task record with owner, files, and depends fields in canonical order.',
    $code{task_id} =>
        'Use the exact header - [ ] TNNN with one three-digit task ID.',
    $code{owner} =>
        'Replace the owner with one literal lowercase scope identifier.',
    $code{ownership} =>
        'Replace ownership with unique literal normalized repository-relative paths or none.',
    $code{dependency} =>
        'Replace dependencies with unique existing earlier TNNN IDs or none.',
    $code{adjacency} =>
        'Make the dependency-graph row exactly equal the task depends field.',
    $code{cycle} =>
        'Remove a dependency edge so the task graph is acyclic.',
    $code{conflict} =>
        'Order the conflicting tasks by dependency or give them disjoint owned paths.',
);

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
    my ($kind, $source, @fields) = @_;
    return join ' ',
        $code{$kind},
        "source=$source",
        @fields;
}

sub render_error {
    my ($error) = @_;
    my ($error_code) = split / /, $error, 2;
    my $correction = $remedy{$error_code}
        // 'Correct the repository-relative planning artifact.';
    if (
        $error_code eq $code{parse}
        && $error =~ /\breason=checkbox-outside-task-section\b/
    ) {
        $correction =
            'Move every unchecked task record before the canonical ## Dependency graph section.';
    } elsif (
        $error_code eq $code{parse}
        && $error =~ /\breason=unsupported-arguments\b/
    ) {
        $correction =
            'Invoke the validator with no argument or with only --self-test.';
    } elsif (
        $error_code eq $code{task_id}
        && $error =~ /\breason=duplicate-task\b/
    ) {
        $correction =
            'Assign every task one unique canonical TNNN identifier and update its dependency row.';
    }
    return "FAIL $error\n"
        . "REMEDY $error_code $correction\n"
        . "RERUN $error_code $rerun\n";
}

sub parse_owned_path {
    my ($raw, $id, $source, $files, $errors) = @_;
    my $path = trim($raw);

    if ($path eq '' || $path eq 'none') {
        push @$errors,
            error_record('ownership', $source, "task=$id", 'reason=empty-path');
        return;
    }

    my $backticks = () = $path =~ /`/g;
    if ($backticks == 2 && $path =~ /\A`([^`]*)`\z/) {
        $path = $1;
    } elsif ($backticks != 0 || $path =~ /['"]/) {
        push @$errors,
            error_record(
                'ownership', $source, "task=$id",
                'reason=malformed-quoting'
            );
        return;
    }

    if ($path =~ m{\A/}) {
        push @$errors,
            error_record('ownership', $source, "task=$id", 'reason=absolute-path');
        return;
    }
    if ($path =~ m{//}) {
        push @$errors,
            error_record(
                'ownership', $source, "task=$id",
                'reason=repeated-separator'
            );
        return;
    }

    my @components = split m{/}, $path, -1;
    if (grep { $_ eq '.' || $_ eq '..' } @components) {
        push @$errors,
            error_record('ownership', $source, "task=$id", 'reason=dot-component');
        return;
    }

    if (
        $path =~ /\b(?:and\s+every|listed\s+in|generated\s+paths?)\b/i
        || $path =~ /[*?\[\]{}]/
        || $path !~ m{\A[A-Za-z0-9_.@+-]+(?:/[A-Za-z0-9_.@+-]+)*\z}
    ) {
        push @$errors,
            error_record(
                'ownership', $source, "task=$id",
                'reason=unresolved-expression'
            );
        return;
    }

    if ($files->{$path}++) {
        push @$errors,
            error_record(
                'ownership', $source, "task=$id",
                'reason=duplicate-path', "path=$path"
            );
    }
}

sub validate_text {
    my ($text, $source) = @_;
    my @errors;

    my @sections = $text =~ /^## Dependency graph\s*$/mg;
    if (@sections != 1) {
        push @errors,
            error_record(
                'section', $source,
                'reason=' . (@sections ? 'duplicate' : 'missing')
            );
    }
    my ($task_text, $adjacency_text) =
        split /^## Dependency graph\s*$/m, $text, 2;
    $task_text = $text if !defined $task_text;
    $adjacency_text = '' if !defined $adjacency_text;

    # Census every unchecked Markdown task-like checkbox in the entire
    # document before attempting to parse records. A malformed ID or marker,
    # or a task moved below the dependency graph, must not disappear.
    my @all_checkboxes = $text =~ /^([ \t]*[-*+][ \t]*\[[ \t]*\].*)$/mg;
    my @checkboxes = $task_text =~ /^([ \t]*[-*+][ \t]*\[[ \t]*\].*)$/mg;
    my $checkbox_outside_task_section = @all_checkboxes != @checkboxes;
    if ($checkbox_outside_task_section) {
        push @errors,
            error_record(
                'parse', $source,
                'reason=checkbox-outside-task-section',
                'all=' . scalar(@all_checkboxes),
                'task-section=' . scalar(@checkboxes)
            );
    }
    my @canonical_headers;
    for my $checkbox (@all_checkboxes) {
        if ($checkbox =~ /\A- \[ \] (T[0-9]{3})(?=\s|\z)/) {
            push @canonical_headers, $1;
        } else {
            push @errors,
                error_record(
                    'task_id', $source,
                    'reason=malformed-header'
                );
        }
    }

    my @records =
        $task_text =~ /(^- \[ \] T[0-9]{3}\b.*?)(?=^[ \t]*[-*+][ \t]*\[[ \t]*\]|\z)/msg;
    my (@tasks, %seen);
    for my $record (@records) {
        if (
            $record !~
            /\A-\x20\[\x20\]\x20(T[0-9]{3})\s+
              \[owner:\s*([^\]\r\n]+)\]\s+
              \[files:\s*(.*?)\]\s+
              \[depends:\s*([^\]\r\n]+)\](?=\s|\z)(.*)\z/sx
        ) {
            my ($id) = $record =~ /\A- \[ \] (T[0-9]{3})\b/;
            push @errors,
                error_record(
                    'parse', $source,
                    'task=' . ($id // 'unknown'),
                    'reason=malformed-record'
                );
            next;
        }

        my ($id, $owner, $raw_files, $raw_depends, $remainder) =
            ($1, $2, $3, $4, $5);
        if ($remainder =~ /\[\s*(?:owner|files|depends)\s*:/i) {
            push @errors,
                error_record(
                    'parse', $source, "task=$id",
                    'reason=repeated-metadata-field'
                );
            next;
        }
        if ($seen{$id}++) {
            push @errors,
                error_record(
                    'task_id', $source, "task=$id",
                    'reason=duplicate-task'
                );
        }

        $owner = trim($owner);
        if ($owner !~ /\A[a-z0-9-]+\z/) {
            push @errors,
                error_record('owner', $source, "task=$id", 'reason=malformed');
        }

        my %files;
        my @file_items = split /,/, $raw_files, -1;
        if (@file_items == 1 && trim($file_items[0]) eq 'none') {
            @file_items = ();
        }
        parse_owned_path($_, $id, $source, \%files, \@errors)
            for @file_items;

        my (@depends, %depends_seen);
        if (trim($raw_depends) ne 'none') {
            for my $raw (split /,/, $raw_depends, -1) {
                my $dependency = trim($raw);
                if ($dependency !~ /\AT[0-9]{3}\z/) {
                    push @errors,
                        error_record(
                            'dependency', $source, "task=$id",
                            'reason=malformed'
                        );
                    next;
                }
                if ($depends_seen{$dependency}++) {
                    push @errors,
                        error_record(
                            'dependency', $source, "task=$id",
                            'reason=duplicate', "dependency=$dependency"
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
        };
    }

    if (
        !$checkbox_outside_task_section
        && (
            @records != @all_checkboxes
            || @canonical_headers != @all_checkboxes
            || @tasks != @all_checkboxes
        )
    ) {
        push @errors,
            error_record(
                'parse', $source,
                'reason=task-record-census',
                'checkboxes=' . scalar(@all_checkboxes),
                'canonical=' . scalar(@canonical_headers),
                'records=' . scalar(@records),
                'parsed=' . scalar(@tasks)
            );
    }

    my %by_id = map { $_->{id} => $_ } @tasks;
    my %graph = map {
        $_->{id} => { map { $_ => 1 } @{$_->{depends}} }
    } @tasks;
    for my $task (@tasks) {
        for my $dependency (@{$task->{depends}}) {
            if (!exists $by_id{$dependency}) {
                push @errors,
                    error_record(
                        'dependency', $source, "task=$task->{id}",
                        'reason=missing', "dependency=$dependency"
                    );
                next;
            }
            if ($by_id{$dependency}->{order} >= $task->{order}) {
                push @errors,
                    error_record(
                        'dependency', $source, "task=$task->{id}",
                        'reason=not-earlier', "dependency=$dependency"
                    );
            }
        }
    }

    my (%rows, %row_seen);
    while ($adjacency_text =~ /^(T[0-9]{3})\s+<-\s+(.+)$/mg) {
        my ($id, $raw) = ($1, $2);
        if ($row_seen{$id}++) {
            push @errors,
                error_record(
                    'adjacency', $source, "task=$id",
                    'reason=duplicate-row'
                );
        }
        my (%dependencies, %adjacency_seen);
        if (trim($raw) ne 'none') {
            for my $value (split /,/, $raw, -1) {
                my $dependency = trim($value);
                if ($dependency !~ /\AT[0-9]{3}\z/) {
                    push @errors,
                        error_record(
                            'adjacency', $source, "task=$id",
                            'reason=malformed'
                        );
                    next;
                }
                if ($adjacency_seen{$dependency}++) {
                    push @errors,
                        error_record(
                            'adjacency', $source, "task=$id",
                            'reason=duplicate-dependency',
                            "dependency=$dependency"
                        );
                    next;
                }
                $dependencies{$dependency} = 1;
            }
        }
        $rows{$id} = \%dependencies;
    }
    for my $id (sort keys %by_id) {
        if (!exists $rows{$id}) {
            push @errors,
                error_record('adjacency', $source, "task=$id", 'reason=missing-row');
            next;
        }
        my $inline = join ',', sort keys %{$graph{$id}};
        my $row = join ',', sort keys %{$rows{$id}};
        if ($inline ne $row) {
            push @errors,
                error_record('adjacency', $source, "task=$id", 'reason=mismatch');
        }
    }
    for my $id (sort keys %rows) {
        if (!exists $by_id{$id}) {
            push @errors,
                error_record('adjacency', $source, "task=$id", 'reason=extra-row');
        }
    }

    my (%visiting, %visited);
    my $visit;
    $visit = sub {
        my ($id) = @_;
        return if $visited{$id} || !exists $graph{$id};
        if ($visiting{$id}) {
            push @errors, error_record('cycle', $source, 'reason=detected');
            return;
        }
        $visiting{$id} = 1;
        $visit->($_) for sort keys %{$graph{$id}};
        delete $visiting{$id};
        $visited{$id} = 1;
    };
    $visit->($_) for sort keys %graph;

    my %fatal_graph_code = map { $_ => 1 } (
        $code{parse}, $code{task_id}, $code{dependency}, $code{cycle},
    );
    my $graph_invalid = grep {
        my ($error_code) = split / /, $_, 2;
        $fatal_graph_code{$error_code};
    } @errors;

    if (!$graph_invalid) {
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
                            'conflict', $source,
                            "tasks=$left->{id},$right->{id}",
                            "path=$overlap[0]"
                        );
                }
            }
        }
    }

    return (\@errors, scalar @tasks);
}

sub run_self_tests {
    my @cases = (
        ['positive.md',              undef,              undef],
        ['malformed-header.md',      $code{task_id},     'reason=malformed-header'],
        ['dot-alias.md',             $code{ownership},   'reason=dot-component'],
        ['dotdot-alias.md',          $code{ownership},   'reason=dot-component'],
        ['absolute-path.md',         $code{ownership},   'reason=absolute-path'],
        ['repeated-separator.md',    $code{ownership},   'reason=repeated-separator'],
        ['malformed-quoting.md',     $code{ownership},   'reason=malformed-quoting'],
        ['duplicate-path.md',        $code{ownership},   'reason=duplicate-path'],
        ['parser-omission.md',       $code{parse},       'reason=malformed-record'],
        ['repeated-metadata-field.md', $code{parse},     'reason=repeated-metadata-field'],
        ['task-after-graph.md',       $code{parse},       'reason=checkbox-outside-task-section'],
        ['dependency-failure.md',    $code{dependency},  'reason=missing'],
        ['adjacency-mismatch.md',    $code{adjacency},   'reason=mismatch'],
        ['cycle.md',                 $code{cycle},       'reason=detected'],
        ['concurrent-conflict.md',   $code{conflict},    undef],
        ['dynamic-ownership.md',     $code{ownership},   'reason=unresolved-expression'],
    );
    my $fixture_dir = File::Spec->catdir($Bin, 'validator-fixtures');
    for my $case (@cases) {
        my ($name, $expected_code, $expected_reason) = @$case;
        my $source =
            "specs/003-adr052-bazel-rust/tools/validator-fixtures/$name";
        my $text = read_repository_file(File::Spec->catfile($fixture_dir, $name));
        if (!defined $text) {
            print STDERR render_error(
                error_record('read', $source, 'reason=unreadable')
            );
            return 1;
        }
        my ($errors) = validate_text($text, $source);
        if (!defined $expected_code) {
            if (@$errors) {
                print STDERR render_error(
                    error_record('parse', $source, 'reason=unexpected-positive-failure')
                );
                return 1;
            }
            next;
        }
        my $found = grep {
            index($_, "$expected_code ") == 0
                && (!defined $expected_reason || index($_, $expected_reason) >= 0)
        } @$errors;
        if (!$found) {
            print STDERR render_error(
                error_record('parse', $source, 'reason=expected-failure-missing')
            );
            return 1;
        }
        if ($name eq 'task-after-graph.md') {
            my ($outside_error) = grep {
                index($_, 'reason=checkbox-outside-task-section') >= 0
            } @$errors;
            my $rendered = render_error($outside_error);
            my $expected =
                "REMEDY $code{parse} Move every unchecked task record before "
                . "the canonical ## Dependency graph section.\n";
            if (index($rendered, $expected) < 0) {
                print STDERR render_error(
                    error_record('parse', $source, 'reason=diagnostic-contract')
                );
                return 1;
            }
        }
    }

    for my $error_code (sort values %code) {
        my $rendered = render_error(
            "$error_code source=$tasks_path reason=self-test"
        );
        if (
            index($rendered, "REMEDY $error_code ") < 0
            || index($rendered, "RERUN $error_code $rerun") < 0
            || $rendered =~ m{\$\!|(?:^|[ =])/|descriptor|errno|raw (?:tool|OS) output|(?:pid|uid|runId|attemptId|candidateId|tagId)=}im
        ) {
            print STDERR render_error(
                error_record('parse', $tasks_path, 'reason=diagnostic-contract')
            );
            return 1;
        }
    }

    print 'PASS: 16 validator self-tests; positive fixture accepted; '
        . "malformed header, dot and dot-dot alias, absolute path, repeated separator, "
        . "malformed quoting, duplicate path, parser omission, repeated metadata, "
        . "task after graph, dependency failure, pure adjacency mismatch, cycle, "
        . "concurrent conflict, and dynamic ownership fixtures rejected; fixed-code "
        . "remedies are repository-relative\n";
    return 0;
}

if (@ARGV) {
    if (@ARGV == 1 && $ARGV[0] eq '--self-test') {
        exit run_self_tests();
    }
    print STDERR render_error(
        error_record('parse', $tool_path, 'reason=unsupported-arguments')
    );
    exit 2;
}

my $filesystem_tasks_path = File::Spec->catfile($Bin, '..', 'tasks.md');
my $text = read_repository_file($filesystem_tasks_path);
if (!defined $text) {
    print STDERR render_error(
        error_record('read', $tasks_path, 'reason=unreadable')
    );
    exit 1;
}
my ($errors, $task_count) = validate_text($text, $tasks_path);
if (@$errors) {
    print STDERR render_error($_) for @$errors;
    exit 1;
}

print "PASS: $task_count unique tasks with exact canonical headers and owned paths; "
    . "dependencies exist and precede consumers; adjacency matches; "
    . "graph is acyclic; concurrently ready ownership is disjoint\n";
