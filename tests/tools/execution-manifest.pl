#!/usr/bin/env perl
#
# The execution-manifest emitter is deliberately plumbing, not a scheduler.
# GNU Make owns the Rust DAG; this helper owns only the evidence file
# lifecycle. It uses Linux *at/* syscalls so a path is resolved once through
# an anchored descriptor and later operations stay descriptor-relative.
#
# The helper is kept dependency-free because it runs before the Rust toolchain
# is available on some CI images. JSON::PP, Fcntl, POSIX, and Time::HiRes are
# Perl core modules.

use strict;
use warnings;

use Errno qw(EACCES EAGAIN ECHILD EEXIST EINTR EINVAL ENOENT ENOSYS ENOTDIR);
use Fcntl qw(F_SETFD);
use JSON::PP ();
use POSIX qw(WNOHANG);
use Time::HiRes qw(sleep time);

use constant {
    AT_FDCWD    => -100,
    AT_REMOVEDIR => 0x200,
    FD_CLOEXEC  => 1,
    F_OFD_SETLK => 37,
    F_WRLCK     => 1,
    O_CLOEXEC   => 0x80000,
    O_CREAT     => 0x40,
    O_DIRECTORY => 0x10000,
    O_EXCL      => 0x80,
    O_NOFOLLOW  => 0x40000,
    O_PATH      => 0x200000,
    O_RDONLY    => 0,
    O_RDWR      => 0x2,
    O_WRONLY   => 0x1,
    PR_SET_CHILD_SUBREAPER => 36,
    MAX_INTERRUPT_RETRIES => 16,
    SEEK_SET    => 0,
    SHUTDOWN_GRACE_SECONDS => 10,
};

{
    package ExecutionManifest::Fatal;
    use overload '""' => sub { $_[0]->{message} }, fallback => 1;

    sub new {
        my ($class, $message) = @_;
        return bless { message => "$message" }, $class;
    }
}

package main;

my $machine = `uname -m 2>/dev/null`;
chomp $machine;

sub syscall_number {
    my ($x86, $arm) = @_;
    return $x86 if $machine =~ /^(?:x86_64|amd64)$/;
    return $arm if $machine =~ /^(?:aarch64|arm64)$/;
    fatal("unsupported Linux architecture for execution-manifest process control");
}

sub sys_openat    { syscall_number(257, 56) }
sub sys_mkdirat   { syscall_number(258, 34) }
sub sys_unlinkat  { syscall_number(263, 35) }
sub sys_renameat  { syscall_number(264, 38) }
sub sys_getdents  { syscall_number(217, 61) }
sub sys_fsync     { syscall_number(74, 82) }
sub sys_lseek     { syscall_number(8, 62) }
sub sys_openat2   { 437 }
sub sys_prctl     { syscall_number(157, 167) }

sub fatal {
    my ($message) = @_;
    die ExecutionManifest::Fatal->new($message);
}

sub safe_error_message {
    my ($error) = @_;
    return "$error"
        if ref($error) && eval { $error->isa("ExecutionManifest::Fatal") };
    return "operation failed";
}

sub close_handle {
    my ($fh) = @_;
    close $fh if defined $fh;
}

sub set_cloexec {
    my ($fh) = @_;
    fcntl($fh, F_SETFD, FD_CLOEXEC)
        or fatal("could not mark an evidence descriptor close-on-exec");
}

sub fd_handle {
    my ($fd) = @_;
    my $fh;
    open($fh, "+<&=$fd")
        or fatal("could not materialize an anchored evidence descriptor");
    set_cloexec($fh);
    return $fh;
}

