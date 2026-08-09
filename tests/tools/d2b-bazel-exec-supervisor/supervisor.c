/*
 * d2b-bazel-exec-supervisor
 *
 * This is deliberately a small, single-threaded C boundary.  It is built as
 * a static test-tooling executable and is not part of the product Rust
 * workspace.  The Rust parent supplies the verified executable open file
 * description on fd 9 and a status stream on fd 8.
 *
 * The child is released only by the kernel ptrace exec event.  A status frame
 * is never inferred from the helper's exit status.
 */

#define _GNU_SOURCE

#include <errno.h>
#include <dirent.h>
#include <fcntl.h>
#include <limits.h>
#include <poll.h>
#include <sys/resource.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/ptrace.h>
#include <sys/signalfd.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

#ifndef AT_EMPTY_PATH
#define AT_EMPTY_PATH 0x1000
#endif

#ifndef PTRACE_EVENT_EXEC
#define PTRACE_EVENT_EXEC 4
#endif

#define D2B_PRIVATE_EXECUTABLE_FD 9
#define D2B_STATUS_FD 8
#define D2B_HELPER_ERROR_FD 10
#define D2B_STATUS_MAGIC "D2BS"
#define D2B_STATUS_VERSION 1
#define D2B_EXEC_ERROR_MAGIC "D2BE"
#define D2B_PROTOCOL_VERSION 1
#define D2B_EXEC_ERROR_SIZE 8
#define D2B_STATUS_HEADER_SIZE 8
#define D2B_EXEC_DEADLINE_MS 10000
#define D2B_TERM_GRACE_MS 1000

extern char **environ;

enum d2b_write_result {
  D2B_WRITE_OK = 0,
  D2B_WRITE_ERROR = -1,
  D2B_WRITE_EPIPE = -2,
};

enum d2b_status_type {
  D2B_READY = 1,
  D2B_EXECUTED = 2,
  D2B_EXITED = 3,
  D2B_SIGNALED = 4,
};

enum d2b_child_error {
  D2B_CHILD_GROUP = 1,
  D2B_CHILD_SIGNAL = 2,
  D2B_CHILD_STDIO = 3,
  D2B_CHILD_CLOEXEC = 4,
  D2B_CHILD_CLOSE = 5,
  D2B_CHILD_PTRACE = 6,
  D2B_CHILD_STOP = 7,
  D2B_CHILD_EXECVEAT = 8,
};

enum d2b_error_kind {
  D2B_CHILD_ERROR = 1,
  D2B_HELPER_ERROR = 2,
};

/*
 * Keep the public recovery vocabulary in the immutable helper.  The Rust
 * parent maps these fixed records to the repository's redacted diagnostic
 * table; no errno, pid, path, or child output crosses this boundary.
 */
static const char *const d2b_recovery_codes[] = {
    "D2B-BZLEXEC-HELPER-SIGNAL-INHERITED-IGNORED",
    "D2B-BZLEXEC-HELPER-SIGNAL-HANDOFF",
    "D2B-BZLEXEC-HELPER-ADOPT",
    "D2B-BZLEXEC-HELPER-SIGNAL-NORMALIZE",
    "D2B-BZLEXEC-HELPER-EXEC-PIPE",
    "D2B-BZLEXEC-HELPER-FORK",
    "D2B-BZLEXEC-HELPER-GROUP-ESRCH",
    "D2B-BZLEXEC-HELPER-GROUP-EPERM",
    "D2B-BZLEXEC-HELPER-GROUP-ERROR",
    "D2B-BZLEXEC-HELPER-GROUP-EARLY-EXIT",
    "D2B-BZLEXEC-HELPER-PTRACE-STOP",
    "D2B-BZLEXEC-HELPER-PTRACE-OPTIONS",
    "D2B-BZLEXEC-HELPER-PTRACE-CONT",
    "D2B-BZLEXEC-HELPER-PRE-EXEC-TERMINATION",
    "D2B-BZLEXEC-HELPER-PRE-EXEC-DEATH",
    "D2B-BZLEXEC-HELPER-PTRACE-EVENT",
    "D2B-BZLEXEC-HELPER-PTRACE-DETACH",
    "D2B-BZLEXEC-HELPER-EXEC-TIMEOUT",
    "D2B-BZLEXEC-HELPER-EXEC-PARTIAL",
    "D2B-BZLEXEC-HELPER-EXEC-OVERLONG",
    "D2B-BZLEXEC-HELPER-EXEC-UNKNOWN",
    "D2B-BZLEXEC-HELPER-EXEC-EPIPE",
    "D2B-BZLEXEC-HELPER-EXEC-IO",
    "D2B-BZLEXEC-HELPER-SIGNAL-FORWARD",
    "D2B-BZLEXEC-HELPER-DEADLINE",
    "D2B-BZLEXEC-HELPER-WAIT",
    "D2B-BZLEXEC-HELPER-REAP",
    "D2B-BZLEXEC-HELPER-TERMINAL-WRITE",
    "D2B-BZLEXEC-HELPER-STATUS-MIRROR",
    "D2B-BZLEXEC-HELPER-CLEANUP",
    "D2B-BZLEXEC-CHILD-GROUP",
    "D2B-BZLEXEC-CHILD-SIGNAL",
    "D2B-BZLEXEC-CHILD-STDIO",
    "D2B-BZLEXEC-CHILD-CLOEXEC",
    "D2B-BZLEXEC-CHILD-CLOSE",
    "D2B-BZLEXEC-CHILD-PTRACE",
    "D2B-BZLEXEC-CHILD-STOP",
    "D2B-BZLEXEC-CHILD-EXECVEAT",
};

