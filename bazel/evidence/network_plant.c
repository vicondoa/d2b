#define _GNU_SOURCE

#include <errno.h>
#include <linux/if_packet.h>
#include <linux/netlink.h>
#include <netinet/in.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/syscall.h>
#include <unistd.h>

static int denied_socket(int domain, int type, int protocol) {
  errno = 0;
  int fd = socket(domain, type, protocol);
  if (fd >= 0) {
    close(fd);
    return 1;
  }
  return errno == EACCES ? 0 : 2;
}

static int denied_socketpair(void) {
  int sockets[2];
  errno = 0;
  if (socketpair(AF_UNIX, SOCK_STREAM, 0, sockets) == 0) {
    close(sockets[0]);
    close(sockets[1]);
    return 1;
  }
  return errno == EACCES ? 0 : 2;
}

static int denied_invalid_syscalls(void) {
  struct sockaddr_storage address;
  memset(&address, 0, sizeof(address));
#define CHECK_DENIED(label, expression)                                      \
  do {                                                                       \
    errno = 0;                                                               \
    if ((expression) != -1 || errno != EACCES) {                             \
      fprintf(stderr, "%s:%d:%d\n", label, errno, EACCES);                 \
      return 1;                                                              \
    }                                                                        \
  } while (0)
  CHECK_DENIED(
      "connect",
      syscall(SYS_connect, -1, (struct sockaddr *)&address, sizeof(address)));
  CHECK_DENIED("bind", bind(-1, (struct sockaddr *)&address, sizeof(address)));
  CHECK_DENIED("listen", listen(-1, 1));
  CHECK_DENIED("accept", accept(-1, NULL, NULL));
  CHECK_DENIED("accept4", accept4(-1, NULL, NULL, 0));
  CHECK_DENIED("sendto", sendto(-1, NULL, 0, 0, NULL, 0));
  CHECK_DENIED("sendmsg", sendmsg(-1, NULL, 0));
  CHECK_DENIED("recvfrom", recvfrom(-1, NULL, 0, 0, NULL, NULL));
  CHECK_DENIED("recvmsg", recvmsg(-1, NULL, 0));
  CHECK_DENIED("shutdown", shutdown(-1, SHUT_RDWR));
  CHECK_DENIED("getsockname", getsockname(-1, NULL, NULL));
  CHECK_DENIED("getpeername", getpeername(-1, NULL, NULL));
  CHECK_DENIED("setsockopt", setsockopt(-1, 0, 0, NULL, 0));
  CHECK_DENIED("getsockopt", getsockopt(-1, 0, 0, NULL, NULL));
#ifdef SYS_pidfd_getfd
  CHECK_DENIED("pidfd_getfd", syscall(SYS_pidfd_getfd, -1, -1, 0));
#endif
#ifdef SYS_io_uring_setup
  CHECK_DENIED("io_uring_setup", syscall(SYS_io_uring_setup, 1, NULL));
#endif
#ifdef SYS_io_uring_enter
  CHECK_DENIED("io_uring_enter",
               syscall(SYS_io_uring_enter, -1, 0, 0, 0, NULL, 0));
#endif
#ifdef SYS_io_uring_register
  CHECK_DENIED("io_uring_register",
               syscall(SYS_io_uring_register, -1, 0, NULL, 0));
#endif
#undef CHECK_DENIED
  return 0;
}

int main(void) {
  int result = 0;
  int status = denied_socket(AF_INET, SOCK_STREAM, 0);
  if (status != 0) fprintf(stderr, "ipv4:%d:%d\n", status, errno);
  result |= status;
  status = denied_socket(AF_INET6, SOCK_STREAM, 0);
  if (status != 0) fprintf(stderr, "ipv6:%d:%d\n", status, errno);
  result |= status;
  status = denied_socket(AF_NETLINK, SOCK_RAW, NETLINK_ROUTE);
  if (status != 0) fprintf(stderr, "netlink:%d:%d\n", status, errno);
  result |= status;
  status = denied_socket(AF_PACKET, SOCK_RAW, 0);
  if (status == 2 && errno == EPERM) {
    status = 0;
  }
  if (status != 0) fprintf(stderr, "packet:%d:%d\n", status, errno);
  result |= status;
  status = denied_socket(AF_UNIX, SOCK_STREAM, 0);
  if (status != 0) fprintf(stderr, "unix:%d:%d\n", status, errno);
  result |= status;
  status = denied_socketpair();
  if (status != 0) fprintf(stderr, "socketpair:%d:%d\n", status, errno);
  result |= status;
  status = denied_invalid_syscalls();
  if (status != 0) fprintf(stderr, "invalid:%d:%d\n", status, errno);
  result |= status;
  if (result != 0) {
    fprintf(stderr, "sandbox network denial plant failed: %d\n", result);
    return 1;
  }
  puts("sandbox-network-denied");
  return 0;
}
