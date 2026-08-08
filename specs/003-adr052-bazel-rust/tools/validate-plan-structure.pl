#!/usr/bin/env perl
use strict;
use warnings;
use FindBin qw($Bin);
use File::Copy qw(copy);
use File::Path qw(make_path);
use File::Spec;
use File::Temp qw(tempdir);
use Errno qw(ECHILD EINTR EINVAL);
use IPC::Open3 qw(open3);
use Scalar::Util qw(blessed openhandle);
use Symbol qw(gensym);

my $tool_path = 'specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl';
my $tasks_path = 'specs/003-adr052-bazel-rust/tasks.md';
my $rerun =
    "perl $tool_path --self-test && perl $tool_path";
my $fixture_root =
    'specs/003-adr052-bazel-rust/tools/validator-fixtures/';
my $max_record_ordinal = 999;
my $max_line_number = 9999;
my $max_cleanup_wait_attempts = 8;

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
    cleanup    => 'D2B-SPEC003-PLAN-CLEANUP',
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
    cleanup =>
        'Correct validator self-test descriptor close and child reap; verify local process resources, then rerun.',
);

my %specific_remedy = (
    'parse:checkbox-outside-task-section' =>
        'Move every unchecked task record before the canonical ## Dependency graph section.',
    'parse:no-tasks' =>
        'Provide at least one canonical unchecked task record before the dependency graph.',
    'parse:unsupported-arguments' =>
        'Invoke the validator with no argument or with only --self-test.',
    'parse:self-test-setup-temp-dir' =>
        'Correct validator self-test temporary-directory creation; verify writable temporary storage, then rerun.',
    'parse:self-test-setup-path-resolution' =>
        'Correct validator self-test executable-path resolution; restore the validator path, then rerun.',
    'parse:self-test-setup-make-path' =>
        'Correct validator self-test tools-directory creation; verify temporary-directory permissions, then rerun.',
    'parse:self-test-setup-copy' =>
        'Correct validator self-test script-copy setup; verify source readability and destination permissions, then rerun.',
    'parse:self-test-setup-mkdir' =>
        'Correct validator self-test unreadable-source fixture creation; verify temporary-directory permissions, then rerun.',
    'parse:self-test-setup-open3' =>
        'Correct validator self-test process creation; verify the Perl interpreter and validator executable, then rerun.',
    'parse:self-test-setup-subprocess' =>
        'Correct validator self-test subprocess capture and wait; verify local process resources, then rerun.',
    'cleanup:self-test-subprocess-cleanup' =>
        'Correct validator self-test descriptor close and child reap; verify local process resources, then rerun.',
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
                self-test-setup-temp-dir
                self-test-setup-path-resolution
                self-test-setup-make-path
                self-test-setup-copy
                self-test-setup-mkdir
                self-test-setup-open3
                self-test-setup-subprocess
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
    cleanup => { map { $_ => 1 } qw(self-test-subprocess-cleanup) },
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
    actual-task-omitted-from-census.md
    census-missing.md
    census-duplicate.md
    census-malformed.md
    census-malformed-marker.md
    census-unbalanced-marker.md
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

sub physical_line_of {
    my ($text, $needle, $wanted_occurrence) = @_;
    my $occurrence = 0;
    my $line_number = 0;
    for my $line (split /\n/, $text, -1) {
        $line_number++;
        $line =~ s/\r\z//;
        next unless $line eq $needle;
        $occurrence++;
        return $line_number if $occurrence == $wanted_occurrence;
    }
    return;
}

sub canonical_task_ordinal_for {
    my ($text, $wanted_id) = @_;
    my $ordinal = 0;
    for my $line (split /\n/, $text, -1) {
        $line =~ s/\r\z//;
        next unless $line =~ /\A- \[ \] (T[0-9]{3})(?=\s|\z)/;
        $ordinal++;
        return $ordinal if $1 eq $wanted_id;
    }
    return;
}

sub trim {
    my ($value) = @_;
    $value =~ s/^\s+|\s+$//g;
    return $value;
}

sub error_record {
    my ($kind, $reason, $record, $line) = @_;
    $record =
        !defined($record) || $record !~ /\A[0-9]+\z/ || $record < 1
        ? 'none'
        : $record > $max_record_ordinal
        ? 'overflow'
        : $record;
    $line =
        !defined($line) || $line !~ /\A[0-9]+\z/ || $line < 1
        ? 'none'
        : $line > $max_line_number
        ? 'overflow'
        : $line;
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

sub self_test_contract_stderr {
    return render_error(
        error_record('parse', 'self-test-contract'),
        'tasks'
    );
}

sub self_test_setup_stderr {
    my ($reason) = @_;
    return render_error(
        error_record('parse', $reason),
        'tasks'
    );
}

sub self_test_cleanup_stderr {
    return render_error(
        error_record('cleanup', 'self-test-subprocess-cleanup'),
        'tasks'
    );
}

{
    package D2B::Spec003::OwnedProcessCapture;

    sub new {
        my (
            $class,
            $actual_pid,
            $stdin_fh,
            $stdout_fh,
            $stderr_fh,
            %args
        ) = @_;
        my @descriptors = ($stdin_fh, $stdout_fh, $stderr_fh);
        my @descriptor_identities =
            ref($args{descriptor_birth_identities}) eq 'ARRAY'
            ? @{$args{descriptor_birth_identities}}
            : ();
        return bless {
            actual_pid            => $actual_pid,
            supplied_pid          => exists($args{supplied_pid})
                ? $args{supplied_pid}
                : $actual_pid,
            descriptors           => \@descriptors,
            descriptor_identities => \@descriptor_identities,
            close_attempted       => [0, 0, 0],
            consuming_reap        => 0,
            reaped_pid            => undef,
        }, $class;
    }

    sub has_valid_initial_shape {
        my ($self) = @_;
        return 0
            if !defined($self->{actual_pid})
            || ref($self->{actual_pid})
            || $self->{actual_pid} !~ /\A[1-9][0-9]*\z/
            || !defined($self->{supplied_pid})
            || ref($self->{supplied_pid})
            || $self->{supplied_pid} !~ /\A[1-9][0-9]*\z/
            || $self->{supplied_pid} != $self->{actual_pid}
            || @{$self->{descriptors}} != 3
            || @{$self->{descriptor_identities}} != 3;
        for my $position (0 .. 2) {
            my $fh = $self->{descriptors}->[$position];
            my $identity = $self->{descriptor_identities}->[$position];
            return 0
                if !defined($identity)
                || ref($identity)
                || $identity !~ /\A[0-9]+\z/
                || !Scalar::Util::openhandle($fh)
                || fileno($fh) != $identity;
        }
        return 1;
    }

    sub actual_pid {
        my ($self) = @_;
        return $self->{actual_pid};
    }

    sub supplied_pid {
        my ($self) = @_;
        return $self->{supplied_pid};
    }

    sub descriptor_identity {
        my ($self, $position) = @_;
        return $self->{descriptor_identities}->[$position];
    }

    sub descriptor_handle {
        my ($self, $position) = @_;
        return $self->{descriptors}->[$position];
    }

    sub descriptor_close_attempted {
        my ($self, $position) = @_;
        return $self->{close_attempted}->[$position] ? 1 : 0;
    }

    sub attempt_descriptor_close {
        my ($self, $position, $closer) = @_;
        die "descriptor close attempted twice"
            if $self->{close_attempted}->[$position];
        $self->{close_attempted}->[$position] = 1;
        return $closer->(
            $self->{descriptors}->[$position],
            $self->{descriptor_identities}->[$position],
            $position
        );
    }

    sub all_descriptors_attempted_once {
        my ($self) = @_;
        return !grep { !$_ } @{$self->{close_attempted}};
    }

    sub all_descriptors_closed {
        my ($self) = @_;
        return !grep {
            Scalar::Util::openhandle($_)
        } @{$self->{descriptors}};
    }

    sub record_consuming_reap {
        my ($self, $waited_pid) = @_;
        return 0
            if !defined($waited_pid)
            || ref($waited_pid)
            || $waited_pid != $self->{actual_pid}
            || $self->{consuming_reap};
        $self->{consuming_reap} = 1;
        $self->{reaped_pid} = $waited_pid;
        return 1;
    }

    sub consuming_reap_recorded {
        my ($self) = @_;
        return $self->{consuming_reap} ? 1 : 0;
    }

    sub reaped_pid {
        my ($self) = @_;
        return $self->{reaped_pid};
    }
}

{
    package D2B::Spec003::SelfTestSetupFailure;

    sub new {
        my ($class, $reason, $cleanup_failed) = @_;
        return bless {
            reason         => $reason,
            cleanup_failed => $cleanup_failed ? 1 : 0,
        }, $class;
    }

    sub reason {
        my ($self) = @_;
        return $self->{reason};
    }

    sub cleanup_failed {
        my ($self) = @_;
        return $self->{cleanup_failed};
    }

    sub with_cleanup_failure {
        my ($self) = @_;
        $self->{cleanup_failed} = 1;
        return $self;
    }
}

sub default_self_test_operations {
    return {
        temp_dir => sub {
            return tempdir(CLEANUP => 1);
        },
        path_resolution => sub {
            my ($path) = @_;
            return File::Spec->rel2abs($path);
        },
        make_path => sub {
            my ($path) = @_;
            make_path($path);
            return -d $path;
        },
        copy => sub {
            return copy(@_);
        },
        mkdir => sub {
            return CORE::mkdir($_[0]);
        },
        open3 => sub {
            my (@command) = @_;
            my $stderr_fh = gensym;
            my ($stdin_fh, $stdout_fh);
            my $pid = open3(
                $stdin_fh,
                $stdout_fh,
                $stderr_fh,
                @command
            );
            my @descriptor_birth_identities = map {
                Scalar::Util::openhandle($_) ? fileno($_) : undef
            } ($stdin_fh, $stdout_fh, $stderr_fh);
            return D2B::Spec003::OwnedProcessCapture->new(
                $pid,
                $stdin_fh,
                $stdout_fh,
                $stderr_fh,
                descriptor_birth_identities =>
                    \@descriptor_birth_identities
            );
        },
        subprocess => sub {
            my ($process) = @_;
            my $stdin_closed = $process->attempt_descriptor_close(
                0,
                sub { return close $_[0]; }
            );
            die "stdin close failed" if !$stdin_closed;
            local $/;
            my $stdout_fh = $process->descriptor_handle(1);
            my $stderr_fh = $process->descriptor_handle(2);
            my $stdout = <$stdout_fh> // '';
            my $stderr = <$stderr_fh> // '';
            my $stdout_closed = $process->attempt_descriptor_close(
                1,
                sub { return close $_[0]; }
            );
            die "stdout close failed" if !$stdout_closed;
            my $stderr_closed = $process->attempt_descriptor_close(
                2,
                sub { return close $_[0]; }
            );
            die "stderr close failed" if !$stderr_closed;
            my $pid = $process->actual_pid();
            waitpid($pid, 0) == $pid or die "subprocess wait failed";
            my $wait_status = $?;
            $process->record_consuming_reap($pid)
                or die "subprocess reap ownership failed";
            my $status =
                $wait_status == -1 || ($wait_status & 127)
                ? 255
                : $wait_status >> 8;
            return [$status, $stdout, $stderr];
        },
        cleanup_close => sub {
            my ($fh) = @_;
            return close $fh;
        },
        cleanup_wait => sub {
            my ($pid) = @_;
            local $! = 0;
            my $waited = waitpid($pid, 0);
            return [$waited, 0 + $!];
        },
    };
}

sub self_test_operations {
    my ($overrides) = @_;
    my $operations = default_self_test_operations();
    if (defined $overrides) {
        $operations->{$_} = $overrides->{$_} for keys %$overrides;
    }
    return $operations;
}

sub run_setup_boundary {
    my ($reason, $operation) = @_;
    my $result;
    my $completed = eval {
        local $SIG{__WARN__} = sub { die $_[0]; };
        $result = $operation->();
        1;
    };
    if (!$completed) {
        die D2B::Spec003::SelfTestSetupFailure->new($reason);
    }
    return $result;
}

sub is_plain_nonempty_scalar {
    my ($value) = @_;
    return defined($value) && !ref($value) && $value && length($value) > 0;
}

sub create_self_test_temp_dir {
    my ($operations) = @_;
    return run_setup_boundary(
        'self-test-setup-temp-dir',
        sub {
            my $path = $operations->{temp_dir}->();
            die "temporary directory unavailable"
                if !is_plain_nonempty_scalar($path) || !-d $path;
            return $path;
        }
    );
}

sub resolve_self_test_path {
    my ($operations, $path) = @_;
    return run_setup_boundary(
        'self-test-setup-path-resolution',
        sub {
            die "path unavailable" if !is_plain_nonempty_scalar($path);
            my $resolved = $operations->{path_resolution}->($path);
            die "path unresolved"
                if !is_plain_nonempty_scalar($resolved) || !-f $resolved;
            return $resolved;
        }
    );
}

sub create_self_test_tools_dir {
    my ($operations, $path) = @_;
    return run_setup_boundary(
        'self-test-setup-make-path',
        sub {
            my $created = $operations->{make_path}->($path);
            die "tools directory unavailable"
                if !defined($created) || ref($created) || !$created || !-d $path;
            return $path;
        }
    );
}

sub copy_self_test_script {
    my ($operations, $source, $destination) = @_;
    return run_setup_boundary(
        'self-test-setup-copy',
        sub {
            my $copied = $operations->{copy}->($source, $destination);
            die "script copy failed"
                if !defined($copied) || ref($copied) || !$copied || !-f $destination;
            return $destination;
        }
    );
}

sub create_unreadable_source_fixture {
    my ($operations, $path) = @_;
    return run_setup_boundary(
        'self-test-setup-mkdir',
        sub {
            my $created = $operations->{mkdir}->($path);
            die "unreadable-source fixture unavailable"
                if !defined($created) || ref($created) || !$created || !-d $path;
            return $path;
        }
    );
}

sub cleanup_failed_subprocess_capture {
    my ($operations, $process) = @_;
    return 1 if !defined $process;
    my $cleanup_ok = 1;

    for my $position (0 .. 2) {
        next if $process->descriptor_close_attempted($position);
        my $closed;
        my $completed = eval {
            local $SIG{__WARN__} = sub { die $_[0]; };
            $closed = $process->attempt_descriptor_close(
                $position,
                $operations->{cleanup_close}
            );
            1;
        };
        $cleanup_ok = 0
            if !$completed || !defined($closed) || ref($closed) || !$closed;
    }
    $cleanup_ok = 0
        if !$process->all_descriptors_attempted_once()
        || !$process->all_descriptors_closed();

    if (!$process->consuming_reap_recorded()) {
        my $pid = $process->actual_pid();
        my $wait_complete = 0;
        for (1 .. $max_cleanup_wait_attempts) {
            my $wait_result;
            my $completed = eval {
                local $SIG{__WARN__} = sub { die $_[0]; };
                $wait_result = $operations->{cleanup_wait}->($pid);
                1;
            };
            if (
                !$completed
                || ref($wait_result) ne 'ARRAY'
                || @$wait_result != 2
                || !defined($wait_result->[0])
                || ref($wait_result->[0])
                || $wait_result->[0] !~ /\A-?[0-9]+\z/
                || !defined($wait_result->[1])
                || ref($wait_result->[1])
                || $wait_result->[1] !~ /\A[0-9]+\z/
            ) {
                $cleanup_ok = 0;
                last;
            }
            my ($waited, $errno) = @$wait_result;
            if ($waited == $pid) {
                if (!$process->record_consuming_reap($waited)) {
                    $cleanup_ok = 0;
                    last;
                }
                $wait_complete = 1;
                last;
            }
            if ($waited == -1 && $errno == ECHILD) {
                if ($process->consuming_reap_recorded()) {
                    $wait_complete = 1;
                } else {
                    $cleanup_ok = 0;
                }
                last;
            }
            if ($waited == -1 && $errno == EINTR) {
                next;
            }
            $cleanup_ok = 0;
            last;
        }
        $cleanup_ok = 0 if !$wait_complete;
    }
    return $cleanup_ok;
}

sub subprocess_postconditions {
    my ($process) = @_;
    return
        $process->all_descriptors_attempted_once()
        && $process->all_descriptors_closed()
        && $process->consuming_reap_recorded();
}

sub spawn_owned_process {
    my ($operations, @command) = @_;
    my $opened;
    my $completed = eval {
        local $SIG{__WARN__} = sub { die $_[0]; };
        $opened = $operations->{open3}->(@command);
        1;
    };
    if (!$completed) {
        die D2B::Spec003::SelfTestSetupFailure->new(
            'self-test-setup-open3'
        );
    }

    my $owned =
        blessed($opened)
        && $opened->isa('D2B::Spec003::OwnedProcessCapture')
        ? $opened
        : undef;
    if (!defined($owned) || !$owned->has_valid_initial_shape()) {
        my $failure = D2B::Spec003::SelfTestSetupFailure->new(
            'self-test-setup-open3'
        );
        if (
            defined($owned)
            && !cleanup_failed_subprocess_capture($operations, $owned)
        ) {
            $failure->with_cleanup_failure();
        }
        die $failure;
    }
    return $owned;
}

sub run_subprocess {
    my ($operations, @command) = @_;
    die "missing subprocess command" if !@command;

    my $process = spawn_owned_process($operations, @command);

    my $result;
    my $completed = eval {
        $result = run_setup_boundary(
            'self-test-setup-subprocess',
            sub {
                my $captured = $operations->{subprocess}->($process);
                die "subprocess capture failed"
                    if ref($captured) ne 'ARRAY'
                    || @$captured != 3
                    || !defined($captured->[0])
                    || ref($captured->[0])
                    || $captured->[0] !~ /\A[0-9]+\z/
                    || $captured->[0] > 255
                    || !defined($captured->[1])
                    || ref($captured->[1])
                    || !defined($captured->[2])
                    || ref($captured->[2]);
                die "subprocess postcondition failed"
                    if !subprocess_postconditions($process);
                return $captured;
            }
        );
        1;
    };
    if (!$completed) {
        my $failure = $@;
        my $cleanup_ok = cleanup_failed_subprocess_capture(
            $operations,
            $process
        );
        if (
            !$cleanup_ok
            && ref($failure) eq 'D2B::Spec003::SelfTestSetupFailure'
        ) {
            $failure->with_cleanup_failure();
        }
        die $failure;
    }
    return @$result;
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
    my @lines = split /\n/, $text, -1;
    my (@begins, @ends);
    my $mention_ordinal = 0;
    for my $index (0 .. $#lines) {
        my $line = $lines[$index];
        $line =~ s/\r\z//;
        next unless $line =~ /D2B-SPEC003-PLAN-TASK-CENSUS/;
        $mention_ordinal++;
        if ($line =~ /\A\Q$begin\E[ \t]*\z/) {
            push @begins, $index;
            next;
        }
        if ($line =~ /\A\Q$end\E[ \t]*\z/) {
            push @ends, $index;
            next;
        }
        return (
            undef,
            undef,
            error_record(
                'census',
                'malformed-declaration',
                $mention_ordinal,
                $index + 1
            )
        );
    }

    if (!@begins && !@ends) {
        return (
            undef,
            undef,
            error_record('census', 'missing-declaration')
        );
    }
    if (@begins > 1 || @ends > 1) {
        my $duplicate_index =
            @begins > 1 ? $begins[1] : $ends[1];
        return (
            undef,
            undef,
            error_record(
                'census',
                'duplicate-declaration',
                2,
                $duplicate_index + 1
            )
        );
    }
    if (@begins != 1 || @ends != 1 || $begins[0] >= $ends[0]) {
        my $bad_index = @ends ? $ends[0] : $begins[0];
        return (
            undef,
            undef,
            error_record(
                'census',
                'malformed-declaration',
                1,
                $bad_index + 1
            )
        );
    }

    my (@ids, @locations);
    my %seen;
    for my $index ($begins[0] + 1 .. $ends[0] - 1) {
        my $id = $lines[$index];
        $id =~ s/\r\z//;
        my $ordinal = scalar(@ids) + 1;
        if ($id !~ /\AT[0-9]{3}\z/ || $seen{$id}++) {
            return (
                undef,
                undef,
                error_record(
                    'census',
                    'malformed-declaration',
                    $ordinal,
                    $index + 1
                )
            );
        }
        push @ids, $id;
        push @locations, {
            record => $ordinal,
            line   => $index + 1,
        };
    }
    return (\@ids, \@locations, undef);
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

    my ($expected_ids, $census_locations, $census_error) =
        parse_census_declaration($text);
    return ([$census_error], 0) if defined $census_error;

    my (@section_offsets, @section_lines);
    while ($text =~ /^## Dependency graph[ \t]*$/mg) {
        push @section_offsets, $-[0];
        push @section_lines, line_number_at($text, $-[0]);
    }
    if (@section_offsets != 1) {
        if (@section_offsets) {
            return (
                [
                    error_record(
                        'section',
                        'duplicate',
                        2,
                        $section_lines[1]
                    )
                ],
                0
            );
        }
        return ([error_record('section', 'missing')], 0);
    }
    my ($task_text, $adjacency_text) =
        split /^## Dependency graph[ \t]*$/m, $text, 2;
    $task_text //= '';
    $adjacency_text //= '';
    my $graph_offset = $section_offsets[0];
    my $adjacency_first_line =
        line_number_at($text, $graph_offset);

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
        my %actual = map { $_->{id} => 1 } @tasks;
        my $located = 0;
        for my $index (0 .. $#$expected_ids) {
            next if $actual{$expected_ids->[$index]};
            push @errors,
                error_record(
                    'census',
                    'mismatch',
                    $census_locations->[$index]->{record},
                    $census_locations->[$index]->{line}
                );
            $located = 1;
            last;
        }
        if (!$located) {
            my %expected = map { $_ => 1 } @$expected_ids;
            for my $task (@tasks) {
                next if $expected{$task->{id}};
                push @errors,
                    error_record(
                        'census',
                        'mismatch',
                        $task->{record},
                        $task->{line}
                    );
                $located = 1;
                last;
            }
        }
        push @errors, error_record('census', 'mismatch')
            if !$located;
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
            exists($by_id{$id}) ? $by_id{$id}->{record} : undef;
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
    'empty.md' => q|FAIL D2B-SPEC003-PLAN-PARSE source=specs/003-adr052-bazel-rust/tools/validator-fixtures/empty.md record=none line=none reason=no-tasks
REMEDY D2B-SPEC003-PLAN-PARSE Provide at least one canonical unchecked task record before the dependency graph.
RERUN D2B-SPEC003-PLAN-PARSE perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'whole-task-omission.md' => q|FAIL D2B-SPEC003-PLAN-CENSUS source=specs/003-adr052-bazel-rust/tools/validator-fixtures/whole-task-omission.md record=2 line=5 reason=mismatch
REMEDY D2B-SPEC003-PLAN-CENSUS Declare one independent task-ID census with exactly the canonical task IDs.
RERUN D2B-SPEC003-PLAN-CENSUS perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'actual-task-omitted-from-census.md' => q|FAIL D2B-SPEC003-PLAN-CENSUS source=specs/003-adr052-bazel-rust/tools/validator-fixtures/actual-task-omitted-from-census.md record=2 line=8 reason=mismatch
REMEDY D2B-SPEC003-PLAN-CENSUS Declare one independent task-ID census with exactly the canonical task IDs.
RERUN D2B-SPEC003-PLAN-CENSUS perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'census-missing.md' => q|FAIL D2B-SPEC003-PLAN-CENSUS source=specs/003-adr052-bazel-rust/tools/validator-fixtures/census-missing.md record=none line=none reason=missing-declaration
REMEDY D2B-SPEC003-PLAN-CENSUS Declare one independent task-ID census with exactly the canonical task IDs.
RERUN D2B-SPEC003-PLAN-CENSUS perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'census-duplicate.md' => q|FAIL D2B-SPEC003-PLAN-CENSUS source=specs/003-adr052-bazel-rust/tools/validator-fixtures/census-duplicate.md record=2 line=6 reason=duplicate-declaration
REMEDY D2B-SPEC003-PLAN-CENSUS Declare one independent task-ID census with exactly the canonical task IDs.
RERUN D2B-SPEC003-PLAN-CENSUS perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'census-malformed.md' => q|FAIL D2B-SPEC003-PLAN-CENSUS source=specs/003-adr052-bazel-rust/tools/validator-fixtures/census-malformed.md record=1 line=4 reason=malformed-declaration
REMEDY D2B-SPEC003-PLAN-CENSUS Declare one independent task-ID census with exactly the canonical task IDs.
RERUN D2B-SPEC003-PLAN-CENSUS perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'census-malformed-marker.md' => q|FAIL D2B-SPEC003-PLAN-CENSUS source=specs/003-adr052-bazel-rust/tools/validator-fixtures/census-malformed-marker.md record=1 line=3 reason=malformed-declaration
REMEDY D2B-SPEC003-PLAN-CENSUS Declare one independent task-ID census with exactly the canonical task IDs.
RERUN D2B-SPEC003-PLAN-CENSUS perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'census-unbalanced-marker.md' => q|FAIL D2B-SPEC003-PLAN-CENSUS source=specs/003-adr052-bazel-rust/tools/validator-fixtures/census-unbalanced-marker.md record=1 line=3 reason=malformed-declaration
REMEDY D2B-SPEC003-PLAN-CENSUS Declare one independent task-ID census with exactly the canonical task IDs.
RERUN D2B-SPEC003-PLAN-CENSUS perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'census-duplicate-id.md' => q|FAIL D2B-SPEC003-PLAN-CENSUS source=specs/003-adr052-bazel-rust/tools/validator-fixtures/census-duplicate-id.md record=2 line=5 reason=malformed-declaration
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
    'adjacency-extra-row.md' => q|FAIL D2B-SPEC003-PLAN-ADJACENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/adjacency-extra-row.md record=none line=13 reason=extra-row
REMEDY D2B-SPEC003-PLAN-ADJACENCY Make the dependency-graph row exactly equal the task depends field.
RERUN D2B-SPEC003-PLAN-ADJACENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'adjacency-duplicate-row.md' => q|FAIL D2B-SPEC003-PLAN-ADJACENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/adjacency-duplicate-row.md record=1 line=13 reason=duplicate-row
REMEDY D2B-SPEC003-PLAN-ADJACENCY Make the dependency-graph row exactly equal the task depends field.
RERUN D2B-SPEC003-PLAN-ADJACENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'adjacency-malformed.md' => q|FAIL D2B-SPEC003-PLAN-ADJACENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/adjacency-malformed.md record=1 line=12 reason=malformed
REMEDY D2B-SPEC003-PLAN-ADJACENCY Make the dependency-graph row exactly equal the task depends field.
RERUN D2B-SPEC003-PLAN-ADJACENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'adjacency-duplicate-dependency.md' => q|FAIL D2B-SPEC003-PLAN-ADJACENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/adjacency-duplicate-dependency.md record=2 line=15 reason=duplicate-dependency
REMEDY D2B-SPEC003-PLAN-ADJACENCY Make the dependency-graph row exactly equal the task depends field.
RERUN D2B-SPEC003-PLAN-ADJACENCY perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'adjacency-mismatch.md' => q|FAIL D2B-SPEC003-PLAN-ADJACENCY source=specs/003-adr052-bazel-rust/tools/validator-fixtures/adjacency-mismatch.md record=2 line=15 reason=mismatch
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
    'section-missing.md' => q|FAIL D2B-SPEC003-PLAN-SECTION source=specs/003-adr052-bazel-rust/tools/validator-fixtures/section-missing.md record=none line=none reason=missing
REMEDY D2B-SPEC003-PLAN-SECTION Create or retain exactly one canonical ## Dependency graph section.
RERUN D2B-SPEC003-PLAN-SECTION perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    'section-duplicate.md' => q|FAIL D2B-SPEC003-PLAN-SECTION source=specs/003-adr052-bazel-rust/tools/validator-fixtures/section-duplicate.md record=2 line=15 reason=duplicate
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
            error_record('read', 'unreadable'),
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

sub run_self_test_entrypoint {
    my (%args) = @_;
    my ($captured_stdout, $captured_stderr) = ('', '');
    my $status;
    my $completed = eval {
        local $SIG{__WARN__} = sub { die $_[0]; };
        $status =
            $args{runner}->(\$captured_stdout, \$captured_stderr);
        die "invalid self-test status"
            if !defined($status) || $status !~ /\A[0-9]+\z/;
        1;
    };
    if (!$completed) {
        my $failure = $@;
        my $failure_stderr =
            ref($failure) eq 'D2B::Spec003::SelfTestSetupFailure'
            ? self_test_setup_stderr($failure->reason())
                . ($failure->cleanup_failed()
                    ? self_test_cleanup_stderr()
                    : '')
            : self_test_contract_stderr();
        ${$args{stdout}} = '';
        ${$args{stderr}} = $failure_stderr;
        return 1;
    }
    ${$args{stdout}} .= $captured_stdout;
    ${$args{stderr}} .= $captured_stderr;
    return $status;
}

sub run_cli_entrypoint {
    my (%args) = @_;
    my $argv = $args{argv};
    my $stdout = $args{stdout};
    my $stderr = $args{stderr};

    if (@$argv) {
        if (@$argv == 1 && $argv->[0] eq '--self-test') {
            my $runner = $args{self_test_runner};
            if (!defined $runner) {
                $runner = sub {
                    return run_self_tests(
                        @_,
                        self_test_ops => $args{self_test_ops}
                    );
                };
            }
            return run_self_test_entrypoint(
                runner => $runner,
                stdout => $stdout,
                stderr => $stderr,
            );
        }
        $$stderr .= render_error(
            error_record('parse', 'unsupported-arguments'),
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
    my ($self_stdout, $self_stderr, %args) = @_;
    my $operations = self_test_operations($args{self_test_ops});
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
        ['actual-task-omitted-from-census', 'census',  'mismatch'],
        ['census-missing',               'census',     'missing-declaration'],
        ['census-duplicate',             'census',     'duplicate-declaration'],
        ['census-malformed',             'census',     'malformed-declaration'],
        ['census-malformed-marker',      'census',     'malformed-declaration'],
        ['census-unbalanced-marker',     'census',     'malformed-declaration'],
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

    my %adjacency_physical_locator = (
        'adjacency-extra-row' =>
            ['T002 <- none', 1, 'T002'],
        'adjacency-duplicate-row' =>
            ['T001 <- none', 2, 'T001'],
        'adjacency-malformed' =>
            ['T001 <- bad', 1, 'T001'],
        'adjacency-duplicate-dependency' =>
            ['T002 <- T001, T001', 1, 'T002'],
        'adjacency-mismatch' =>
            ['T002 <- none', 1, 'T002'],
    );
    my %census_physical_locator = (
        'actual-task-omitted-from-census' => [
            '- [ ] T002 [owner: beta] [files: beta/two.rs] [depends: T001] Omitted from census.',
            1,
            2,
        ],
        'census-malformed-marker' => [
            '<!-- D2B-SPEC003-PLAN-TASK-CENSUS:BEGIN -- >',
            1,
            1,
        ],
        'census-unbalanced-marker' => [
            '<!-- D2B-SPEC003-PLAN-TASK-CENSUS:BEGIN -->',
            1,
            1,
        ],
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
    if (exists $adjacency_physical_locator{$stem}) {
        my ($needle, $occurrence, $task_id) =
            @{$adjacency_physical_locator{$stem}};
        my $physical_line =
            physical_line_of($text, $needle, $occurrence);
        my $task_ordinal =
            canonical_task_ordinal_for($text, $task_id);
        my $expected_record =
            defined($task_ordinal) ? $task_ordinal : 'none';
        my ($reported_record, $reported_line) =
            $case_stderr =~
            /\brecord=([^ ]+) line=([^ ]+) reason=/;
        if (
            !defined($physical_line)
            || !defined($reported_record)
            || !defined($reported_line)
            || $reported_record ne $expected_record
            || $reported_line ne "$physical_line"
        ) {
            $$self_stderr .= render_error(
                error_record(
                    'parse',
                    'diagnostic-contract',
                    1,
                    1
                ),
                $name
            );
            return 1;
        }
    }
    if (exists $census_physical_locator{$stem}) {
        my ($needle, $occurrence, $expected_record) =
            @{$census_physical_locator{$stem}};
        my $physical_line =
            physical_line_of($text, $needle, $occurrence);
        my ($reported_record, $reported_line) =
            $case_stderr =~
            /\brecord=([^ ]+) line=([^ ]+) reason=/;
        if (
            !defined($physical_line)
            || !defined($reported_record)
            || !defined($reported_line)
            || $reported_record ne "$expected_record"
            || $reported_line ne "$physical_line"
        ) {
            $$self_stderr .= self_test_contract_stderr();
            return 1;
        }
    }
    }

    my $unsupported_expected = q|FAIL D2B-SPEC003-PLAN-PARSE source=specs/003-adr052-bazel-rust/tasks.md record=none line=none reason=unsupported-arguments
REMEDY D2B-SPEC003-PLAN-PARSE Invoke the validator with no argument or with only --self-test.
RERUN D2B-SPEC003-PLAN-PARSE perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|;
    my $script_path = resolve_self_test_path($operations, $0);
    my ($unsupported_status, $unsupported_stdout, $unsupported_stderr) =
    run_subprocess(
        $operations,
        $^X,
        $script_path,
        '--unsupported'
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

    my $read_expected = q|FAIL D2B-SPEC003-PLAN-READ source=specs/003-adr052-bazel-rust/tasks.md record=none line=none reason=unreadable
REMEDY D2B-SPEC003-PLAN-READ Restore the repository-relative source and make it readable.
RERUN D2B-SPEC003-PLAN-READ perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|;
    my $unreadable_root = create_self_test_temp_dir($operations);
    my $unreadable_tools =
        File::Spec->catdir($unreadable_root, 'tools');
    create_self_test_tools_dir($operations, $unreadable_tools);
    my $unreadable_script =
        File::Spec->catfile($unreadable_tools, 'validate-plan-structure.pl');
    copy_self_test_script(
        $operations,
        $script_path,
        $unreadable_script
    );
    my $unreadable_source =
        File::Spec->catdir($unreadable_root, 'tasks.md');
    create_unreadable_source_fixture($operations, $unreadable_source);
    my ($read_status, $read_stdout, $read_stderr) =
        run_subprocess($operations, $^X, $unreadable_script);
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

    my %setup_failure_expected = (
        'self-test-setup-temp-dir' => q|FAIL D2B-SPEC003-PLAN-PARSE source=specs/003-adr052-bazel-rust/tasks.md record=none line=none reason=self-test-setup-temp-dir
REMEDY D2B-SPEC003-PLAN-PARSE Correct validator self-test temporary-directory creation; verify writable temporary storage, then rerun.
RERUN D2B-SPEC003-PLAN-PARSE perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
        'self-test-setup-path-resolution' => q|FAIL D2B-SPEC003-PLAN-PARSE source=specs/003-adr052-bazel-rust/tasks.md record=none line=none reason=self-test-setup-path-resolution
REMEDY D2B-SPEC003-PLAN-PARSE Correct validator self-test executable-path resolution; restore the validator path, then rerun.
RERUN D2B-SPEC003-PLAN-PARSE perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
        'self-test-setup-make-path' => q|FAIL D2B-SPEC003-PLAN-PARSE source=specs/003-adr052-bazel-rust/tasks.md record=none line=none reason=self-test-setup-make-path
REMEDY D2B-SPEC003-PLAN-PARSE Correct validator self-test tools-directory creation; verify temporary-directory permissions, then rerun.
RERUN D2B-SPEC003-PLAN-PARSE perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
        'self-test-setup-copy' => q|FAIL D2B-SPEC003-PLAN-PARSE source=specs/003-adr052-bazel-rust/tasks.md record=none line=none reason=self-test-setup-copy
REMEDY D2B-SPEC003-PLAN-PARSE Correct validator self-test script-copy setup; verify source readability and destination permissions, then rerun.
RERUN D2B-SPEC003-PLAN-PARSE perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
        'self-test-setup-mkdir' => q|FAIL D2B-SPEC003-PLAN-PARSE source=specs/003-adr052-bazel-rust/tasks.md record=none line=none reason=self-test-setup-mkdir
REMEDY D2B-SPEC003-PLAN-PARSE Correct validator self-test unreadable-source fixture creation; verify temporary-directory permissions, then rerun.
RERUN D2B-SPEC003-PLAN-PARSE perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
        'self-test-setup-open3' => q|FAIL D2B-SPEC003-PLAN-PARSE source=specs/003-adr052-bazel-rust/tasks.md record=none line=none reason=self-test-setup-open3
REMEDY D2B-SPEC003-PLAN-PARSE Correct validator self-test process creation; verify the Perl interpreter and validator executable, then rerun.
RERUN D2B-SPEC003-PLAN-PARSE perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
        'self-test-setup-subprocess' => q|FAIL D2B-SPEC003-PLAN-PARSE source=specs/003-adr052-bazel-rust/tasks.md record=none line=none reason=self-test-setup-subprocess
REMEDY D2B-SPEC003-PLAN-PARSE Correct validator self-test subprocess capture and wait; verify local process resources, then rerun.
RERUN D2B-SPEC003-PLAN-PARSE perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|,
    );
    my @setup_failure_cases = (
        ['temp-dir',        'temp_dir',        'self-test-setup-temp-dir'],
        ['path-resolution', 'path_resolution', 'self-test-setup-path-resolution'],
        ['make-path',       'make_path',       'self-test-setup-make-path'],
        ['copy',            'copy',            'self-test-setup-copy'],
        ['mkdir',           'mkdir',           'self-test-setup-mkdir'],
        ['open3',           'open3',           'self-test-setup-open3'],
        ['subprocess',      'subprocess',      'self-test-setup-subprocess'],
    );
    for my $setup_case (@setup_failure_cases) {
        my ($name, $operation_key, $reason) = @$setup_case;
        for my $mode (
            qw(failure warning false undefined malformed missing-side-effect)
        ) {
            my $injected_operation;
            my $resource_capture;
            my @resource_close_attempts;
            my @resource_wait_attempts;
            my @resource_cleanup_events;
            my $resource_reaped_pid;
            if ($mode eq 'failure' || $mode eq 'warning') {
                $injected_operation = sub {
                    if ($mode eq 'warning') {
                        warn
                            "raw $name setup warning at /tmp/d2b-sensitive-self-test-path\n";
                        return;
                    }
                    die
                        "raw $name setup failure at /tmp/d2b-sensitive-self-test-path\n";
                };
            } elsif ($mode eq 'false') {
                $injected_operation = sub { return 0; };
            } elsif ($mode eq 'undefined') {
                $injected_operation = sub { return undef; };
            } elsif ($mode eq 'malformed') {
                if ($operation_key eq 'open3') {
                    $injected_operation = sub {
                        my (@command) = @_;
                        my $stderr_fh = gensym;
                        my ($stdin_fh, $stdout_fh);
                        my $actual_pid = IPC::Open3::open3(
                            $stdin_fh,
                            $stdout_fh,
                            $stderr_fh,
                            @command
                        );
                        my @descriptor_birth_identities = map {
                            fileno($_)
                        } ($stdin_fh, $stdout_fh, $stderr_fh);
                        $resource_capture =
                            D2B::Spec003::OwnedProcessCapture->new(
                                $actual_pid,
                                $stdin_fh,
                                $stdout_fh,
                                $stderr_fh,
                                supplied_pid => $actual_pid + 1000000,
                                descriptor_birth_identities =>
                                    \@descriptor_birth_identities
                            );
                        return $resource_capture;
                    };
                } else {
                    $injected_operation =
                    $operation_key eq 'subprocess'
                    ? sub { return [0, {}, '']; }
                    : sub { return { malformed => 1 }; };
                }
            } elsif ($operation_key eq 'temp_dir') {
                $injected_operation = sub {
                    return File::Spec->catfile(
                        $0,
                        'missing-temp-dir-side-effect'
                    );
                };
            } elsif ($operation_key eq 'path_resolution') {
                $injected_operation = sub {
                    return File::Spec->catfile(
                        $_[0],
                        'missing-path-resolution-side-effect'
                    );
                };
            } elsif (
                $operation_key eq 'make_path'
                || $operation_key eq 'copy'
                || $operation_key eq 'mkdir'
            ) {
                $injected_operation = sub { return 1; };
            } elsif ($operation_key eq 'open3') {
                $injected_operation = sub { return { missing => 1 }; };
            } else {
                $injected_operation = sub {
                    return [0, '', ''];
                };
            }
            my %injected_overrides = (
                $operation_key => $injected_operation,
            );
            if ($operation_key eq 'open3' && $mode eq 'malformed') {
                $injected_overrides{cleanup_close} = sub {
                    my ($fh, $identity, $position) = @_;
                    push @resource_close_attempts,
                        [$position, $identity, fileno($fh)];
                    push @resource_cleanup_events, "close:$position";
                    return close $fh;
                };
                $injected_overrides{cleanup_wait} = sub {
                    my ($pid) = @_;
                    push @resource_wait_attempts, $pid;
                    push @resource_cleanup_events, 'wait';
                    local $! = 0;
                    my $waited = waitpid($pid, 0);
                    $resource_reaped_pid = $waited if $waited == $pid;
                    return [$waited, 0 + $!];
                };
            }
            my ($failure_stdout, $failure_stderr) = ('', '');
            my $failure_status = run_cli_entrypoint(
                argv => ['--self-test'],
                self_test_runner => sub {
                    my ($runner_stdout, $runner_stderr) = @_;
                    $$runner_stdout .=
                        "sentinel $name $mode stdout /tmp/d2b-sensitive-self-test-path\n";
                    $$runner_stderr .=
                        "sentinel $name $mode stderr /tmp/d2b-sensitive-self-test-path\n";
                    return run_self_tests(
                        $runner_stdout,
                        $runner_stderr,
                        self_test_ops => \%injected_overrides,
                    );
                },
                stdout => \$failure_stdout,
                stderr => \$failure_stderr,
            );
            if (
                $failure_status != 1
                || $failure_stdout ne ''
                || $failure_stderr ne $setup_failure_expected{$reason}
            ) {
                $$self_stderr .= self_test_contract_stderr();
                return 1;
            }
            if ($operation_key eq 'open3' && $mode eq 'malformed') {
                my $resource_observation_ok =
                    defined($resource_capture)
                    && $resource_capture->actual_pid()
                        != $resource_capture->supplied_pid()
                    && @resource_close_attempts == 3
                    && @resource_wait_attempts == 1
                    && join(',', @resource_cleanup_events)
                        eq 'close:0,close:1,close:2,wait'
                    && $resource_wait_attempts[0]
                        == $resource_capture->actual_pid()
                    && defined($resource_reaped_pid)
                    && $resource_reaped_pid
                        == $resource_capture->actual_pid()
                    && $resource_capture->consuming_reap_recorded()
                    && $resource_capture->reaped_pid()
                        == $resource_capture->actual_pid()
                    && $resource_capture->all_descriptors_attempted_once()
                    && $resource_capture->all_descriptors_closed();
                for my $position (0 .. 2) {
                    $resource_observation_ok = 0
                        if !defined($resource_close_attempts[$position])
                        || $resource_close_attempts[$position]->[0]
                            != $position
                        || $resource_close_attempts[$position]->[1]
                            != $resource_capture->descriptor_identity($position)
                        || $resource_close_attempts[$position]->[2]
                            != $resource_capture->descriptor_identity($position);
                }
                if (!$resource_observation_ok) {
                    $$self_stderr .= self_test_contract_stderr();
                    return 1;
                }
            }
        }
    }

    for my $mismatch_position (0 .. 2) {
        my $resource_capture;
        my @raw_birth_identities;
        my @resource_close_attempts;
        my @resource_wait_attempts;
        my @resource_cleanup_events;
        my $resource_reaped_pid;
        my %overrides = (
            open3 => sub {
                my (@command) = @_;
                my $stderr_fh = gensym;
                my ($stdin_fh, $stdout_fh);
                my $actual_pid = IPC::Open3::open3(
                    $stdin_fh,
                    $stdout_fh,
                    $stderr_fh,
                    @command
                );
                @raw_birth_identities = map {
                    fileno($_)
                } ($stdin_fh, $stdout_fh, $stderr_fh);
                my @supplied_birth_identities = @raw_birth_identities;
                $supplied_birth_identities[$mismatch_position] += 1000000;
                $resource_capture =
                    D2B::Spec003::OwnedProcessCapture->new(
                        $actual_pid,
                        $stdin_fh,
                        $stdout_fh,
                        $stderr_fh,
                        descriptor_birth_identities =>
                            \@supplied_birth_identities
                    );
                return $resource_capture;
            },
            cleanup_close => sub {
                my ($fh, $identity, $position) = @_;
                push @resource_close_attempts,
                    [$position, $identity, fileno($fh)];
                push @resource_cleanup_events, "close:$position";
                return close $fh;
            },
            cleanup_wait => sub {
                my ($pid) = @_;
                push @resource_wait_attempts, $pid;
                push @resource_cleanup_events, 'wait';
                local $! = 0;
                my $waited = waitpid($pid, 0);
                $resource_reaped_pid = $waited if $waited == $pid;
                return [$waited, 0 + $!];
            },
        );
        my ($failure_stdout, $failure_stderr) = ('', '');
        my $failure_status = run_cli_entrypoint(
            argv => ['--self-test'],
            self_test_runner => sub {
                my ($runner_stdout, $runner_stderr) = @_;
                $$runner_stdout .=
                    "sentinel descriptor mismatch stdout /tmp/d2b-sensitive-self-test-path\n";
                $$runner_stderr .=
                    "sentinel descriptor mismatch stderr /tmp/d2b-sensitive-self-test-path\n";
                return run_self_tests(
                    $runner_stdout,
                    $runner_stderr,
                    self_test_ops => \%overrides,
                );
            },
            stdout => \$failure_stdout,
            stderr => \$failure_stderr,
        );
        my $resource_observation_ok =
            $failure_status == 1
            && $failure_stdout eq ''
            && $failure_stderr eq
                $setup_failure_expected{'self-test-setup-open3'}
            && defined($resource_capture)
            && @raw_birth_identities == 3
            && @resource_close_attempts == 3
            && @resource_wait_attempts == 1
            && join(',', @resource_cleanup_events)
                eq 'close:0,close:1,close:2,wait'
            && $resource_wait_attempts[0]
                == $resource_capture->actual_pid()
            && defined($resource_reaped_pid)
            && $resource_reaped_pid
                == $resource_capture->actual_pid()
            && $resource_capture->consuming_reap_recorded()
            && $resource_capture->reaped_pid()
                == $resource_capture->actual_pid()
            && $resource_capture->all_descriptors_attempted_once()
            && $resource_capture->all_descriptors_closed();
        for my $position (0 .. 2) {
            my $expected_supplied_identity =
                $position == $mismatch_position
                ? $raw_birth_identities[$position] + 1000000
                : $raw_birth_identities[$position];
            $resource_observation_ok = 0
                if !defined($resource_close_attempts[$position])
                || $resource_close_attempts[$position]->[0] != $position
                || $resource_close_attempts[$position]->[1]
                    != $expected_supplied_identity
                || $resource_close_attempts[$position]->[2]
                    != $raw_birth_identities[$position];
        }
        if (!$resource_observation_ok) {
            $$self_stderr .= self_test_contract_stderr();
            return 1;
        }
    }

    my $cleanup_failure_expected =
        $setup_failure_expected{'self-test-setup-subprocess'}
        . q|FAIL D2B-SPEC003-PLAN-CLEANUP source=specs/003-adr052-bazel-rust/tasks.md record=none line=none reason=self-test-subprocess-cleanup
REMEDY D2B-SPEC003-PLAN-CLEANUP Correct validator self-test descriptor close and child reap; verify local process resources, then rerun.
RERUN D2B-SPEC003-PLAN-CLEANUP perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|;
    my @cleanup_cases = (
        ['close-failure-stdin',  0, 'success',    1, $cleanup_failure_expected],
        ['close-failure-stdout', 1, 'success',    1, $cleanup_failure_expected],
        ['close-failure-stderr', 2, 'success',    1, $cleanup_failure_expected],
        ['wait-failure',     undef, 'failure',    1, $cleanup_failure_expected],
        ['wait-echild',      undef, 'echild',     1, $cleanup_failure_expected],
        [
            'wait-eintr-retry',
            undef,
            'retry-success',
            8,
            $setup_failure_expected{'self-test-setup-subprocess'},
        ],
        [
            'wait-eintr-exhaustion',
            undef,
            'retry-exhaustion',
            8,
            $cleanup_failure_expected,
        ],
    );
    for my $cleanup_case (@cleanup_cases) {
        my (
            $name,
            $failed_close_position,
            $wait_mode,
            $expected_wait_attempts,
            $expected
        ) = @$cleanup_case;
        my $owned_process;
        my @close_attempts;
        my @wait_attempts;
        my @cleanup_events;
        my $actual_reaped_pid;
        my $ninth_wait_attempted = 0;
        my %overrides = (
            subprocess => sub {
                ($owned_process) = @_;
                die
                    "raw $name primary failure at /tmp/d2b-sensitive-self-test-path\n";
            },
            cleanup_close => sub {
                my ($fh, $identity, $position) = @_;
                push @close_attempts, [$position, $identity, fileno($fh)];
                push @cleanup_events, "close:$position";
                my $closed = close $fh;
                return 0
                    if defined($failed_close_position)
                    && $position == $failed_close_position;
                return $closed;
            },
            cleanup_wait => sub {
                my ($pid) = @_;
                push @wait_attempts, $pid;
                push @cleanup_events, 'wait';
                $ninth_wait_attempted = 1 if @wait_attempts > 8;
                if ($wait_mode eq 'retry-success') {
                    return [-1, EINTR] if @wait_attempts < 8;
                } elsif ($wait_mode eq 'retry-exhaustion') {
                    if (@wait_attempts == 8) {
                        my $waited = waitpid($pid, 0);
                        $actual_reaped_pid = $waited
                            if $waited == $pid;
                    }
                    return [-1, EINTR];
                }
                my $waited = waitpid($pid, 0);
                $actual_reaped_pid = $waited if $waited == $pid;
                return [-1, EINVAL] if $wait_mode eq 'failure';
                return [-1, ECHILD] if $wait_mode eq 'echild';
                return [$waited, 0];
            },
        );
        my ($failure_stdout, $failure_stderr) = ('', '');
        my $failure_status = run_cli_entrypoint(
            argv => ['--self-test'],
            self_test_runner => sub {
                my ($runner_stdout, $runner_stderr) = @_;
                $$runner_stdout .=
                    "sentinel $name stdout /tmp/d2b-sensitive-self-test-path\n";
                $$runner_stderr .=
                    "sentinel $name stderr /tmp/d2b-sensitive-self-test-path\n";
                return run_self_tests(
                    $runner_stdout,
                    $runner_stderr,
                    self_test_ops => \%overrides,
                );
            },
            stdout => \$failure_stdout,
            stderr => \$failure_stderr,
        );
        if (
            $failure_status != 1
            || $failure_stdout ne ''
            || $failure_stderr ne $expected
        ) {
            $$self_stderr .= self_test_contract_stderr();
            return 1;
        }
        my $cleanup_observation_ok =
        defined($owned_process)
        && @close_attempts == 3
        && @wait_attempts == $expected_wait_attempts
        && !$ninth_wait_attempted
        && @wait_attempts <= 8
        && join(',', @cleanup_events)
            eq join(
                ',',
                'close:0',
                'close:1',
                'close:2',
                ('wait') x $expected_wait_attempts
            )
        && defined($actual_reaped_pid)
        && $actual_reaped_pid == $owned_process->actual_pid()
        && $owned_process->all_descriptors_attempted_once()
        && $owned_process->all_descriptors_closed();
        for my $position (0 .. 2) {
        $cleanup_observation_ok = 0
            if !defined($close_attempts[$position])
            || $close_attempts[$position]->[0] != $position
            || $close_attempts[$position]->[1]
                != $owned_process->descriptor_identity($position)
            || $close_attempts[$position]->[2]
                != $owned_process->descriptor_identity($position);
        }
        for my $wait_pid (@wait_attempts) {
        $cleanup_observation_ok = 0
            if $wait_pid != $owned_process->actual_pid();
        }
        my $should_record_reap =
        $wait_mode eq 'success'
        || $wait_mode eq 'retry-success';
        $cleanup_observation_ok = 0
        if $owned_process->consuming_reap_recorded()
            != ($should_record_reap ? 1 : 0);
        $cleanup_observation_ok = 0
        if $should_record_reap
        && (
            !defined($owned_process->reaped_pid())
            || $owned_process->reaped_pid()
                != $owned_process->actual_pid()
        );
        if (!$cleanup_observation_ok) {
        $$self_stderr .= self_test_contract_stderr();
        return 1;
        }
    }

    my @prefix_progress_cases = (
        ['prefix-0-success', 0, 'success'],
        ['prefix-0-failure', 0, 'failure'],
        ['prefix-0-1-success', 1, 'success'],
        ['prefix-0-1-failure', 1, 'failure'],
    );
    for my $prefix_case (@prefix_progress_cases) {
        my ($name, $last_prefix_position, $prefix_mode) = @$prefix_case;
        my $owned_process;
        my @raw_birth_identities;
        my @close_attempts;
        my @wait_attempts;
        my @cleanup_events;
        my $actual_reaped_pid;
        my %overrides = (
            subprocess => sub {
                ($owned_process) = @_;
                @raw_birth_identities = map {
                    fileno($owned_process->descriptor_handle($_))
                } 0 .. 2;
                for my $position (0 .. $last_prefix_position) {
                    my $close_result =
                        $owned_process->attempt_descriptor_close(
                            $position,
                            sub {
                                my ($fh, $identity, $attempted_position) = @_;
                                push @close_attempts,
                                    [
                                        'prefix',
                                        $attempted_position,
                                        $identity,
                                        fileno($fh)
                                    ];
                                push @cleanup_events,
                                    "prefix-close:$attempted_position";
                                my $closed = close $fh;
                                return $prefix_mode eq 'success'
                                    ? $closed
                                    : 0;
                            }
                        );
                    die "prefix result mismatch"
                        if $close_result
                            != ($prefix_mode eq 'success' ? 1 : 0);
                }
                die
                    "raw $name primary failure at /tmp/d2b-sensitive-self-test-path\n";
            },
            cleanup_close => sub {
                my ($fh, $identity, $position) = @_;
                push @close_attempts,
                    ['cleanup', $position, $identity, fileno($fh)];
                push @cleanup_events, "cleanup-close:$position";
                return close $fh;
            },
            cleanup_wait => sub {
                my ($pid) = @_;
                push @wait_attempts, $pid;
                push @cleanup_events, 'wait';
                local $! = 0;
                my $waited = waitpid($pid, 0);
                $actual_reaped_pid = $waited if $waited == $pid;
                return [$waited, 0 + $!];
            },
        );
        my ($failure_stdout, $failure_stderr) = ('', '');
        my $failure_status = run_cli_entrypoint(
            argv => ['--self-test'],
            self_test_runner => sub {
                my ($runner_stdout, $runner_stderr) = @_;
                $$runner_stdout .=
                    "sentinel $name stdout /tmp/d2b-sensitive-self-test-path\n";
                $$runner_stderr .=
                    "sentinel $name stderr /tmp/d2b-sensitive-self-test-path\n";
                return run_self_tests(
                    $runner_stdout,
                    $runner_stderr,
                    self_test_ops => \%overrides,
                );
            },
            stdout => \$failure_stdout,
            stderr => \$failure_stderr,
        );
        my @expected_events;
        push @expected_events,
            map { "prefix-close:$_" } 0 .. $last_prefix_position;
        push @expected_events,
            map { "cleanup-close:$_" }
                $last_prefix_position + 1 .. 2;
        push @expected_events, 'wait';
        my $prefix_observation_ok =
            $failure_status == 1
            && $failure_stdout eq ''
            && $failure_stderr eq
                $setup_failure_expected{'self-test-setup-subprocess'}
            && defined($owned_process)
            && @raw_birth_identities == 3
            && @close_attempts == 3
            && @wait_attempts == 1
            && join(',', @cleanup_events)
                eq join(',', @expected_events)
            && $wait_attempts[0] == $owned_process->actual_pid()
            && defined($actual_reaped_pid)
            && $actual_reaped_pid == $owned_process->actual_pid()
            && $owned_process->consuming_reap_recorded()
            && $owned_process->reaped_pid()
                == $owned_process->actual_pid()
            && $owned_process->all_descriptors_attempted_once()
            && $owned_process->all_descriptors_closed();
        my %attempt_count_by_position;
        for my $attempt (@close_attempts) {
            my ($owner, $position, $identity, $raw_identity) = @$attempt;
            $attempt_count_by_position{$position}++;
            my $expected_owner =
                $position <= $last_prefix_position
                ? 'prefix'
                : 'cleanup';
            $prefix_observation_ok = 0
                if $owner ne $expected_owner
                || $identity != $raw_birth_identities[$position]
                || $raw_identity != $raw_birth_identities[$position];
        }
        for my $position (0 .. 2) {
            $prefix_observation_ok = 0
                if ($attempt_count_by_position{$position} // 0) != 1;
        }
        if (!$prefix_observation_ok) {
            $$self_stderr .= self_test_contract_stderr();
            return 1;
        }
    }

    my $self_test_contract_expected = self_test_contract_stderr();
    my ($contract_stdout, $contract_stderr) = ('', '');
    my $contract_status = run_cli_entrypoint(
        argv => ['--self-test'],
        self_test_runner => sub {
            my ($runner_stdout, $runner_stderr) = @_;
            $$runner_stdout .=
                "sentinel contract stdout /tmp/d2b-sensitive-self-test-path\n";
            $$runner_stderr .=
                "sentinel contract stderr /tmp/d2b-sensitive-self-test-path\n";
            return 'invalid-status';
        },
        stdout => \$contract_stdout,
        stderr => \$contract_stderr,
    );
    if (
        $contract_status != 1
        || $contract_stdout ne ''
        || $contract_stderr ne $self_test_contract_expected
    ) {
        $$self_stderr .= self_test_contract_stderr();
        return 1;
    }

    my $oversized_record_text =
        join('', ("* [ ] T001 oversized record\n")
                x ($max_record_ordinal + 1));
    my ($oversized_record_errors) =
        validate_text($oversized_record_text);
    my $oversized_record_expected = q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tasks.md record=overflow line=1000 reason=noncanonical-task-form
REMEDY D2B-SPEC003-PLAN-TASK-ID Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|;
    if (
        @$oversized_record_errors != $max_record_ordinal + 1
        || render_error(
            $oversized_record_errors->[-1],
            'tasks'
        ) ne $oversized_record_expected
    ) {
        $$self_stderr .= render_error(
            error_record('parse', 'diagnostic-contract', 1, 1),
            'tasks'
        );
        return 1;
    }

    my $oversized_line_text =
        ("\n" x $max_line_number)
        . "* [ ] T001 oversized line\n";
    my ($oversized_line_errors) =
        validate_text($oversized_line_text);
    my $oversized_line_expected = q|FAIL D2B-SPEC003-PLAN-TASK-ID source=specs/003-adr052-bazel-rust/tasks.md record=1 line=overflow reason=noncanonical-task-form
REMEDY D2B-SPEC003-PLAN-TASK-ID Use the exact unindented - [ ] TNNN header and one unique three-digit task ID.
RERUN D2B-SPEC003-PLAN-TASK-ID perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl --self-test && perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
|;
    if (
        @$oversized_line_errors != 1
        || render_error(
            $oversized_line_errors->[0],
            'tasks'
        ) ne $oversized_line_expected
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
        'PASS: 109 validator self-tests; positive fixture accepted; '
        . '47 independent negative fixtures cover noncanonical unchecked-list forms, census declarations, '
        . 'task parsing, ownership, dependency, adjacency, section, cycle, '
        . 'and conflict fixtures rejected; full stderr byte-matched against '
        . 'independent literals; physical census/mismatch and adjacency rows '
        . 'and bounded numeric, none, and overflow locators verified; actual '
        . 'temp-dir, path-resolution, make-path, copy, mkdir, open3, and subprocess '
        . 'exceptions, warnings, false, undefined, malformed, and missing-side-effect results '
        . 'emit only their seam-specific fixed setup diagnostics after sentinel output is discarded; '
        . 'failed-subprocess owned-child, independently snapshotted three-descriptor birth identity, '
        . 'per-position rebound refusal, prefix-progress close-once, ECHILD refusal, and literal-eight '
        . 'bounded consume-reap results preserve the primary failure and add only '
        . 'the fixed cleanup code when cleanup fails; actual unreadable-source '
        . 'status 1 and unsupported-argument '
        . 'status 2 subprocesses verified; self-test-contract is reserved for '
        . "validator contract failures\n";
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
        stdout           => \$stdout,
        stderr           => \$stderr,
    );
    print STDOUT $stdout;
    print STDERR $stderr;
    return $status;
}

exit main() unless caller;
1;