static int d2b_helper_error_published;
static int d2b_helper_error_fd = D2B_HELPER_ERROR_FD;

static int d2b_write_error_record(int fd, unsigned char kind,
                                  unsigned char error, int64_t deadline);
static int d2b_recovery_code_number(const char *code);

static int64_t d2b_monotonic_ms(void) {
  struct timespec now;
  if (clock_gettime(CLOCK_MONOTONIC, &now) != 0) {
    return -1;
  }
  return (int64_t)now.tv_sec * 1000 + now.tv_nsec / 1000000;
}

static int d2b_remaining_ms(int64_t deadline) {
  int64_t now = d2b_monotonic_ms();
  if (now < 0 || now >= deadline) {
    return 0;
  }
  int64_t remaining = deadline - now;
  return remaining > INT_MAX ? INT_MAX : (int)remaining;
}

static void d2b_emit_code(const char *code) {
  if (d2b_helper_error_published) {
    return;
  }
  d2b_helper_error_published = 1;
  int number = d2b_recovery_code_number(code);
  int64_t now = d2b_monotonic_ms();
  int result = number < 0 || now < 0
                   ? D2B_WRITE_ERROR
                   : d2b_write_error_record(
                         d2b_helper_error_fd, D2B_HELPER_ERROR,
                         (unsigned char)number, now + D2B_EXEC_DEADLINE_MS);
  if (result != D2B_WRITE_OK) {
    /*
     * The dedicated error channel is authoritative. Closing status is an
     * independent publication-failure signal; target stderr is never used
     * for helper diagnostics.
     */
    close(d2b_helper_error_fd);
    close(D2B_STATUS_FD);
    return;
  }
  close(d2b_helper_error_fd);
}

static int d2b_write_full(int fd, const void *data, size_t length,
                          int64_t deadline) {
  const unsigned char *cursor = (const unsigned char *)data;
  size_t written = 0;
  while (written < length) {
    ssize_t result = write(fd, cursor + written, length - written);
    if (result > 0) {
      written += (size_t)result;
      continue;
    }
    if (result < 0 && errno == EINTR) {
      if (d2b_remaining_ms(deadline) == 0) {
        return D2B_WRITE_ERROR;
      }
      continue;
    }
    if (result < 0 && errno == EPIPE) {
      return D2B_WRITE_EPIPE;
    }
    if (result < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
      struct pollfd wait_fd = {.fd = fd, .events = POLLOUT};
      int wait_result;
      do {
        wait_result = poll(&wait_fd, 1, d2b_remaining_ms(deadline));
      } while (wait_result < 0 && errno == EINTR &&
               d2b_remaining_ms(deadline) > 0);
      if (wait_result < 0 || wait_result == 0) {
        return D2B_WRITE_ERROR;
      }
      if ((wait_fd.revents & POLLHUP) != 0) {
        return D2B_WRITE_EPIPE;
      }
      if ((wait_fd.revents & (POLLERR | POLLNVAL)) != 0) {
        return D2B_WRITE_ERROR;
      }
      continue;
    }
    return D2B_WRITE_ERROR;
  }
  return D2B_WRITE_OK;
}

static int d2b_write_error_record(int fd, unsigned char kind,
                                  unsigned char error, int64_t deadline) {
  unsigned char record[D2B_EXEC_ERROR_SIZE] = {
      'D', '2', 'B', 'E', D2B_PROTOCOL_VERSION, kind, 0, error};
  return d2b_write_full(fd, record, sizeof(record), deadline);
}

static int d2b_write_exec_error(int fd, enum d2b_child_error error,
                                int64_t deadline) {
  return d2b_write_error_record(fd, D2B_CHILD_ERROR, (unsigned char)error,
                                deadline);
}

static int d2b_recovery_code_number(const char *code) {
  size_t count = sizeof(d2b_recovery_codes) / sizeof(d2b_recovery_codes[0]);
  for (size_t index = 0; index < count; ++index) {
    if (strcmp(code, d2b_recovery_codes[index]) == 0) {
      return (int)index + 1;
    }
  }
  return -1;
}

static int d2b_write_frame(int fd, enum d2b_status_type type,
                           unsigned char value, int64_t deadline) {
  unsigned char frame[9] = {
      'D', '2', 'B', 'S', D2B_STATUS_VERSION, (unsigned char)type, 0, 0, value};
  size_t length = (type == D2B_EXITED || type == D2B_SIGNALED) ? 9 : 8;
  if (length == 9) {
    frame[7] = 1;
  }
  return d2b_write_full(fd, frame, length, deadline);
}

static void d2b_emit_status_write_failure(int write_result,
                                          const char *ordinary_code) {
  if (write_result == D2B_WRITE_EPIPE) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-EPIPE");
  } else {
    d2b_emit_code(ordinary_code);
  }
}

static int d2b_check_fd(int fd, int require_cloexec) {
  int flags = fcntl(fd, F_GETFD);
  if (flags < 0) {
    return -1;
  }
  if (require_cloexec && (flags & FD_CLOEXEC) == 0) {
    return -1;
  }
  return 0;
}

static const char *d2b_child_error_code(unsigned char error) {
  switch (error) {
    case D2B_CHILD_GROUP:
      return "D2B-BZLEXEC-CHILD-GROUP";
    case D2B_CHILD_SIGNAL:
      return "D2B-BZLEXEC-CHILD-SIGNAL";
    case D2B_CHILD_STDIO:
      return "D2B-BZLEXEC-CHILD-STDIO";
    case D2B_CHILD_CLOEXEC:
      return "D2B-BZLEXEC-CHILD-CLOEXEC";
    case D2B_CHILD_CLOSE:
      return "D2B-BZLEXEC-CHILD-CLOSE";
    case D2B_CHILD_PTRACE:
      return "D2B-BZLEXEC-CHILD-PTRACE";
    case D2B_CHILD_STOP:
      return "D2B-BZLEXEC-CHILD-STOP";
    case D2B_CHILD_EXECVEAT:
      return "D2B-BZLEXEC-CHILD-EXECVEAT";
    default:
      return NULL;
  }
}