sub openat_handle {
    my ($dirfh, $name, $flags, $mode) = @_;
    my $dirfd = defined($dirfh) ? fileno($dirfh) : AT_FDCWD;
    my $mutable_name = "$name";
    my $mutable_flags = 0 + $flags;
    my $mutable_mode = 0 + ($mode // 0);
    my $fd = syscall(sys_openat(), $dirfd, $mutable_name, $mutable_flags, $mutable_mode);
    return undef if $fd < 0;
    return fd_handle($fd);
}

sub open_start_handle {
    my ($absolute) = @_;
    my $start = $absolute ? "/" : ".";
    my $fh = openat_handle(undef, $start, O_PATH | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW, 0);
    return $fh if defined($fh);
    fatal("could not open the starting directory for anchored resolution");
}

sub mode_is_dir {
    my ($mode) = @_;
    return (($mode & 0170000) == 0040000);
}

sub mode_is_regular {
    my ($mode) = @_;
    return (($mode & 0170000) == 0100000);
}

sub mode_is_symlink {
    my ($mode) = @_;
    return (($mode & 0170000) == 0120000);
}

sub verify_owner_mode {
    my ($fh, $type, $mode, $label) = @_;
    my @st = stat($fh);
    fatal("$label could not be inspected") unless @st;
    my $is_type = $type eq "directory" ? mode_is_dir($st[2]) : mode_is_regular($st[2]);
    fatal("$label has an unsafe type") unless $is_type;
    fatal("$label has an unsafe owner") unless $st[4] == $>;
    fatal("$label has unsafe permissions") unless (($st[2] & 07777) == $mode);
    return \@st;
}

sub normalized_manifest_path {
    my ($raw) = @_;
    fatal("D2B_EXECUTION_MANIFEST must name a file") unless defined($raw) && length($raw);
    fatal("D2B_EXECUTION_MANIFEST contains a NUL") if $raw =~ /\0/;
    my $absolute = substr($raw, 0, 1) eq "/";
    my @parts = split m{/}, $raw, -1;
    my $base = pop @parts;
    fatal("D2B_EXECUTION_MANIFEST must name a file") unless defined($base) && length($base);
    fatal("D2B_EXECUTION_MANIFEST has an unsafe filename") if $base eq "." || $base eq "..";
    for my $part (@parts) {
        next if $part eq "" || $part eq ".";
        fatal("D2B_EXECUTION_MANIFEST rejects parent traversal") if $part eq "..";
        fatal("D2B_EXECUTION_MANIFEST has an unsafe path component")
            if $part =~ /\0/;
    }
    return ($absolute, \@parts, $base);
}

sub open_manifest_parent {
    my ($raw) = @_;
    my ($absolute, $parts, $base) = normalized_manifest_path($raw);

    # Prefer the kernel's anchored resolver. The component-walk fallback
    # below is equivalent for older kernels and still checks every descriptor.
    my @parent_parts = grep { $_ ne "" && $_ ne "." } @{$parts};
    my $parent_path = @parent_parts ? join("/", @parent_parts) : ".";
    $parent_path = "/$parent_path" if $absolute;
    my $how = pack("QQQ", O_PATH | O_DIRECTORY | O_CLOEXEC, 0, 0x06);
    my $mutable_parent_path = "$parent_path";
    my $mutable_how = "$how";
    my $openat2_fd = syscall(
        sys_openat2(),
        AT_FDCWD,
        $mutable_parent_path,
        $mutable_how,
        length($mutable_how),
    );
    if ($openat2_fd >= 0) {
        my $anchored = fd_handle($openat2_fd);
        my @st = stat($anchored);
        fatal("manifest parent could not be inspected") unless @st;
        fatal("manifest parent contains a symlink or magic link") if mode_is_symlink($st[2]);
        fatal("manifest parent is not a directory") unless mode_is_dir($st[2]);
        return ($anchored, $base);
    }
    fatal("manifest parent contains a symlink or magic link")
        unless $! == ENOSYS || $! == EINVAL || $! == ENOENT || $! == ENOTDIR;

    my $parent = open_start_handle($absolute);
    for my $part (@{$parts}) {
        next if $part eq "" || $part eq ".";
        my $next = openat_handle(
            $parent,
            $part,
            O_PATH | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW,
            0,
        );
        fatal("manifest parent contains a symlink, magic link, or non-directory")
            unless defined($next);
        my @st = stat($next);
        fatal("manifest parent could not be inspected") unless @st;
        fatal("manifest parent contains a symlink or magic link") if mode_is_symlink($st[2]);
        fatal("manifest parent contains a non-directory") unless mode_is_dir($st[2]);
        close_handle($parent);
        $parent = $next;
    }
    return ($parent, $base);
}

sub open_readable_directory {
    my ($parent, $name) = @_;
    return openat_handle(
        $parent,
        $name,
        O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW,
        0,
    );
}

sub directory_entries {
    my ($dirfh) = @_;
    my @names;
    my $interrupt_retries = 0;
    my $offset = 0;
    my $whence = SEEK_SET;
    syscall(sys_lseek(), fileno($dirfh), $offset, $whence);
    my $buffer = "\0" x 32768;
    while (1) {
        my $n = syscall(sys_getdents(), fileno($dirfh), $buffer, length($buffer));
        if ($n < 0) {
            if ($! == EINTR || $! == EAGAIN) {
                ++$interrupt_retries;
                fatal("fd-relative directory enumeration exceeded the interrupt retry limit")
                    if $interrupt_retries > MAX_INTERRUPT_RETRIES;
                next;
            }
            fatal("fd-relative directory enumeration failed");
        }
        $interrupt_retries = 0;
        last if $n == 0;
        $offset = 0;
        while ($offset < $n) {
            my ($ino, $off, $reclen, $type) =
                unpack("QQSC", substr($buffer, $offset, 19));
            fatal("invalid fd-relative directory record") unless $reclen && $reclen >= 19;
            my $name = substr($buffer, $offset + 19, $reclen - 19);
            $name =~ s/\0.*\z//s;
            push @names, $name if length($name) && $name ne "." && $name ne "..";
            $offset += $reclen;
        }
    }
    return @names;
}

sub unlinkat_name {
    my ($dirfh, $name, $flags) = @_;
    my $dirfd = fileno($dirfh);
    my $mutable_name = "$name";
    my $mutable_flags = 0 + ($flags // 0);
    my $result = syscall(sys_unlinkat(), $dirfd, $mutable_name, $mutable_flags);
    return 1 if $result == 0;
    return 0 if $! == ENOENT;
    fatal("fd-relative cleanup failed");
}

sub renameat_name {
    my ($fromfh, $from, $tofh, $to) = @_;
    my $fromfd = fileno($fromfh);
    my $tofd = fileno($tofh);
    my $mutable_from = "$from";
    my $mutable_to = "$to";
    my $result = syscall(sys_renameat(), $fromfd, $mutable_from, $tofd, $mutable_to);
    fatal("atomic fragment rename failed") if $result < 0;
}

sub make_directory {
    my ($parent, $name) = @_;
    my $parentfd = fileno($parent);
    my $mutable_name = "$name";
    my $mutable_mode = 0700;
    my $result = syscall(sys_mkdirat(), $parentfd, $mutable_name, $mutable_mode);
    if ($result < 0 && $! != EEXIST) {
        fatal("could not create the execution-manifest fragment directory");
    }
}

sub sync_handle {
    my ($fh) = @_;
    my $result = syscall(sys_fsync(), fileno($fh));
    fatal("could not flush an execution-manifest evidence file") if $result < 0;
}

sub establish_child_subreaper {
    my $result = syscall(
        sys_prctl(),
        PR_SET_CHILD_SUBREAPER,
        1,
        0,
        0,
        0,
    );
    fatal("could not establish Linux child subreaper")
        if $result < 0;
}

sub write_all {
    my ($fh, $data) = @_;
    my $offset = 0;
    my $interrupt_retries = 0;
    while ($offset < length($data)) {
        my $written = syswrite($fh, $data, length($data) - $offset, $offset);
        if (!defined($written)) {
            if ($! == EINTR || $! == EAGAIN) {
                ++$interrupt_retries;
                fatal("execution-manifest evidence write exceeded the interrupt retry limit")
                    if $interrupt_retries > MAX_INTERRUPT_RETRIES;
                next;
            }
            fatal("could not write an execution-manifest evidence file");
        }
        fatal("could not write an execution-manifest evidence file") if $written == 0;
        $interrupt_retries = 0;
        $offset += $written;
    }
}

sub drain_adopted_descendants {
    my ($process_control) = @_;
    my $interrupt_retries = 0;
    while (1) {
        # Drain every adopted child already waiting. If one is still alive,
        # wait for it after process-group termination rather than publishing
        # evidence while a scheduler descendant remains unreaped.
        my $adopted = $process_control->{waitpid}->(-1, WNOHANG);
        if ($adopted > 0) {
            $interrupt_retries = 0;
            next;
        }
        if ($adopted == 0) {
            my $reaped = $process_control->{waitpid}->(-1, 0);
            if ($reaped > 0) {
                $interrupt_retries = 0;
                next;
            }
            if ($reaped < 0 && $! == EINTR) {
                ++$interrupt_retries;
                fatal("scheduler descendant reap exceeded the interrupt retry limit")
                    if $interrupt_retries > MAX_INTERRUPT_RETRIES;
                next;
            }
            last if $reaped < 0 && $! == ECHILD;
            fatal("could not drain adopted scheduler descendants");
        }
        if ($! == EINTR) {
            ++$interrupt_retries;
            fatal("scheduler descendant reap exceeded the interrupt retry limit")
                if $interrupt_retries > MAX_INTERRUPT_RETRIES;
            next;
        }
        last if $! == ECHILD;
        fatal("could not drain adopted scheduler descendants");
    }
}

sub lock_manifest {
    my ($parent, $base) = @_;
    my $lock_name = "$base.lock";
    my $lockfh = openat_handle(
        $parent,
        $lock_name,
        O_RDWR | O_CREAT | O_CLOEXEC | O_NOFOLLOW,
        0600,
    );
    fatal("could not open the execution-manifest lock") unless defined($lockfh);
    verify_owner_mode($lockfh, "file", 0600, "execution-manifest lock");

    # Linux struct flock has four bytes of padding after the two shorts and
    # four bytes of tail padding on the supported 64-bit ABIs.
    my $request = pack("ssxxxxqqixxxx", F_WRLCK, SEEK_SET, 0, 0, 0);
    if (!defined(fcntl($lockfh, F_OFD_SETLK, $request))) {
        if ($! == EACCES || $! == EAGAIN) {
            print STDERR
                "manifest-lock-contended: execution-manifest lock is active; "
                . "wait for the active run to finish and retry.\n";
            exit 73;
        }
        fatal("could not acquire the execution-manifest lock");
    }
    return $lockfh;
}

sub remove_prior_manifest {
    my ($parent, $base) = @_;
    my $existing = openat_handle(
        $parent,
        $base,
        O_PATH | O_CLOEXEC | O_NOFOLLOW,
        0,
    );
    if (!defined($existing)) {
        return if $! == ENOENT;
        fatal("could not inspect the prior execution manifest");
    }
    verify_owner_mode($existing, "file", 0600, "prior execution manifest");
    close_handle($existing);
    unlinkat_name($parent, $base, 0);
}

sub cleanup_fragment_directory {
    my ($parent, $name, $require_empty) = @_;
    my $dirfh = open_readable_directory($parent, $name);
    return unless defined($dirfh);
    my @parent_st = stat($parent);
    my @dir_st = stat($dirfh);
    fatal("execution-manifest fragments are on a different filesystem")
        unless @parent_st && @dir_st && $parent_st[0] == $dir_st[0];
    fatal("execution-manifest fragment directory has an unsafe owner")
        unless $dir_st[4] == $>;
    fatal("execution-manifest fragment directory has unsafe permissions")
        unless (($dir_st[2] & 07777) == 0700);

    for my $entry (directory_entries($dirfh)) {
        my $entryfh = openat_handle(
            $dirfh,
            $entry,
            O_PATH | O_CLOEXEC | O_NOFOLLOW,
            0,
        );
        unless (defined($entryfh)) {
            print STDERR "execution-manifest cleanup: invalid stale entry skipped\n";
            next;
        }
        my @st = stat($entryfh);
        my $valid = @st
            && mode_is_regular($st[2])
            && $st[4] == $>
            && (($st[2] & 07777) == 0600);
        close_handle($entryfh);
        unless ($valid) {
            print STDERR "execution-manifest cleanup: invalid stale entry skipped\n";
            next;
        }
        unlinkat_name($dirfh, $entry, 0);
    }

    if ($require_empty && directory_entries($dirfh)) {
        fatal("execution-manifest cleanup found unsafe stale entries");
    }
    close_handle($dirfh);
    unlinkat_name($parent, $name, AT_REMOVEDIR) if $require_empty;
}

sub create_fragment_directory {
    my ($parent, $base) = @_;
    my $name = ".$base.fragments";
    cleanup_fragment_directory($parent, $name, 1);
    make_directory($parent, $name);
    my $dirfh = open_readable_directory($parent, $name);
    fatal("could not open the execution-manifest fragment directory")
        unless defined($dirfh);
    verify_owner_mode($dirfh, "directory", 0700, "execution-manifest fragment directory");
    my @parent_st = stat($parent);
    my @dir_st = stat($dirfh);
    fatal("execution-manifest fragments are on a different filesystem")
        unless @parent_st && @dir_st && $parent_st[0] == $dir_st[0];
    return ($dirfh, $name);
}

sub write_atomic_fragment {
    my ($manifest, $leaf, $status) = @_;
    fatal("execution-manifest leaf name is unsafe") unless $leaf =~ /\A[A-Za-z0-9_.-]+\z/;
    fatal("execution-manifest leaf status is unsafe")
        unless $status eq "passed" || $status eq "failed" || $status eq "interrupted";
    my ($parent, $base) = open_manifest_parent($manifest);
    # The run process owns the persistent lock. Fragment writers verify the
    # anchored parent and directory but do not try to acquire that lock again.
    my $dir_name = ".$base.fragments";
    my $dirfh = open_readable_directory($parent, $dir_name);
    fatal("execution-manifest fragment directory is unavailable") unless defined($dirfh);
    verify_owner_mode($dirfh, "directory", 0700, "execution-manifest fragment directory");
    my $name = ".fragment.$leaf.$$";
    my $fh = openat_handle(
        $dirfh,
        $name,
        O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC | O_NOFOLLOW,
        0600,
    );
    fatal("could not create an execution-manifest fragment") unless defined($fh);
    my $json = JSON::PP->new->canonical(1)->utf8(1)->encode({
        leaf => $leaf,
        run_status => $status,
        completed_leaves => $status eq "passed" ? [$leaf] : [],
        failed_surfaces => $status eq "passed" ? [] : [$leaf],
        installables => [],
        realized_checks => [],
    }) . "\n";
    write_all($fh, $json);
    sync_handle($fh);
    close_handle($fh);
    my $existing = openat_handle(
        $dirfh,
        "fragment.$leaf",
        O_PATH | O_CLOEXEC | O_NOFOLLOW,
        0,
    );
    if (defined($existing)) {
        close_handle($existing);
        fatal("duplicate execution-manifest leaf fragment");
    }
    fatal("could not inspect the execution-manifest leaf fragment")
        unless $! == ENOENT;
    renameat_name($dirfh, $name, $dirfh, "fragment.$leaf");
    close_handle($dirfh);
    close_handle($parent);
}

sub read_fragment {
    my ($dirfh, $name) = @_;
    fatal("execution-manifest fragment name is unsafe")
        unless $name =~ /\Afragment\.[A-Za-z0-9_.-]+\z/;
    my $fh = openat_handle($dirfh, $name, O_RDONLY | O_CLOEXEC | O_NOFOLLOW, 0);
    fatal("could not open an execution-manifest fragment") unless defined($fh);
    verify_owner_mode($fh, "file", 0600, "execution-manifest fragment");
    my $data = "";
    my $interrupt_retries = 0;
    while (1) {
        my $chunk = "";
        my $n = sysread($fh, $chunk, 65536);
        if (!defined($n)) {
            if ($! == EINTR || $! == EAGAIN) {
                ++$interrupt_retries;
                fatal("execution-manifest fragment read exceeded the interrupt retry limit")
                    if $interrupt_retries > MAX_INTERRUPT_RETRIES;
                next;
            }
            fatal("could not read an execution-manifest fragment");
        }
        $interrupt_retries = 0;
        last if $n == 0;
        $data .= $chunk;
    }
    close_handle($fh);
    my $decoded = eval { JSON::PP::decode_json($data) };
    fatal("execution-manifest fragment is not valid JSON") unless ref($decoded) eq "HASH";
    return $decoded;
}

sub finalize_manifest {
    my ($parent, $base, $dirfh, $run_status, $commit, $target) = @_;
    my (@completed, @failed, @installables, @realized);
    my %seen;
    for my $name (sort(directory_entries($dirfh))) {
        next unless $name =~ /\Afragment\./;
        my $entry = read_fragment($dirfh, $name);
        my $leaf = $entry->{leaf};
        fatal("execution-manifest fragment has no stable leaf") unless defined($leaf);
        fatal("duplicate execution-manifest leaf fragment") if $seen{$leaf}++;
        push @completed, @{$entry->{completed_leaves} // []};
        push @failed, @{$entry->{failed_surfaces} // []};
        push @installables, @{$entry->{installables} // []};
        push @realized, @{$entry->{realized_checks} // []};
    }
    push @failed, "scheduler" if $run_status eq "failed";
    push @failed, "scheduler-interrupted" if $run_status eq "interrupted";
    my %unique = map { $_ => 1 } grep { defined($_) && length($_) } @completed;
    @completed = sort keys %unique;
    %unique = map { $_ => 1 } grep { defined($_) && length($_) } @failed;
    @failed = sort keys %unique;
    %unique = map { $_ => 1 } grep { defined($_) && length($_) } @installables;
    @installables = sort keys %unique;
    %unique = map { $_ => 1 } grep { defined($_) && length($_) } @realized;
    @realized = sort keys %unique;

    my $manifest = {
        commit => "$commit",
        completed_leaves => \@completed,
        external_contention => "not-measured",
        failed_surfaces => \@failed,
        installables => \@installables,
        realized_checks => \@realized,
        run_status => $run_status,
        source_inventory_digest => "",
        target => "$target",
        version => 1,
    };
    my $json = JSON::PP->new->canonical(1)->utf8(1)->encode($manifest) . "\n";
    my $temp = ".manifest.$$";
    my $tmpfh = openat_handle(
        $dirfh,
        $temp,
        O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC | O_NOFOLLOW,
        0600,
    );
    fatal("could not create the complete execution manifest") unless defined($tmpfh);
    write_all($tmpfh, $json);
    sync_handle($tmpfh);
    close_handle($tmpfh);
    renameat_name($dirfh, $temp, $parent, $base);
}

sub finalize_and_cleanup {
    my ($parent, $base, $dirfh, $dir_name, $run_status, $commit, $target) = @_;
    # Publish before cleanup. If cleanup is interrupted, the next run verifies
    # and removes only this fixed fragment directory.
    my $error;
    eval {
        finalize_manifest($parent, $base, $dirfh, $run_status, $commit, $target);
        1;
    } or $error = $@;
    if ($error) {
        close_handle($dirfh);
        close_handle($parent);
        die $error;
    }

    # The same anchored descriptors remain open for verified fd-relative stale
    # cleanup. No stat-then-path-unlink sequence is used.
    for my $entry (directory_entries($dirfh)) {
        next unless $entry =~ /\A(?:fragment\..+|\.manifest\.\d+)\z/;
        my $entryfh = openat_handle(
            $dirfh,
            $entry,
            O_PATH | O_CLOEXEC | O_NOFOLLOW,
            0,
        );
        next unless defined($entryfh);
        my @st = stat($entryfh);
        close_handle($entryfh);
        next unless @st && mode_is_regular($st[2]) && $st[4] == $> && (($st[2] & 07777) == 0600);
        unlinkat_name($dirfh, $entry, 0);
    }
    my @remaining = directory_entries($dirfh);
    close_handle($dirfh);
    if (!@remaining) {
        unlinkat_name($parent, $dir_name, AT_REMOVEDIR);
    } else {
        print STDERR "execution-manifest cleanup: invalid stale entry skipped\n";
    }
    close_handle($parent);
}

sub run_manifest_lifecycle {
    my (%args) = @_;
    my $cmd = $args{command};
    my $manifest = $args{manifest};
    my $target = $args{target};
    my $commit = $args{commit};
    my $path_boundary = $args{path_boundary} // sub { open_manifest_parent($_[0]) };
    my $clock = $args{clock} // sub { time() };
    my $sleep_fn = $args{sleep} // sub { sleep($_[0]) };
    my $process_control = $args{process_control} // {
        fork => sub { fork() },
        kill => sub { kill($_[0], $_[1]) },
        waitpid => sub { waitpid($_[0], $_[1]) },
        subreaper => sub { establish_child_subreaper() },
    };
    my ($parent, $base) = $path_boundary->($manifest);
    my $lockfh = lock_manifest($parent, $base);
    remove_prior_manifest($parent, $base);
    my ($dirfh, $dir_name) = create_fragment_directory($parent, $base);

    my $handled_signal = 0;
    my $forwarded = 0;
    $SIG{INT} = sub { $handled_signal ||= 2 };
    $SIG{TERM} = sub { $handled_signal ||= 15 };

    my $pid = $process_control->{fork}->();
    fatal("could not create the Rust scheduler process") unless defined($pid);
    if ($pid == 0) {
        POSIX::setsid() or exit 127;
        $ENV{D2B_EXECUTION_MANIFEST} = $manifest;
        exec @{$cmd};
        exit 127;
    }
    my $status;
    while (1) {
        if ($handled_signal && !$forwarded) {
            $forwarded = 1;
            # Become the nearest reaper only for shutdown. Setting this before
            # the scheduler starts would change orphan adoption semantics
            # inside the test processes themselves.
            $process_control->{subreaper}->();
            $process_control->{kill}->($handled_signal, -$pid);
            my $deadline = $clock->() + SHUTDOWN_GRACE_SECONDS;
            while ($clock->() < $deadline) {
                my $done = $process_control->{waitpid}->($pid, WNOHANG);
                last if $done == $pid;
                $sleep_fn->(0.05);
            }
            if ($process_control->{kill}->(0, -$pid)) {
                $process_control->{kill}->(9, -$pid);
            }
            $process_control->{waitpid}->($pid, 0);
            drain_adopted_descendants($process_control);
            $status = 128 + $handled_signal;
            last;
        }
        my $done = $process_control->{waitpid}->($pid, WNOHANG);
        if ($done == $pid) {
            $status = $? == -1 ? 127 : ($? >> 8);
            $status = 128 + ($? & 127) if $? & 127;
            last;
        }
        $sleep_fn->(0.05);
    }

    my $run_status = $handled_signal ? "interrupted" : ($status == 0 ? "passed" : "failed");
    $SIG{INT} = "IGNORE";
    $SIG{TERM} = "IGNORE";
    my $finalize_error;
    eval {
        finalize_and_cleanup(
            $parent,
            $base,
            $dirfh,
            $dir_name,
            $run_status,
            $commit,
            $target,
        );
        1;
    } or $finalize_error = $@;
    close_handle($lockfh);
    if ($finalize_error) {
        # A failing finalizer is allowed to leave its own error evidence
        # behind, but never its descriptors. The normal finalizer closes
        # these handles on both its success and publication-error paths;
        # this also covers an injected/internal failure before it gets that
        # far.
        close_handle($dirfh);
        close_handle($parent);
        my $detail = safe_error_message($finalize_error);
        if ($status == 0) {
            print STDERR
                "execution-manifest: $detail; finalization failed after scheduler success; "
                . "execution evidence is unavailable; retry the target.\n";
            return 74;
        }
        print STDERR
            "execution-manifest: $detail; finalization failed; "
            . "preserving the scheduler status.\n";
    }
    return $status;
}

sub parse_options {
    my ($args) = @_;
    my %options;
    while (@{$args} && $args->[0] ne "--") {
        my $key = shift @{$args};
        fatal("unknown execution-manifest option") unless $key =~ /\A--[a-z-]+\z/;
        fatal("missing execution-manifest option value") unless @{$args};
        my $value = shift @{$args};
        $key =~ s/\A--//;
        $options{$key} = $value;
    }
    return \%options;
}

sub main {
    my $operation = shift @ARGV // "";
    if ($operation eq "fragment") {
        my $options = parse_options(\@ARGV);
        shift @ARGV if @ARGV && $ARGV[0] eq "--";
        my $manifest = $options->{manifest} // $ENV{D2B_EXECUTION_MANIFEST};
        my $leaf = $options->{leaf} // "";
        my $status = $options->{status} // "";
        fatal("fragment mode requires a manifest, leaf, and status")
            unless defined($manifest) && length($manifest) && length($leaf) && length($status);
        write_atomic_fragment($manifest, $leaf, $status);
        return 0;
    }
    if ($operation eq "run") {
        my $options = parse_options(\@ARGV);
        fatal("run mode requires a command") unless @ARGV && $ARGV[0] eq "--";
        shift @ARGV;
        my $manifest = $options->{manifest} // "";
        my $target = $options->{target} // "test-rust";
        my $commit = $options->{commit} // "unknown";
        fatal("run mode requires a manifest") unless length($manifest);
        my $status = run_manifest_lifecycle(
            command => [@ARGV],
            manifest => $manifest,
            target => $target,
            commit => $commit,
        );
        return $status;
    }
    fatal("usage: execution-manifest.pl run|fragment [options] -- command");
}

unless (caller) {
    my $status = eval { main() };
    if ($@) {
        print STDERR "execution-manifest: " . safe_error_message($@) . "\n";
        exit 1;
    }
    exit $status;
}

1;
