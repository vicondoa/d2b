/*
 * Deterministic test plant for the patched Bazel PID-namespace monitor.
 *
 * The plant has no network or host-cleanup behavior.  It only gives the
 * real linux-sandbox a controlled action process that exits at a named
 * supervisor protocol phase and, when requested, leaves descendants for the
 * namespace PID-1 monitor to adopt and kill.
 */

#define _GNU_SOURCE

#include <errno.h>
#include <dirent.h>
#include <fcntl.h>
#include <signal.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

enum plant_stage {
  PLANT_BEFORE_READY,
  PLANT_AFTER_READY,
  PLANT_AFTER_EXECUTED,
  PLANT_DURING_GRACE,
  PLANT_EXIT_DURING_GRACE,
  PLANT_FD_AUDIT,
  PLANT_DESCENDANT,
  PLANT_DOUBLE_FORK_DESCENDANT,
  PLANT_BEYOND_CEILING,
};

static enum plant_stage stage = PLANT_BEFORE_READY;
static bool hold_liveness_fd;
static bool crash_with_signal;
static int liveness_fd = -1;
static const char *liveness_path;
static const char *barrier_path;

static void usage(const char *name) {
  fprintf(stderr,
          "usage: %s --stage "
          "{before-ready|after-ready|after-executed|during-grace|"
          "exit-during-grace|fd-audit|direct-descendant|"
          "double-fork-descendant|beyond-ceiling} "
          "[--hold-liveness-fd --liveness-path PATH] "
          "[--barrier-path PATH] [--sigsegv]\n",
          name);
  _exit(64);
}

static enum plant_stage parse_stage(const char *value) {
  if (strcmp(value, "before-ready") == 0) return PLANT_BEFORE_READY;
  if (strcmp(value, "after-ready") == 0) return PLANT_AFTER_READY;
  if (strcmp(value, "after-executed") == 0) return PLANT_AFTER_EXECUTED;
  if (strcmp(value, "during-grace") == 0) return PLANT_DURING_GRACE;
  if (strcmp(value, "exit-during-grace") == 0)
    return PLANT_EXIT_DURING_GRACE;
  if (strcmp(value, "fd-audit") == 0) return PLANT_FD_AUDIT;
  if (strcmp(value, "direct-descendant") == 0) return PLANT_DESCENDANT;
  if (strcmp(value, "double-fork-descendant") == 0)
    return PLANT_DOUBLE_FORK_DESCENDANT;
  if (strcmp(value, "beyond-ceiling") == 0) return PLANT_BEYOND_CEILING;
  usage("d2b-bazel-sandbox-crash-plant");
  return PLANT_BEFORE_READY;
}

static void keep_descriptor_open(void) {
  if (liveness_fd >= 0) {
    return;
  }
  if (liveness_path == NULL || *liveness_path == '\0') {
    _exit(65);
  }
  /*
   * The harness creates this FIFO and opens its read end before launching the
   * plant. Every descendant inherits the same write end. The harness therefore
   * observes a byte for liveness and EOF only after the namespace has closed
   * every copy during cleanup; no private descriptor or host identifier is
   * rendered.
   */
  liveness_fd = open(liveness_path, O_WRONLY | O_NONBLOCK | O_CLOEXEC);
  if (liveness_fd < 0) {
    _exit(65);
  }
  const char marker = '1';
  if (write(liveness_fd, &marker, 1) != 1) {
    _exit(65);
  }
}

static void descendant_loop(void) {
  keep_descriptor_open();
  for (;;) {
    pause();
  }
}

static void start_descendants(bool double_fork) {
  pid_t first = fork();
  if (first < 0) _exit(66);
  if (first == 0) {
    (void)prctl(PR_SET_PDEATHSIG, SIGKILL);
    if (!double_fork) {
      descendant_loop();
    }
    pid_t second = fork();
    if (second < 0) _exit(67);
    if (second == 0) descendant_loop();
    _exit(0);
  }
}

static void barrier_until_release(void) {
  /*
   * The harness creates a FIFO and keeps its writer open without writing.
   * Reading it is a deterministic, externally observable barrier: the harness
   * can release it after checking quarantine, while SIGKILL closes the plant's
   * descriptor and lets the reader observe EOF. Without a barrier path the
   * plant remains live for ordinary crash cases.
   */
  if (barrier_path == NULL || *barrier_path == '\0') {
    pause();
    return;
  }
  char byte;
  int fd = open(barrier_path, O_RDONLY | O_CLOEXEC);
  if (fd < 0) {
    _exit(68);
  }
  while (read(fd, &byte, 1) < 0 && errno == EINTR) {
  }
  close(fd);
}

static void exit_on_term(int signal_number) {
  (void)signal_number;
  _exit(73);
}

static int has_inherited_signalfd(void) {
  DIR *directory = opendir("/proc/self/fd");
  if (directory == NULL) return -1;
  int found = 0;
  struct dirent *entry;
  while ((entry = readdir(directory)) != NULL) {
    if (entry->d_name[0] == '.') continue;
    char link_path[64];
    char target[128];
    int path_length = snprintf(link_path, sizeof(link_path), "/proc/self/fd/%s",
                               entry->d_name);
    if (path_length < 0 || (size_t)path_length >= sizeof(link_path)) {
      found = -1;
      break;
    }
    ssize_t target_length =
        readlink(link_path, target, sizeof(target) - 1);
    if (target_length < 0) continue;
    target[target_length] = '\0';
    if (strcmp(target, "anon_inode:[signalfd]") == 0) {
      found = 1;
      break;
    }
  }
  closedir(directory);
  return found;
}

int main(int argc, char **argv) {
  for (int i = 1; i < argc; ++i) {
    if (strcmp(argv[i], "--stage") == 0 && i + 1 < argc) {
      stage = parse_stage(argv[++i]);
    } else if (strcmp(argv[i], "--hold-liveness-fd") == 0) {
      hold_liveness_fd = true;
    } else if (strcmp(argv[i], "--liveness-path") == 0 && i + 1 < argc) {
      liveness_path = argv[++i];
      hold_liveness_fd = true;
    } else if (strcmp(argv[i], "--barrier-path") == 0 && i + 1 < argc) {
      barrier_path = argv[++i];
    } else if (strcmp(argv[i], "--sigsegv") == 0) {
      crash_with_signal = true;
    } else {
      usage(argv[0]);
    }
  }

  if (hold_liveness_fd) {
    keep_descriptor_open();
  }

  switch (stage) {
    case PLANT_BEFORE_READY:
      _exit(crash_with_signal ? 128 + SIGSEGV : 70);
    case PLANT_AFTER_READY:
      _exit(crash_with_signal ? 128 + SIGSEGV : 71);
    case PLANT_AFTER_EXECUTED:
      _exit(crash_with_signal ? 128 + SIGSEGV : 72);
    case PLANT_DURING_GRACE:
      signal(SIGTERM, SIG_IGN);
      barrier_until_release();
      _exit(73);
    case PLANT_EXIT_DURING_GRACE:
      if (signal(SIGTERM, exit_on_term) == SIG_ERR) _exit(68);
      barrier_until_release();
      _exit(73);
    case PLANT_FD_AUDIT:
      _exit(has_inherited_signalfd() == 0 ? 0 : 69);
    case PLANT_DESCENDANT:
      start_descendants(false);
      _exit(74);
    case PLANT_DOUBLE_FORK_DESCENDANT:
      start_descendants(true);
      _exit(75);
    case PLANT_BEYOND_CEILING:
      signal(SIGTERM, SIG_IGN);
      start_descendants(true);
      barrier_until_release();
      _exit(76);
  }
  return 77;
}
