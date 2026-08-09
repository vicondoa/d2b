#define _GNU_SOURCE

#include <fcntl.h>
#include <linux/io_uring.h>
#include <netinet/in.h>
#include <sched.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/syscall.h>
#include <unistd.h>

static int keep_open(int fd) {
  int flags = fcntl(fd, F_GETFD);
  if (flags < 0) {
    return 1;
  }
  return fcntl(fd, F_SETFD, flags & ~FD_CLOEXEC) < 0;
}

static int open_inherited_socket(void) {
  int fd = socket(AF_INET, SOCK_STREAM, 0);
  if (fd < 0 || keep_open(fd) != 0) {
    return -1;
  }
  return fd;
}

static int open_inherited_ring(void) {
#ifdef SYS_io_uring_setup
  struct io_uring_params params;
  memset(&params, 0, sizeof(params));
  int fd = (int)syscall(SYS_io_uring_setup, 1, &params);
  if (fd < 0 || keep_open(fd) != 0) {
    return -1;
  }
  return fd;
#else
  return -1;
#endif
}

int main(int argc, char **argv) {
  if (argc < 3 || (strcmp(argv[1], "socket") != 0 &&
                   strcmp(argv[1], "ring") != 0)) {
    fprintf(stderr, "usage: %s socket|ring BAZEL [ARGS...]\n", argv[0]);
    return 64;
  }
  int fd = strcmp(argv[1], "socket") == 0 ? open_inherited_socket()
                                          : open_inherited_ring();
  if (fd < 0) {
    return 70;
  }
  (void)fd;
  execv(argv[2], &argv[2]);
  return 71;
}
