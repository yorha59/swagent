"""Execute the production outbound connector with a permanently pending socket."""
import pathlib
import subprocess
import tempfile
import unittest
from test_shutdown import function

class ConnectTest(unittest.TestCase):
    def test_pending_connect_is_bounded_and_closed(self):
        prelude = r'''
#include <assert.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include <netdb.h>
#include <arpa/inet.h>
#include <poll.h>
#include <unistd.h>
typedef uint64_t u64;
static u64 clock_ms;
static int closes, timeouts, polls, mode;
static u64 sa_now_ms(void) { return clock_ms; }
static int sa_stopping(void) { return 0; }
static void sa_persist(const char *s,size_t n) { (void)n;if(strstr(s,"connect timeout attempt="))timeouts++; }
static int track_socket(int fd) { return fd; }
static int sa_close(int fd) { closes++;return close(fd); }
static void sa_timeout(int fd,int seconds) { (void)fd;assert(seconds==2); }
static int pending_connect(int fd,const struct sockaddr *a,socklen_t n) {
    (void)a;(void)n;assert(fcntl(fd,F_GETFL)&O_NONBLOCK);errno=EINPROGRESS;return -1;
}
static int fake_poll(struct pollfd *p,nfds_t n,int timeout) {
    (void)p;assert(n==1);assert(timeout>0 && timeout<=100);clock_ms+=timeout;polls++;
    return mode ? 1 : 0;
}
static int fake_getsockopt(int fd,int level,int opt,void *value,socklen_t *n) {
    (void)fd;(void)level;(void)opt;(void)n;*(int*)value=mode==1 ? 0 : ECONNREFUSED;return 0;
}
#define connect pending_connect
#define poll fake_poll
#define getsockopt fake_getsockopt
'''
        trailer = r'''
int main(void) {
    assert(sa_connect("127.0.0.1","80")==-1);assert(errno==ETIMEDOUT);
    assert(clock_ms==45000 && closes==3 && timeouts==3 && polls==450);
    mode=1;int fd=sa_connect("127.0.0.1","80");assert(fd>=0);sa_close(fd);
    mode=2;assert(sa_connect("127.0.0.1","80")==-1);assert(errno==ECONNREFUSED);
    assert(closes==7);return 0;
}
'''
        with tempfile.TemporaryDirectory() as tmp:
            source=pathlib.Path(tmp)/'connect.c'
            binary=pathlib.Path(tmp)/'connect'
            source.write_text(prelude+function('int sa_connect(const char *host, const char *port)')+trailer)
            subprocess.run(['cc','-Wall','-Wextra','-Werror',str(source),'-o',str(binary)],check=True)
            subprocess.run([str(binary)],check=True,timeout=5)

if __name__ == '__main__':
    unittest.main()