static int d2b_inspect_inherited_signals(sigset_t *managed) {
  int signals[] = {SIGHUP, SIGINT, SIGTERM, SIGQUIT};
  sigset_t inherited;
  if (sigemptyset(managed) != 0 || sigemptyset(&inherited) != 0) {
    return -1;
  }
  for (size_t i = 0; i < sizeof(signals) / sizeof(signals[0]); ++i) {
    if (sigaddset(managed, signals[i]) != 0) {
      return -1;
    }
  }

  /*
   * This is observation only.  It is intentionally the first supervisor
   * operation: an inherited managed SIG_IGN is refused, never reset.
   */
  if (sigprocmask(SIG_BLOCK, NULL, &inherited) != 0) {
    return -1;
  }
  for (size_t i = 0; i < sizeof(signals) / sizeof(signals[0]); ++i) {
    struct sigaction action;
    if (sigaction(signals[i], NULL, &action) != 0) {
      return -1;
    }
    if (action.sa_handler == SIG_IGN) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-SIGNAL-INHERITED-IGNORED");
      return -2;
    }
    if (sigismember(&inherited, signals[i]) != 1) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-SIGNAL-HANDOFF");
      return -3;
    }
  }
  return 0;
}

static int d2b_normalize_signals(const sigset_t *managed, int *signal_fd) {
  struct sigaction default_action;
  memset(&default_action, 0, sizeof(default_action));
  default_action.sa_handler = SIG_DFL;
  if (sigemptyset(&default_action.sa_mask) != 0) {
    return -1;
  }

  for (int signal_number = 1; signal_number < NSIG; ++signal_number) {
    if (signal_number == SIGKILL || signal_number == SIGSTOP ||
        signal_number == SIGPIPE || signal_number == SIGCHLD) {
      continue;
    }
    if (sigaction(signal_number, &default_action, NULL) != 0 &&
        errno != EINVAL) {
      return -1;
    }
  }

  struct sigaction ignore_action = default_action;
  ignore_action.sa_handler = SIG_IGN;
  if (sigaction(SIGPIPE, &ignore_action, NULL) != 0 ||
      sigaction(SIGCHLD, &default_action, NULL) != 0) {
    return -1;
  }

  if (sigprocmask(SIG_SETMASK, managed, NULL) != 0) {
    return -1;
  }
  int flags = SFD_CLOEXEC | SFD_NONBLOCK;
  *signal_fd = signalfd(-1, managed, flags);
  return *signal_fd < 0 ? -1 : 0;
}

static void d2b_child_fail(int error_fd, enum d2b_child_error error,
                           int64_t deadline) {
  (void)d2b_write_exec_error(error_fd, error, deadline);
  _exit(127);
}

static int d2b_close_inherited_descriptors(int error_writer) {
  DIR *directory = opendir("/proc/self/fd");
  if (directory != NULL) {
    int directory_fd = dirfd(directory);
    struct dirent *entry;
    while ((entry = readdir(directory)) != NULL) {
      char *end = NULL;
      long value = strtol(entry->d_name, &end, 10);
      if (end == entry->d_name || *end != '\0' || value < 3 ||
          value > INT_MAX) {
        continue;
      }
      int descriptor = (int)value;
      if (descriptor == directory_fd ||
          descriptor == D2B_PRIVATE_EXECUTABLE_FD ||
          descriptor == error_writer) {
        continue;
      }
      if (close(descriptor) != 0 && errno != EBADF) {
        closedir(directory);
        return -1;
      }
    }
    closedir(directory);
    return 0;
  }

  struct rlimit limit;
  if (getrlimit(RLIMIT_NOFILE, &limit) != 0) {
    return -1;
  }
  rlim_t upper = limit.rlim_cur;
  if (upper == RLIM_INFINITY) {
    long configured = sysconf(_SC_OPEN_MAX);
    if (configured <= 0) {
      return -1;
    }
    upper = (rlim_t)configured;
  }
  for (rlim_t value = 3; value < upper; ++value) {
    int descriptor = (int)value;
    if (descriptor == D2B_PRIVATE_EXECUTABLE_FD ||
        descriptor == error_writer) {
      continue;
    }
    if (close(descriptor) != 0 && errno != EBADF) {
      return -1;
    }
  }
  return 0;
}

