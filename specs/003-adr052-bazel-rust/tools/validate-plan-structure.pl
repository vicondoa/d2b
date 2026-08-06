#!/usr/bin/env perl
use strict;
use warnings;
use FindBin qw($Bin);
use File::Spec;

my $tasks_path = File::Spec->catfile($Bin, '..', 'tasks.md');
open my $fh, '<', $tasks_path or die "cannot read $tasks_path: $!\n";
local $/;
my $text = <$fh>;
close $fh or die "cannot close $tasks_path: $!\n";

my ($task_text, $adjacency_text) = split /\n## Dependency graph/, $text, 2;
my @errors;
push @errors, 'missing dependency graph section' unless defined $adjacency_text;
$adjacency_text //= '';

my @tasks;
my %seen;
while (
    $task_text =~
    /^-\s+\[\s\]\s+(T[0-9]{3})\s+
      \[owner:\s*([^\]]+)\]\s+
      \[files:\s*(.*?)\]\s+
      \[depends:\s*([^\]]+)\]/msgx
) {
    my ($id, $owner, $raw_files, $raw_depends) = ($1, $2, $3, $4);
    push @errors, "duplicate task ID: $id" if $seen{$id}++;
    $owner =~ s/^\s+|\s+$//g;
    my %files;
    for my $item (split /,/, $raw_files) {
        $item =~ s/`//g;
        $item =~ s/\s+/ /g;
        $item =~ s/^\s+|\s+$//g;
        next if $item eq 'none';
        $item =~ s/ and every .*\z//;
        $files{$item} = 1;
    }
    my @depends;
    if ($raw_depends !~ /^\s*none\s*$/) {
        @depends = map {
            my $value = $_;
            $value =~ s/^\s+|\s+$//g;
            $value;
        } split /,/, $raw_depends;
    }
    push @tasks, {
        id => $id,
        owner => $owner,
        files => \%files,
        depends => \@depends,
        order => scalar @tasks,
    };
}

my @declared = $task_text =~ /^-\s+\[\s\]\s+(T[0-9]{3})\b/mg;
push @errors,
    'task parser did not consume every task record: parsed=' . scalar(@tasks)
    . ' declared=' . scalar(@declared)
    if @tasks != @declared;

my %by_id = map { $_->{id} => $_ } @tasks;
my %graph = map { $_->{id} => { map { $_ => 1 } @{$_->{depends}} } } @tasks;
for my $task (@tasks) {
    for my $dependency (@{$task->{depends}}) {
        if (!exists $by_id{$dependency}) {
            push @errors, "$task->{id} names missing dependency $dependency";
            next;
        }
        if ($by_id{$dependency}->{order} >= $task->{order}) {
            push @errors, "$task->{id} dependency $dependency is not earlier";
        }
    }
}

my %rows;
while ($adjacency_text =~ /^(T[0-9]{3})\s+<-\s+(.+)$/mg) {
    my ($id, $raw) = ($1, $2);
    push @errors, "duplicate adjacency row: $id" if exists $rows{$id};
    my %dependencies;
    if ($raw !~ /^\s*none\s*$/) {
        for my $dependency (split /,/, $raw) {
            $dependency =~ s/^\s+|\s+$//g;
            $dependencies{$dependency} = 1;
        }
    }
    $rows{$id} = \%dependencies;
}
for my $id (sort keys %by_id) {
    if (!exists $rows{$id}) {
        push @errors, "missing adjacency row: $id";
        next;
    }
    my $inline = join ',', sort keys %{$graph{$id}};
    my $row = join ',', sort keys %{$rows{$id}};
    push @errors, "$id adjacency differs: inline=[$inline] row=[$row]"
        if $inline ne $row;
}
for my $id (sort keys %rows) {
    push @errors, "extra adjacency row: $id" unless exists $by_id{$id};
}

my (%visiting, %visited);
my $visit;
$visit = sub {
    my ($id, @path) = @_;
    return if $visited{$id};
    if ($visiting{$id}) {
        push @errors, 'dependency cycle: ' . join(' -> ', @path, $id);
        return;
    }
    $visiting{$id} = 1;
    $visit->($_, @path, $id) for keys %{$graph{$id}};
    delete $visiting{$id};
    $visited{$id} = 1;
};
$visit->($_) for keys %graph;

if (!@errors) {
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
                "concurrently ready ownership conflict: "
                . "$left->{id}/$left->{owner} and "
                . "$right->{id}/$right->{owner}: "
                . join(', ', @overlap)
                if @overlap;
        }
    }
}

if (@errors) {
    print STDERR "FAIL: $_\n" for @errors;
    exit 1;
}

print 'PASS: ' . scalar(@tasks)
    . " unique tasks; dependencies exist and precede consumers; "
    . "adjacency matches; graph is acyclic; "
    . "concurrently ready ownership is disjoint\n";
