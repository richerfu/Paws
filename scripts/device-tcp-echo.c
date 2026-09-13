/* Device-side VPN data-path acceptance. HDC/root sockets may bypass VPN
 * policy; drop to a caller-selected non-VPN application UID before opening
 * the socket. Verify payload, not merely a successful TCP handshake. */
#include <arpa/inet.h>
#include <errno.h>
#include <grp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>

static unsigned long number(const char *value, unsigned long limit) {
    char *end = NULL;
    errno = 0;
    unsigned long result = strtoul(value, &end, 10);
    if (errno || !*value || *value == '-' || *end || result > limit) {
        fprintf(stderr, "invalid numeric argument: %s\n", value);
        exit(2);
    }
    return result;
}

int main(int argc, char **argv) {
    if (argc != 5) {
        fprintf(stderr, "usage: %s UID IPV4 PORT PAYLOAD\n", argv[0]);
        return 2;
    }
    uid_t uid = (uid_t)number(argv[1], 0xffffffffUL);
    unsigned long port = number(argv[3], 65535);
    size_t length = strlen(argv[4]);
    if (!uid || !port || !length || length > 4096) return 2;
    struct sockaddr_in peer = { .sin_family = AF_INET, .sin_port = htons((unsigned short)port) };
    if (inet_pton(AF_INET, argv[2], &peer.sin_addr) != 1) return 2;
    if (setgroups(0, NULL) || setgid(uid) || setuid(uid)) {
        perror("drop probe UID");
        return 3;
    }
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) { perror("socket"); return 4; }
    struct timeval timeout = { .tv_sec = 8 };
    if (setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout)) ||
        setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout))) {
        perror("socket timeout"); close(fd); return 4;
    }
    if (connect(fd, (struct sockaddr *)&peer, sizeof(peer))) {
        perror("connect"); close(fd); return 5;
    }
    struct sockaddr_in local = {0};
    socklen_t local_length = sizeof(local);
    char address[INET_ADDRSTRLEN] = {0};
    if (!getsockname(fd, (struct sockaddr *)&local, &local_length))
        inet_ntop(AF_INET, &local.sin_addr, address, sizeof(address));
    size_t offset = 0;
    while (offset < length) {
        ssize_t count = send(fd, argv[4] + offset, length - offset, MSG_NOSIGNAL);
        if (count <= 0) { perror("send"); close(fd); return 6; }
        offset += (size_t)count;
    }
    char echoed[4096];
    offset = 0;
    while (offset < length) {
        ssize_t count = recv(fd, echoed + offset, length - offset, 0);
        if (count <= 0) { fprintf(stderr, "echo ended early: %s\n", count ? strerror(errno) : "EOF"); close(fd); return 7; }
        offset += (size_t)count;
    }
    close(fd);
    if (memcmp(argv[4], echoed, length)) { fprintf(stderr, "echo payload mismatch\n"); return 8; }
    printf("PASS uid=%lu local=%s peer=%s:%lu echoed=%zu bytes\n", (unsigned long)getuid(), address, argv[2], port, length);
    return 0;
}