static void d2b_child_exec(int error_writer, char *const target_argv[]) {
  int64_t deadline = d2b_monotonic_ms() + D2B_EXEC_DEADLINE_MS;

  if (setpgid(0, 0) != 0) {
    d2b_child_fail(error_writer, D2B_CHILD_GROUP, deadline);
  }
  if (d2b_check_fd(STDIN_FILENO, 0) != 0 ||
      d2b_check_fd(STDOUT_FILENO, 0) != 0 ||
      d2b_check_fd(STDERR_FILENO, 0) != 0) {
    d2b_child_fail(error_writer, D2B_CHILD_STDIO, deadline);
  }
  if (fcntl(D2B_PRIVATE_EXECUTABLE_FD, F_SETFD, FD_CLOEXEC) != 0) {
    d2b_child_fail(error_writer, D2B_CHILD_CLOEXEC, deadline);
  }
  if (fcntl(error_writer, F_SETFD, FD_CLOEXEC) != 0) {
    d2b_child_fail(error_writer, D2B_CHILD_CLOEXEC, deadline);
  }
  if (d2b_close_inherited_descriptors(error_writer) != 0) {
    d2b_child_fail(error_writer, D2B_CHILD_CLOSE, deadline);
  }

  /*
   * All four libc ptrace arguments are present and explicitly pointer typed.
   * The initial stop is the sole child-release barrier.
   */
  if (ptrace(PTRACE_TRACEME, 0, (void *)0, (void *)0) != 0) {
    d2b_child_fail(error_writer, D2B_CHILD_PTRACE, deadline);
  }

  sigset_t empty;
  if (sigemptyset(&empty) != 0 || sigprocmask(SIG_SETMASK, &empty, NULL) != 0) {
    d2b_child_fail(error_writer, D2B_CHILD_SIGNAL, deadline);
  }
  struct sigaction default_action;
  memset(&default_action, 0, sizeof(default_action));
  default_action.sa_handler = SIG_DFL;
  sigemptyset(&default_action.sa_mask);
  for (int signal_number = 1; signal_number < NSIG; ++signal_number) {
    if (signal_number != SIGKILL && signal_number != SIGSTOP &&
        sigaction(signal_number, &default_action, NULL) != 0 &&
        errno != EINVAL) {
      d2b_child_fail(error_writer, D2B_CHILD_SIGNAL, deadline);
    }
  }
  if (raise(SIGSTOP) != 0) {
    d2b_child_fail(error_writer, D2B_CHILD_STOP, deadline);
  }

  /*
   * This is the only executable transition.  The private fd refers to the
   * verified open file description. There is no path reopen or descriptor
   * lookup fallback.
   */
  (void)syscall(SYS_execveat, D2B_PRIVATE_EXECUTABLE_FD, "", target_argv,
                environ, AT_EMPTY_PATH);
  d2b_child_fail(error_writer, D2B_CHILD_EXECVEAT, deadline);
}

enum d2b_exec_record_result {
  D2B_EXEC_RECORD_PENDING = 0,
  D2B_EXEC_RECORD_EMPTY_EOF = 1,
  D2B_EXEC_RECORD_COMPLETE = 2,
  D2B_EXEC_RECORD_PARTIAL = -1,
  D2B_EXEC_RECORD_OVERLONG = -2,
  D2B_EXEC_RECORD_UNKNOWN = -3,
  D2B_EXEC_RECORD_IO = -4,
  D2B_EXEC_RECORD_TIMEOUT = -5,
};

static int d2b_validate_exec_record(const unsigned char record[8]) {
  if (memcmp(record, D2B_EXEC_ERROR_MAGIC, 4) != 0 ||
      record[4] != D2B_PROTOCOL_VERSION || record[5] != 1 ||
      record[6] != 0 || d2b_child_error_code(record[7]) == NULL) {
    return -1;
  }
  return 0;
}

static int d2b_read_exec_record(int fd, unsigned char record[8], size_t *length,
                                int *eof, int *probe_complete,
                                int64_t deadline) {
  for (;;) {
    if (*length < D2B_EXEC_ERROR_SIZE) {
      unsigned char byte;
      ssize_t result = read(fd, &byte, 1);
      if (result > 0) {
        record[(*length)++] = byte;
        if (*length == D2B_EXEC_ERROR_SIZE &&
            d2b_validate_exec_record(record) != 0) {
          return D2B_EXEC_RECORD_UNKNOWN;
        }
        continue;
      }
      if (result == 0) {
        *eof = 1;
        return *length == 0 ? D2B_EXEC_RECORD_EMPTY_EOF
                            : D2B_EXEC_RECORD_PARTIAL;
      }
      if (errno == EINTR) {
        if (d2b_remaining_ms(deadline) == 0) {
          return D2B_EXEC_RECORD_TIMEOUT;
        }
        continue;
      }
      if (errno == EAGAIN || errno == EWOULDBLOCK) {
        return D2B_EXEC_RECORD_PENDING;
      }
      return D2B_EXEC_RECORD_IO;
    }

    if (*probe_complete) {
      return D2B_EXEC_RECORD_COMPLETE;
    }

    /*
     * An exact record is not complete until the close-on-exec writer has
     * closed and the one-byte probe has observed EOF.  A ninth byte is a
     * protocol overrun, even if it arrives in a later read.
     */
    unsigned char extra;
    ssize_t result = read(fd, &extra, 1);
    if (result > 0) {
      return D2B_EXEC_RECORD_OVERLONG;
    }
    if (result == 0) {
      *eof = 1;
      *probe_complete = 1;
      return D2B_EXEC_RECORD_COMPLETE;
    }
    if (errno == EINTR) {
      if (d2b_remaining_ms(deadline) == 0) {
        return D2B_EXEC_RECORD_TIMEOUT;
      }
      continue;
    }
    if (errno == EAGAIN || errno == EWOULDBLOCK) {
      return D2B_EXEC_RECORD_PENDING;
    }
    return D2B_EXEC_RECORD_IO;
  }
}

static int d2b_read_signal(int signal_fd);
static int d2b_reap_after_kill(pid_t child, int signal_fd, int *status);

static int d2b_group_kill_and_reap(pid_t child, int child_already_reaped,
                                   int signal_fd, int *status) {
  int group_result = kill(-child, SIGKILL);
  int result = 0;
  if (group_result != 0 && errno != ESRCH) {
    result = -1;
    if (kill(child, SIGKILL) != 0 && errno != ESRCH) {
      result = -1;
    }
  }
  if (child_already_reaped) {
    return result;
  }
  if (d2b_reap_after_kill(child, signal_fd, status) != 0) {
    result = -1;
  }
  return result;
}

