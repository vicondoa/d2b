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
#include <fcntl.h>
#include <signal.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/prctl.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <unistd.h>

enum plant_stage {
  PLANT_BEFORE_READY,
  PLANT_AFTER_READY,
  PLANT_AFTER_EXECUTED,
  PLANT_DURING_GRACE,
  PLANT_DESCENDANT,
  PLANT_DOUBLE_FORK_DESCENDANT,
  PLANT_BEYOND_CEILING,
};

static enum plant_stage stage = PLANT_BEFORE_READY;
static bool hold_liveness_fd;
static bool crash_with_signal;
static int liveness_fd = -1;

static void usage(const char *name) {
  fprintf(stderr,
          "usage: %s --stage "
          "{before-ready|after-ready|after-executed|during-grace|"
          "direct-descendant|double-fork-descendant|beyond-ceiling} "
          "[--hold-liveness-fd] [--sigsegv]\n",
          name);
  _exit(64);
}

static enum plant_stage parse_stage(const char *value) {
  if (strcmp(value, "before-ready") == 0) return PLANT_BEFORE_READY;
  if (strcmp(value, "after-ready") == 0) return PLANT_AFTER_READY;
  if (strcmp(value, "after-executed") == 0) return PLANT_AFTER_EXECUTED;
  if (strcmp(value, "during-grace") == 0) return PLANT_DURING_GRACE;
  if (strcmp(value, "direct-descendant") == 0) return PLANT_DESCENDANT;
  if (strcmp(value, "double-fork-descendant") == 0)
    return PLANT_DOUBLE_FORK_DESCENDANT;
  if (strcmp(value, "beyond-ceiling") == 0) return PLANT_BEYOND_CEILING;
  usage("d2b-bazel-sandbox-crash-plant");
  return PLANT_BEFORE_READY;
}

static void keep_descriptor_open(void) {
  if (liveness_fd < 0) {
    liveness_fd = open("/dev/null", O_RDONLY | O_CLOEXEC);
  }
  if (liveness_fd < 0) {
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
   * A pipe supplied by the test harness is optional.  Without one the plant
   * remains live, which is useful for the ordinary crash/quarantine cases.
   * The monitor owns the eventual namespace cleanup; this process never
   * signals a host pid or process group.
   */
  const char *barrier_fd = getenv("D2B_SANDBOX_PLANT_BARRIER_FD");
  if (barrier_fd == NULL || *barrier_fd == '\0') {
    pause();
    return;
  }
  char byte;
  int fd = atoi(barrier_fd);
  while (read(fd, &byte, 1) < 0 && errno == EINTR) {
  }
}

int main(int argc, char **argv) {
  for (int i = 1; i < argc; ++i) {
    if (strcmp(argv[i], "--stage") == 0 && i + 1 < argc) {
      stage = parse_stage(argv[++i]);
    } else if (strcmp(argv[i], "--hold-liveness-fd") == 0) {
      hold_liveness_fd = true;
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
    case PLANT_DESCENDANT:
      start_descendants(false);
      _exit(74);
    case PLANT_DOUBLE_FORK_DESCENDANT:
      start_descendants(true);
      _exit(75);
    case PLANT_BEYOND_CEILING:
      start_descendants(true);
      barrier_until_release();
      _exit(76);
  }
  return 77;
}
