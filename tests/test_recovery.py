"""Run production log writer and listener probe with deterministic host faults."""
import pathlib
import subprocess
import tempfile
import unittest
from test_shutdown import function

class RecoveryTest(unittest.TestCase):
    def run_c(self, code):
        with tempfile.TemporaryDirectory() as tmp:
            source = pathlib.Path(tmp) / 'recovery.c'
            binary = pathlib.Path(tmp) / 'recovery'
            source.write_text(code)
            subprocess.run(['cc', '-Wall', '-Wextra', '-Werror', str(source), '-o', str(binary)], check=True)
            subprocess.run([str(binary)], check=True, timeout=5)

    def test_writer_reopen_rotation_and_permanent_failure(self):
        self.run_c(r'''
#include <assert.h>
#include <stdbool.h>
#include <stdatomic.h>
#include <stdio.h>
#include <string.h>
#include <sys/stat.h>
static atomic_flag log_lock=ATOMIC_FLAG_INIT;
static atomic_ullong log_beat;
static unsigned long long sa_now_ms(void){return 1;}
static char log_queue[128][768],log_tail[16384];
static size_t log_head,log_count,log_tail_len;
static int opens,closes,writes,flushes,fail_until,rotates;
static long size;
static FILE *fake_open(const char *p,const char *m) {
    (void)p;
    if(!strcmp(m,"rb"))return NULL;
    opens++;
    if(!strcmp(m,"wb")){rotates++;size=0;}
    return (FILE*)1;
}
static int fake_close(FILE *f){assert(f);closes++;return 0;}
static int fake_seek(FILE *f,long off,int whence){(void)f;(void)off;(void)whence;return 0;}
static long fake_tell(FILE *f){(void)f;return size;}
static size_t fake_read(void *p,size_t s,size_t n,FILE *f){(void)p;(void)s;(void)n;(void)f;return 0;}
static size_t fake_write(const void *p,size_t s,size_t n,FILE *f){
    (void)p;(void)s;(void)f;writes++;if(writes<=fail_until)return 0;size+=n;return n;
}
static int fake_flush(FILE *f){(void)f;flushes++;return 0;}
static int sa_stopping(void){return log_count==0;}
static void svcSleepThread(long n){(void)n;}
#define fopen fake_open
#define fclose fake_close
#define fseek fake_seek
#define ftell fake_tell
#define fread fake_read
#define fwrite fake_write
#define fflush fake_flush
''' + function('static void log_cache(const char *p,size_t n)') + '\n' + function('static void log_writer(void *arg)') + r'''
static void reset(int count){
    log_head=log_tail_len=0;log_count=count;opens=closes=writes=flushes=rotates=0;size=0;
    for(int i=0;i<count;i++)strcpy(log_queue[i],"hello\n");
}
int main(void){
    reset(12);fail_until=10;log_writer(NULL);
    assert(opens==2 && closes==2 && writes==13 && log_tail_len==18);
    reset(30);fail_until=100;log_writer(NULL);
    assert(opens==2 && closes==2 && writes==31 && log_tail_len==0);
    reset(1);fail_until=0;size=1024*1024-2;log_writer(NULL);
    assert(rotates==1 && size==6 && log_tail_len==6 && closes==2);
    return 0;
}
''')

    def test_listener_probe_closes_only_owned_unhealthy_fd(self):
        self.run_c(r'''
#include <assert.h>
#include <stdbool.h>
#include <sys/socket.h>
#include <netinet/in.h>
static int listener_lock,listener_fd=7,closed,stopped,broken;
static void sa_trace(const char *s,int rc){(void)s;(void)rc;}
static void mutexLock(int *m){(void)m;}
static void mutexUnlock(int *m){(void)m;}
static int fake_close(int fd){assert(fd==7);closed++;return 0;}
static int fake_shutdown(int fd,int how){assert(fd==7 && how==SHUT_RDWR);stopped++;return 0;}
static int fake_name(int fd,struct sockaddr *a,socklen_t *n){(void)fd;(void)a;(void)n;return broken?-1:0;}
static int fake_opt(int fd,int level,int opt,void *p,socklen_t *n){
    (void)fd;(void)level;(void)n;*(int*)p=opt==SO_ACCEPTCONN;return 0;
}
#define close fake_close
#define shutdown fake_shutdown
#define getsockname fake_name
#define getsockopt fake_opt
''' + function('int sa_listener_health(int fd, int stale)') + r'''
int main(void){
    assert(sa_listener_health(7,0)==0 && !closed);
    assert(sa_listener_health(8,0)==1 && !closed);
    broken=1;assert(sa_listener_health(7,0)==1 && closed==1 && stopped==1 && listener_fd==-1);
    assert(sa_listener_health(7,0)==1 && closed==1);
    listener_fd=7;broken=0;assert(sa_listener_health(7,1)==1 && closed==2);
    return 0;
}
''')

    def test_socket_registry_releases_every_slot(self):
        self.run_c(r'''
#include <assert.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
static int listener_lock,active_sockets[32];
static void mutexLock(int *m){(void)m;}
static void mutexUnlock(int *m){(void)m;}
static int sa_stopping(void){return 0;}
''' + function('static int track_socket(int fd)') + '\n' + function('int sa_close(int fd)') + r'''
int main(void){
    for(int i=0;i<1024;i++){
        int fd=track_socket(open("/dev/null",O_RDONLY));assert(fd>=0);
        assert(sa_close(fd)==0);assert(fcntl(fd,F_GETFD)==-1 && errno==EBADF);
    }
    int fds[32];
    for(int i=0;i<32;i++){fds[i]=track_socket(open("/dev/null",O_RDONLY));assert(fds[i]>=0);}
    int extra=open("/dev/null",O_RDONLY);assert(extra>=0);
    assert(track_socket(extra)==-1 && errno==ECANCELED);
    assert(fcntl(extra,F_GETFD)==-1 && errno==EBADF);
    for(int i=0;i<32;i++)assert(sa_close(fds[i])==0);
    for(int i=0;i<32;i++)assert(active_sockets[i]==0);
    return 0;
}
''')

if __name__ == '__main__':
    unittest.main()