static int d2b_direct_kill_and_reap(pid_t child, int signal_fd, int *status) {
  if (kill(child, SIGKILL) != 0 && errno != ESRCH) {
    return -1;
  }
  return d2b_reap_after_kill(child, signal_fd, status);
}

static int d2b_observe_child_exit(pid_t child) {
  siginfo_t info;
  memset(&info, 0, sizeof(info));
  if (waitid(P_PID, (id_t)child, &info, WEXITED | WNOWAIT | WNOHANG) != 0) {
    return -1;
  }
  return info.si_pid == child ? 1 : 0;
}

static int d2b_reap_after_kill(pid_t child, int signal_fd, int *status) {
  int64_t start = d2b_monotonic_ms();
  if (start < 0) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-REAP");
    return -1;
  }
  int64_t deadline = start + D2B_EXEC_DEADLINE_MS;
  while (d2b_remaining_ms(deadline) > 0) {
    pid_t waited = waitpid(child, status, WNOHANG);
    if (waited == child) {
      return 0;
    }
    if (waited < 0) {
      if (errno == EINTR) {
        continue;
      }
      d2b_emit_code("D2B-BZLEXEC-HELPER-REAP");
      return -1;
    }

    struct pollfd descriptor = {.fd = signal_fd, .events = POLLIN};
    int timeout = d2b_remaining_ms(deadline);
    if (timeout > 10) {
      timeout = 10;
    }
    int poll_result = poll(&descriptor, 1, timeout);
    if (poll_result < 0) {
      if (errno == EINTR) {
        continue;
      }
      d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-IO");
      return -1;
    }
    if (poll_result > 0 &&
        (descriptor.revents & (POLLERR | POLLHUP | POLLNVAL)) != 0) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-IO");
      return -1;
    }
    if (poll_result > 0 && (descriptor.revents & POLLIN) != 0 &&
        d2b_read_signal(signal_fd) < 0) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-IO");
      return -1;
    }
  }
  d2b_emit_code("D2B-BZLEXEC-HELPER-REAP");
  return -1;
}

static int d2b_confirm_group(pid_t child) {
  if (setpgid(child, child) != 0) {
    if (errno == ESRCH) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-GROUP-ESRCH");
    } else if (errno == EPERM) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-GROUP-EPERM");
    } else {
      d2b_emit_code("D2B-BZLEXEC-HELPER-GROUP-ERROR");
    }
    return -1;
  }
  if (getpgid(child) != child) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-GROUP-ERROR");
    return -1;
  }
  return 0;
}

static int d2b_drain_signals(int signal_fd) {
  int first_signal = 0;
  for (;;) {
    struct signalfd_siginfo info;
    ssize_t result = read(signal_fd, &info, sizeof(info));
    if (result == (ssize_t)sizeof(info)) {
      if (first_signal == 0) {
        first_signal = (int)info.ssi_signo;
      }
      continue;
    }
    if (result < 0 && errno == EINTR) {
      continue;
    }
    if (result < 0 && (errno == EAGAIN || errno == EWOULDBLOCK)) {
      return first_signal;
    }
    return -1;
  }
}

static int d2b_wait_initial_stop(pid_t child, int signal_fd, int64_t deadline,
                                 int *status) {
  while (d2b_remaining_ms(deadline) > 0) {
    int signal_number = d2b_drain_signals(signal_fd);
    if (signal_number < 0) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-IO");
      return -1;
    }
    if (signal_number != 0) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-PRE-EXEC-TERMINATION");
      return -2;
    }

    pid_t waited = waitpid(child, status, WNOHANG | __WALL);
    if (waited == child) {
      if (!WIFSTOPPED(*status) || WSTOPSIG(*status) != SIGSTOP ||
          ((*status >> 16) & 0xffff) != 0) {
        if (WIFEXITED(*status) || WIFSIGNALED(*status)) {
          d2b_emit_code("D2B-BZLEXEC-HELPER-GROUP-EARLY-EXIT");
        } else {
          d2b_emit_code("D2B-BZLEXEC-HELPER-PTRACE-STOP");
        }
        return (WIFEXITED(*status) || WIFSIGNALED(*status)) ? -3 : -1;
      }
      return 0;
    }
    if (waited < 0 && errno != EINTR) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-PTRACE-STOP");
      return -1;
    }

    struct pollfd descriptor = {.fd = signal_fd, .events = POLLIN};
    int timeout = d2b_remaining_ms(deadline);
    if (timeout > 10) {
      timeout = 10;
    }
    int poll_result = poll(&descriptor, 1, timeout);
    if (poll_result < 0 && errno == EINTR) {
      continue;
    }
    if (poll_result < 0 ||
        (poll_result > 0 &&
         (descriptor.revents & (POLLERR | POLLHUP | POLLNVAL)) != 0)) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-IO");
      return -1;
    }
  }
  d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-TIMEOUT");
  return -1;
}

static int d2b_read_signal(int signal_fd) {
  struct signalfd_siginfo info;
  ssize_t result = read(signal_fd, &info, sizeof(info));
  if (result == (ssize_t)sizeof(info)) {
    return (int)info.ssi_signo;
  }
  if (result < 0 && (errno == EAGAIN || errno == EWOULDBLOCK ||
                     errno == EINTR)) {
    return 0;
  }
  return -1;
}

