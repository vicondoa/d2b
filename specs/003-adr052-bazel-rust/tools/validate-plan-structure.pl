#!/usr/bin/env perl
use strict;
use warnings;
use FindBin qw($Bin);
use File::Spec;

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

sub validate_text {
    my ($text) = @_;
    my @errors;
    my ($task_text, $adjacency_text) = split /\n## Dependency graph/, $text, 2;
    if (!defined $adjacency_text) {
        push @errors, "$code{section} missing-dependency-graph";
        $adjacency_text = '';
    }

    my @declared = $task_text =~ /^-\s+\[\s\]\s+(T[0-9]{3})\b/mg;
    my @records =
        $task_text =~ /(^-\s+\[\s\]\s+T[0-9]{3}\b.*?)(?=^-\s+\[\s\]\s+T[0-9]{3}\b|\z)/msg;
    my (@tasks, %seen);
    for my $record (@records) {
        if (
            $record !~
            /^-\s+\[\s\]\s+(T[0-9]{3})\s+
              \[owner:\s*([^\]]+)\]\s+
              \[files:\s*(.*?)\]\s+
              \[depends:\s*([^\]]+)\]/sx
        ) {
            my ($id) = $record =~ /^-\s+\[\s\]\s+(T[0-9]{3})\b/;
            push @errors, "$code{parse} task=" . ($id // 'unknown');
            next;
        }

        my ($id, $owner, $raw_files, $raw_depends) = ($1, $2, $3, $4);
        push @errors, "$code{task_id} duplicate-task=$id" if $seen{$id}++;
        $owner = trim($owner);
        push @errors, "$code{owner} task=$id" unless $owner =~ /\A[a-z0-9-]+\z/;

        my %files;
        my @file_items = split /,/, $raw_files, -1;
        if (@file_items == 1 && trim($file_items[0]) eq 'none') {
            @file_items = ();
        }
        for my $raw (@file_items) {
            my $path = trim($raw);
            $path =~ s/\A`|`\z//g;
            if (
                $path eq ''
                || $path eq 'none'
                || $path =~ /\b(?:and\s+every|listed\s+in|generated\s+paths?)\b/i
                || $path =~ /[*?\[\]{}]/
                || $path =~ m{\A/|(?:\A|/)\.\.(?:/|\z)}
                || $path !~ m{\A[A-Za-z0-9_.@+-]+(?:/[A-Za-z0-9_.@+-]+)*\z}
            ) {
                push @errors, "$code{ownership} task=$id unresolved-expression";
                next;
            }
            push @errors, "$code{ownership} task=$id duplicate-path=$path"
                if $files{$path}++;
        }

        my @depends;
        if (trim($raw_depends) ne 'none') {
            for my $raw (split /,/, $raw_depends, -1) {
                my $dependency = trim($raw);
                if ($dependency !~ /\AT[0-9]{3}\z/) {
                    push @errors, "$code{dependency} task=$id malformed";
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

    if (@records != @declared || @tasks != @declared) {
        push @errors,
            "$code{parse} task-record-census declared=" . scalar(@declared)
            . " records=" . scalar(@records)
            . " parsed=" . scalar(@tasks);
    }

    my %by_id = map { $_->{id} => $_ } @tasks;
    my %graph = map {
        $_->{id} => { map { $_ => 1 } @{$_->{depends}} }
    } @tasks;
    for my $task (@tasks) {
        for my $dependency (@{$task->{depends}}) {
            if (!exists $by_id{$dependency}) {
                push @errors,
                    "$code{dependency} task=$task->{id} missing=$dependency";
                next;
            }
            if ($by_id{$dependency}->{order} >= $task->{order}) {
                push @errors,
                    "$code{dependency} task=$task->{id} not-earlier=$dependency";
            }
        }
    }

    my (%rows, %row_seen);
    while ($adjacency_text =~ /^(T[0-9]{3})\s+<-\s+(.+)$/mg) {
        my ($id, $raw) = ($1, $2);
        push @errors, "$code{adjacency} duplicate-row=$id" if $row_seen{$id}++;
        my %dependencies;
        if (trim($raw) ne 'none') {
            for my $value (split /,/, $raw, -1) {
                my $dependency = trim($value);
                if ($dependency !~ /\AT[0-9]{3}\z/) {
                    push @errors, "$code{adjacency} task=$id malformed";
                    next;
                }
                $dependencies{$dependency} = 1;
            }
        }
        $rows{$id} = \%dependencies;
    }
    for my $id (sort keys %by_id) {
        if (!exists $rows{$id}) {
            push @errors, "$code{adjacency} missing-row=$id";
            next;
        }
        my $inline = join ',', sort keys %{$graph{$id}};
        my $row = join ',', sort keys %{$rows{$id}};
        push @errors, "$code{adjacency} task=$id mismatch"
            if $inline ne $row;
    }
    for my $id (sort keys %rows) {
        push @errors, "$code{adjacency} extra-row=$id"
            unless exists $by_id{$id};
    }

    my (%visiting, %visited);
    my $visit;
    $visit = sub {
        my ($id, @path) = @_;
        return if $visited{$id} || !exists $graph{$id};
        if ($visiting{$id}) {
            push @errors, "$code{cycle} detected";
            return;
        }
        $visiting{$id} = 1;
        $visit->($_, @path, $id) for sort keys %{$graph{$id}};
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
                push @errors,
                    "$code{conflict} tasks=$left->{id},$right->{id} "
                    . "path=$overlap[0]"
                    if @overlap;
            }
        }
    }

    return (\@errors, scalar @tasks);
}

sub run_self_tests {
    my @cases = (
        ['positive.md',            undef],
        ['parser-omission.md',     $code{parse}],
        ['dependency-adjacency-mismatch.md', $code{dependency}],
        ['cycle.md',               $code{cycle}],
        ['concurrent-conflict.md', $code{conflict}],
        ['dynamic-ownership.md',   $code{ownership}],
    );
    my $fixture_dir = File::Spec->catdir($Bin, 'validator-fixtures');
    for my $case (@cases) {
        my ($name, $expected) = @$case;
        my $text = read_repository_file(File::Spec->catfile($fixture_dir, $name));
        if (!defined $text) {
            print STDERR "$code{read} fixture=$name\n";
            return 1;
        }
        my ($errors) = validate_text($text);
        if (!defined $expected) {
            if (@$errors) {
                print STDERR "$code{parse} self-test=$name\n";
                return 1;
            }
            next;
        }
        my $found = grep { index($_, $expected) == 0 } @$errors;
        if (!$found) {
            print STDERR "$code{parse} self-test=$name\n";
            return 1;
        }
    }
    print 'PASS: 6 validator self-tests; positive fixture accepted; '
        . "parser omission, dependency/adjacency mismatch, cycle, concurrent conflict, "
        . "and dynamic ownership fixtures rejected\n";
    return 0;
}

if (@ARGV) {
    if (@ARGV == 1 && $ARGV[0] eq '--self-test') {
        exit run_self_tests();
    }
    print STDERR "$code{parse} unsupported-arguments\n";
    exit 2;
}

my $tasks_path = File::Spec->catfile($Bin, '..', 'tasks.md');
my $text = read_repository_file($tasks_path);
if (!defined $text) {
    print STDERR "$code{read} path=specs/003-adr052-bazel-rust/tasks.md\n";
    exit 1;
}
my ($errors, $task_count) = validate_text($text);
if (@$errors) {
    print STDERR "FAIL $_\n" for @$errors;
    exit 1;
}

print "PASS: $task_count unique tasks with exact owned paths; "
    . "dependencies exist and precede consumers; adjacency matches; "
    . "graph is acyclic; concurrently ready ownership is disjoint\n";
