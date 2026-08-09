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

static int open_inherited_ring(unsigned int flags, int registered_socket) {
#ifdef SYS_io_uring_setup
  struct io_uring_params params;
  memset(&params, 0, sizeof(params));
  params.flags = flags;
  if ((flags & IORING_SETUP_SQPOLL) != 0) {
    params.sq_thread_idle = 1000;
  }
  int fd = (int)syscall(SYS_io_uring_setup, 1, &params);
  if (fd < 0 || keep_open(fd) != 0) {
    return -1;
  }
  if (registered_socket) {
    int socket_fd = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (socket_fd < 0 ||
        syscall(SYS_io_uring_register, fd, IORING_REGISTER_FILES, &socket_fd,
                1) < 0) {
      if (socket_fd >= 0) {
        close(socket_fd);
      }
      close(fd);
      return -1;
    }
    close(socket_fd);
  }
  return fd;
#else
  (void)flags;
  (void)registered_socket;
  return -1;
#endif
}

int main(int argc, char **argv) {
  if (argc < 3) {
    fprintf(stderr,
            "usage: %s socket|ring|ring-sqpoll|ring-registered-socket "
            "SANDBOX [ARGS...]\n",
            argv[0]);
    return 64;
  }
  int fd;
  if (strcmp(argv[1], "socket") == 0) {
    fd = open_inherited_socket();
  } else if (strcmp(argv[1], "ring") == 0) {
    fd = open_inherited_ring(0, 0);
  } else if (strcmp(argv[1], "ring-sqpoll") == 0) {
    fd = open_inherited_ring(IORING_SETUP_SQPOLL, 0);
  } else if (strcmp(argv[1], "ring-registered-socket") == 0) {
    fd = open_inherited_ring(0, 1);
  } else {
    return 64;
  }
  if (fd < 0) {
    return 70;
  }
  (void)fd;
  execv(argv[2], &argv[2]);
  return 71;
}