static int d2b_pre_exec_loop(pid_t child, int error_fd, int signal_fd,
                             int64_t deadline, int *status) {
  unsigned char error_record[D2B_EXEC_ERROR_SIZE] = {0};
  size_t error_length = 0;
  int error_eof = 0;
  int probe_complete = 0;
  int child_reaped = 0;
  while (d2b_remaining_ms(deadline) > 0) {
    int signal_number = d2b_read_signal(signal_fd);
    if (signal_number < 0) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-IO");
      (void)d2b_group_kill_and_reap(child, child_reaped, signal_fd, status);
      return -1;
    }
    if (signal_number != 0) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-PRE-EXEC-TERMINATION");
      (void)d2b_group_kill_and_reap(child, child_reaped, signal_fd, status);
      return -1;
    }

    int read_result = d2b_read_exec_record(
        error_fd, error_record, &error_length, &error_eof, &probe_complete,
        deadline);
    if (read_result == D2B_EXEC_RECORD_COMPLETE) {
      d2b_emit_code(d2b_child_error_code(error_record[7]));
      (void)d2b_group_kill_and_reap(child, child_reaped, signal_fd, status);
      return -1;
    }
    if (read_result == D2B_EXEC_RECORD_OVERLONG) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-OVERLONG");
      (void)d2b_group_kill_and_reap(child, child_reaped, signal_fd, status);
      return -1;
    }
    if (read_result == D2B_EXEC_RECORD_UNKNOWN) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-UNKNOWN");
      (void)d2b_group_kill_and_reap(child, child_reaped, signal_fd, status);
      return -1;
    }
    if (read_result == D2B_EXEC_RECORD_PARTIAL) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-PARTIAL");
      (void)d2b_group_kill_and_reap(child, child_reaped, signal_fd, status);
      return -1;
    }
    if (read_result == D2B_EXEC_RECORD_IO) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-IO");
      (void)d2b_group_kill_and_reap(child, child_reaped, signal_fd, status);
      return -1;
    }
    if (read_result == D2B_EXEC_RECORD_TIMEOUT) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-TIMEOUT");
      (void)d2b_group_kill_and_reap(child, child_reaped, signal_fd, status);
      return -1;
    }

    if (!child_reaped) {
      pid_t waited = waitpid(child, status, WNOHANG | __WALL);
      if (waited == child) {
        if (WIFSTOPPED(*status) && WSTOPSIG(*status) == SIGTRAP &&
            ((*status >> 16) & 0xffff) == PTRACE_EVENT_EXEC) {
          return 0;
        }
        if (WIFEXITED(*status) || WIFSIGNALED(*status)) {
          child_reaped = 1;
        } else {
          d2b_emit_code("D2B-BZLEXEC-HELPER-PTRACE-EVENT");
          (void)d2b_group_kill_and_reap(child, 0, signal_fd, status);
          return -1;
        }
      }
      if (waited < 0) {
        if (errno == EINTR) {
          continue;
        }
        d2b_emit_code("D2B-BZLEXEC-HELPER-WAIT");
        (void)d2b_group_kill_and_reap(child, child_reaped, signal_fd, status);
        return -1;
      }
    }

    if (child_reaped && read_result == D2B_EXEC_RECORD_EMPTY_EOF) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-PRE-EXEC-DEATH");
      (void)d2b_group_kill_and_reap(child, child_reaped, signal_fd, status);
      return -1;
    }

    struct pollfd descriptors[2] = {
        {.fd = signal_fd, .events = POLLIN},
        {.fd = error_fd, .events = POLLIN | POLLHUP},
    };
    int poll_result = poll(descriptors, 2, d2b_remaining_ms(deadline));
    if (poll_result < 0) {
      if (errno == EINTR) {
        continue;
      }
      d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-IO");
      (void)d2b_group_kill_and_reap(child, child_reaped, signal_fd, status);
      return -1;
    }
    if (poll_result > 0 &&
        (descriptors[0].revents & (POLLERR | POLLHUP | POLLNVAL)) != 0) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-IO");
      (void)d2b_group_kill_and_reap(child, child_reaped, signal_fd, status);
      return -1;
    }
    if (poll_result > 0 &&
        (descriptors[1].revents & (POLLERR | POLLNVAL)) != 0) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-IO");
      (void)d2b_group_kill_and_reap(child, child_reaped, signal_fd, status);
      return -1;
    }
  }
  d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-TIMEOUT");
  (void)d2b_group_kill_and_reap(child, child_reaped, signal_fd, status);
  return -1;
}

static int d2b_forward_and_escalate(pid_t child, int signal_number,
                                    int signal_fd, int *status) {
  if (signal_number != SIGHUP && signal_number != SIGINT &&
      signal_number != SIGTERM && signal_number != SIGQUIT) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-SIGNAL-FORWARD");
    (void)d2b_group_kill_and_reap(child, 0, signal_fd, status);
    return -1;
  }
  if (kill(-child, signal_number) != 0 && errno != ESRCH) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-SIGNAL-FORWARD");
    (void)d2b_group_kill_and_reap(child, 0, signal_fd, status);
    return -1;
  }
  int64_t grace_start = d2b_monotonic_ms();
  if (grace_start < 0) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-DEADLINE");
    (void)d2b_group_kill_and_reap(child, 0, signal_fd, status);
    return -1;
  }
  int64_t grace_deadline = grace_start + D2B_TERM_GRACE_MS;
  while (d2b_remaining_ms(grace_deadline) > 0) {
    int observed = d2b_observe_child_exit(child);
    if (observed < 0 && errno != EINTR) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-WAIT");
      (void)d2b_group_kill_and_reap(child, 0, signal_fd, status);
      return -1;
    }
    /*
     * waitid(WNOWAIT) is informational only.  Even a repeatedly observed
     * zombie does not shorten the independently timed grace or consume the
     * direct-child status.
     */
    struct pollfd descriptor = {.fd = signal_fd, .events = POLLIN};
    int timeout = d2b_remaining_ms(grace_deadline);
    if (timeout > 10) {
      timeout = 10;
    }
    int poll_result = poll(&descriptor, 1, timeout);
    if (poll_result < 0) {
      if (errno == EINTR) {
        continue;
      }
      d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-IO");
      (void)d2b_group_kill_and_reap(child, 0, signal_fd, status);
      return -1;
    }
    if (poll_result > 0 &&
        (descriptor.revents & (POLLERR | POLLHUP | POLLNVAL)) != 0) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-IO");
      (void)d2b_group_kill_and_reap(child, 0, signal_fd, status);
      return -1;
    }
    if (poll_result > 0 && (descriptor.revents & POLLIN) != 0 &&
        d2b_read_signal(signal_fd) < 0) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-IO");
      (void)d2b_group_kill_and_reap(child, 0, signal_fd, status);
      return -1;
    }
  }
  /*
   * This kill is deliberately unconditional.  The group may already contain
   * only a zombie leader, in which case kill reports ESRCH, but the attempt
   * still occurs after the complete grace.
   */
  if (kill(-child, SIGKILL) != 0 && errno != ESRCH) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-DEADLINE");
    (void)d2b_group_kill_and_reap(child, 0, signal_fd, status);
    return -1;
  }
  return d2b_reap_after_kill(child, signal_fd, status);
}

static int d2b_supervise(pid_t child, int signal_fd, int *status) {
  while (1) {
    int signal_number = d2b_read_signal(signal_fd);
    if (signal_number < 0) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-IO");
      return -1;
    }
    if (signal_number != 0) {
      return d2b_forward_and_escalate(child, signal_number, signal_fd, status);
    }
    pid_t waited = waitpid(child, status, WNOHANG);
    if (waited == child) {
      return 0;
    }
    if (waited < 0 && errno != EINTR) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-WAIT");
      return -1;
    }
    struct pollfd descriptor = {.fd = signal_fd, .events = POLLIN};
    int poll_result = poll(&descriptor, 1, 100);
    if (poll_result < 0) {
      if (errno == EINTR) {
        continue;
      }
      d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-IO");
      return -1;
    }
    if (poll_result > 0 &&
        (descriptor.revents & (POLLERR | POLLHUP | POLLNVAL)) != 0) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-IO");
      return -1;
    }
  }
}

int main(int argc, char **argv, char **envp) {
  (void)envp;
  (void)d2b_recovery_codes;
  sigset_t managed;

  if (d2b_check_fd(D2B_PRIVATE_EXECUTABLE_FD, 0) != 0 ||
      d2b_check_fd(D2B_STATUS_FD, 0) != 0 ||
      d2b_check_fd(D2B_HELPER_ERROR_FD, 0) != 0) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-ADOPT");
    return 1;
  }
  int signal_state = d2b_inspect_inherited_signals(&managed);
  if (signal_state != 0) {
    return 1;
  }

  int signal_fd = -1;
  if (d2b_normalize_signals(&managed, &signal_fd) != 0) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-SIGNAL-NORMALIZE");
    return 1;
  }

  int signal_number = d2b_drain_signals(signal_fd);
  if (signal_number < 0) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-IO");
    close(signal_fd);
    return 1;
  }
  if (signal_number != 0) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-PRE-EXEC-TERMINATION");
    close(signal_fd);
    return 1;
  }

  int exec_pipe[2];
  if (pipe2(exec_pipe, O_CLOEXEC | O_NONBLOCK) != 0) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-PIPE");
    close(signal_fd);
    return 1;
  }

  int64_t deadline = d2b_monotonic_ms();
  if (deadline < 0) {
    close(exec_pipe[0]);
    close(exec_pipe[1]);
    d2b_emit_code("D2B-BZLEXEC-HELPER-DEADLINE");
    close(signal_fd);
    return 1;
  }
  deadline += D2B_EXEC_DEADLINE_MS;

  /*
   * Exactly one fork is owned by this helper.  The child and supervisor each
   * call setpgid; the kernel ptrace stop, not a confirmation pipe, releases it.
   */
  pid_t child = fork();
  if (child < 0) {
    close(exec_pipe[0]);
    close(exec_pipe[1]);
    d2b_emit_code("D2B-BZLEXEC-HELPER-FORK");
    close(signal_fd);
    return 1;
  }
  if (child == 0) {
    close(exec_pipe[0]);
    d2b_child_exec(exec_pipe[1],
                   argc > 1 ? &argv[1] : (char *const[]){"target", NULL});
  }
  close(exec_pipe[1]);

  int status = 0;
  signal_number = d2b_drain_signals(signal_fd);
  if (signal_number < 0) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-IO");
    (void)d2b_direct_kill_and_reap(child, signal_fd, &status);
    close(exec_pipe[0]);
    close(signal_fd);
    return 1;
  }
  if (signal_number != 0) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-PRE-EXEC-TERMINATION");
    (void)d2b_direct_kill_and_reap(child, signal_fd, &status);
    close(exec_pipe[0]);
    close(signal_fd);
    return 1;
  }
  if (d2b_confirm_group(child) != 0) {
    (void)d2b_direct_kill_and_reap(child, signal_fd, &status);
    close(exec_pipe[0]);
    close(signal_fd);
    return 1;
  }

  signal_number = d2b_drain_signals(signal_fd);
  if (signal_number < 0) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-IO");
    (void)d2b_group_kill_and_reap(child, 0, signal_fd, &status);
    close(exec_pipe[0]);
    close(signal_fd);
    return 1;
  }
  if (signal_number != 0) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-PRE-EXEC-TERMINATION");
    (void)d2b_group_kill_and_reap(child, 0, signal_fd, &status);
    close(exec_pipe[0]);
    close(signal_fd);
    return 1;
  }

  int initial_stop_result =
      d2b_wait_initial_stop(child, signal_fd, deadline, &status);
  if (initial_stop_result != 0) {
    (void)d2b_group_kill_and_reap(child, initial_stop_result == -3, signal_fd,
                                  &status);
    close(exec_pipe[0]);
    close(signal_fd);
    return 1;
  }

  if (ptrace(PTRACE_SETOPTIONS, child, (void *)0, (void *)(uintptr_t)PTRACE_O_TRACEEXEC) != 0) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-PTRACE-OPTIONS");
    (void)d2b_group_kill_and_reap(child, 0, signal_fd, &status);
    close(exec_pipe[0]);
    close(signal_fd);
    return 1;
  }
  signal_number = d2b_drain_signals(signal_fd);
  if (signal_number < 0) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-IO");
    (void)d2b_group_kill_and_reap(child, 0, signal_fd, &status);
    close(exec_pipe[0]);
    close(signal_fd);
    return 1;
  }
  if (signal_number != 0) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-PRE-EXEC-TERMINATION");
    (void)d2b_group_kill_and_reap(child, 0, signal_fd, &status);
    close(exec_pipe[0]);
    close(signal_fd);
    return 1;
  }
  /* This final zero-time drain is the READY publication boundary. */
  signal_number = d2b_drain_signals(signal_fd);
  if (signal_number < 0) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-EXEC-IO");
    (void)d2b_group_kill_and_reap(child, 0, signal_fd, &status);
    close(exec_pipe[0]);
    close(signal_fd);
    return 1;
  }
  if (signal_number != 0) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-PRE-EXEC-TERMINATION");
    (void)d2b_group_kill_and_reap(child, 0, signal_fd, &status);
    close(exec_pipe[0]);
    close(signal_fd);
    return 1;
  }
  int write_result = d2b_write_frame(D2B_STATUS_FD, D2B_READY, 0, deadline);
  if (write_result != D2B_WRITE_OK) {
    d2b_emit_status_write_failure(write_result,
                                  "D2B-BZLEXEC-HELPER-TERMINAL-WRITE");
    (void)d2b_group_kill_and_reap(child, 0, signal_fd, &status);
    close(exec_pipe[0]);
    close(signal_fd);
    return 1;
  }
  if (ptrace(PTRACE_CONT, child, (void *)0, (void *)0) != 0) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-PTRACE-CONT");
    (void)d2b_group_kill_and_reap(child, 0, signal_fd, &status);
    close(exec_pipe[0]);
    close(signal_fd);
    return 1;
  }

  if (d2b_pre_exec_loop(child, exec_pipe[0], signal_fd, deadline, &status) != 0) {
    close(exec_pipe[0]);
    close(signal_fd);
    return 1;
  }
  close(exec_pipe[0]);

  if (ptrace(PTRACE_DETACH, child, (void *)0, (void *)0) != 0) {
    d2b_emit_code("D2B-BZLEXEC-HELPER-PTRACE-DETACH");
    (void)d2b_group_kill_and_reap(child, 0, signal_fd, &status);
    close(signal_fd);
    return 1;
  }
  write_result = d2b_write_frame(D2B_STATUS_FD, D2B_EXECUTED, 0,
                                 d2b_monotonic_ms() + D2B_EXEC_DEADLINE_MS);
  if (write_result != D2B_WRITE_OK) {
    d2b_emit_status_write_failure(write_result,
                                  "D2B-BZLEXEC-HELPER-TERMINAL-WRITE");
    (void)d2b_group_kill_and_reap(child, 0, signal_fd, &status);
    close(signal_fd);
    return 1;
  }

  if (d2b_supervise(child, signal_fd, &status) != 0) {
    int child_reaped = WIFEXITED(status) || WIFSIGNALED(status);
    (void)d2b_group_kill_and_reap(child, child_reaped, signal_fd, &status);
    close(signal_fd);
    return 1;
  }
  if (d2b_group_kill_and_reap(child, 1, signal_fd, &status) != 0) {
    if (!d2b_helper_error_published) {
      d2b_emit_code("D2B-BZLEXEC-HELPER-CLEANUP");
    }
    close(signal_fd);
    return 1;
  }
  unsigned char terminal_value = 0;
  enum d2b_status_type terminal_type;
  if (WIFEXITED(status)) {
    terminal_type = D2B_EXITED;
    terminal_value = (unsigned char)WEXITSTATUS(status);
  } else if (WIFSIGNALED(status)) {
    terminal_type = D2B_SIGNALED;
    terminal_value = (unsigned char)WTERMSIG(status);
  } else {
    d2b_emit_code("D2B-BZLEXEC-HELPER-STATUS-MIRROR");
    (void)d2b_group_kill_and_reap(child, 1, signal_fd, &status);
    close(signal_fd);
    return 1;
  }
  write_result = d2b_write_frame(
      D2B_STATUS_FD, terminal_type, terminal_value,
      d2b_monotonic_ms() + D2B_EXEC_DEADLINE_MS);
  if (write_result != D2B_WRITE_OK) {
    d2b_emit_status_write_failure(write_result,
                                  "D2B-BZLEXEC-HELPER-TERMINAL-WRITE");
    (void)d2b_group_kill_and_reap(child, 1, signal_fd, &status);
    close(signal_fd);
    return 1;
  }
  close(signal_fd);
  close(D2B_STATUS_FD);
  close(D2B_PRIVATE_EXECUTABLE_FD);
  return WIFEXITED(status) ? WEXITSTATUS(status) : 128 + WTERMSIG(status);
}
