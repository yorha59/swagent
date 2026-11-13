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

    def test_writer_short_handles_rotation_and_recovery(self):
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
static int opens,closes,writes,flushes,fail_until,rotates,active;
static int open_fail,rotate_fail,seek_fail,tell_fail,flush_fail,close_fail;
static long size;
static FILE *fake_open(const char *p,const char *m) {
    (void)p;
    if(!strcmp(m,"rb"))return NULL;
    assert(!active); /* Model Switch exclusive open, including readers. */
    opens++;
    if(open_fail)return NULL;
    if(!strcmp(m,"wb")){if(rotate_fail)return NULL;rotates++;size=0;}
    else assert(!strcmp(m,"ab"));
    active=1;
    return (FILE*)1;
}
static int fake_close(FILE *f){assert(f && active);active=0;closes++;return close_fail?-1:0;}
static int fake_seek(FILE *f,long off,int whence){(void)f;(void)off;(void)whence;return seek_fail?-1:0;}
static long fake_tell(FILE *f){(void)f;return tell_fail?-1:size;}
static size_t fake_read(void *p,size_t s,size_t n,FILE *f){(void)p;(void)s;(void)n;(void)f;return 0;}
static size_t fake_write(const void *p,size_t s,size_t n,FILE *f){
    (void)p;(void)s;(void)f;writes++;if(writes<=fail_until)return 0;size+=n;return n;
}
static int fake_flush(FILE *f){(void)f;flushes++;return flush_fail?-1:0;}
static int sa_stopping(void){assert(!active);return log_count==0;}
static void svcSleepThread(long n){(void)n;assert(!active);}
#define fopen fake_open
#define fclose fake_close
#define fseek fake_seek
#define ftell fake_tell
#define fread fake_read
#define fwrite fake_write
#define fflush fake_flush
''' + function('static void log_cache(const char *p,size_t n)') + '\n' + function('static void log_writer(void *arg)') + r'''
static void reset(int count){
    assert(!active);
    open_fail=rotate_fail=seek_fail=tell_fail=flush_fail=close_fail=0;
    fail_until=0;
    log_head=log_tail_len=0;log_count=count;opens=closes=writes=flushes=rotates=0;size=0;
    for(int i=0;i<count;i++)strcpy(log_queue[i],"hello\n");
}
int main(void){
    reset(12);fail_until=10;log_writer(NULL);
    assert(opens==12 && closes==12 && writes==12 && log_tail_len==12);
    reset(30);fail_until=100;log_writer(NULL);
    assert(opens==30 && closes==30 && writes==30 && log_tail_len==0);
    reset(1);fail_until=0;size=1024*1024-2;log_writer(NULL);
    assert(rotates==1 && size==6 && log_tail_len==6 && closes==2);
    reset(1);size=1024*1024-6;log_writer(NULL);
    assert(!rotates && size==1024*1024 && closes==1);
    reset(2);size=1024*1024-6;log_writer(NULL);
    assert(rotates==1 && size==6 && log_tail_len==6 && closes==3);
    reset(1);size=1024*1024+10;rotate_fail=1;log_tail_len=7;log_writer(NULL);
    assert(!writes && closes==1 && log_tail_len==7);
    reset(1);open_fail=1;log_writer(NULL);
    assert(!writes && !closes && !log_tail_len);
    reset(1);seek_fail=1;log_writer(NULL);
    assert(!writes && closes==1 && !log_tail_len);
    reset(1);tell_fail=1;log_writer(NULL);
    assert(!writes && closes==1 && !log_tail_len);
    reset(1);flush_fail=1;log_writer(NULL);
    assert(writes==1 && closes==1 && !log_tail_len);
    reset(1);close_fail=1;log_writer(NULL);
    assert(writes==1 && closes==1 && !log_tail_len);
    reset(1);log_writer(NULL);
    assert(writes==1 && closes==1 && log_tail_len==6);
    return 0;
}
''')

    def test_listener_probe_never_mutates_descriptor(self):
        probe = function('int sa_listener_health(int fd, int stale)')
        for forbidden in ('close(', 'shutdown(', 'listen(', 'mutexLock('):
            self.assertNotIn(forbidden, probe)
        self.run_c(r'''
#include <assert.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <errno.h>
static int broken;
static int fake_name(int fd,struct sockaddr *a,socklen_t *n){(void)fd;(void)a;(void)n;if(broken){errno=EBADF;return -1;}return 0;}
static int fake_opt(int fd,int level,int opt,void *p,socklen_t *n){
    (void)fd;(void)level;(void)n;*(int*)p=opt==SO_ACCEPTCONN;return 0;
}
#define getsockname fake_name
#define getsockopt fake_opt
''' + probe + r'''
int main(void){
    assert(sa_listener_health(7,0)==0);
    broken=1;assert(sa_listener_health(7,0)==-EBADF);
    broken=0;assert(sa_listener_health(7,1)==-ETIMEDOUT);
    return 0;
}
''')

    def test_listener_structured_error_context(self):
        self.run_c(r'''
#include <assert.h>
#include <stdio.h>
#include <string.h>
static unsigned long long listener_old_generation=4,listener_generation=5,listener_requested=100;
static int listener_attempt_fd=17;
static char captured[768];
static unsigned long long sa_now_ms(void){return 200;}
static void sa_persist(const char *p,size_t n){memcpy(captured,p,n);captured[n]=0;}
''' + function('static void sa_trace(const char *step,int rc)') + r'''
int main(void){
    sa_trace("listener-nonblocking",-9);
    assert(strstr(captured,"stage=listener-nonblocking"));
    assert(strstr(captured,"old_generation=4 new_generation=5 fd=17"));
    assert(strstr(captured,"rc=-9 errno=9"));
    return 0;
}
''')

    def test_owner_socket_bind_listen_faults(self):
        self.run_c(r'''
#include <assert.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <errno.h>
static int fault,closed,listener_attempt_fd,last_rc;
static const char *last_step;
static void sa_trace(const char *s,int rc){last_step=s;last_rc=rc;}
static int fake_socket(int a,int b,int c){(void)a;(void)b;(void)c;if(fault==1){errno=24;return -1;}return 7;}
static int fake_opt(int fd,int a,int b,const void *p,socklen_t n){(void)fd;(void)a;(void)b;(void)p;(void)n;return 0;}
static int fake_bind(int fd,const struct sockaddr *a,socklen_t n){assert(fd==7);(void)a;(void)n;if(fault==2){errno=48;return -1;}return 0;}
static int fake_listen(int fd,int n){assert(fd==7);(void)n;if(fault==3){errno=22;return -1;}return 0;}
static int fake_fcntl(int fd,int op,int flags){assert(fd==7 && op==F_SETFL && flags==O_NONBLOCK);if(fault==4){errno=9;return -1;}return 0;}
static int fake_close(int fd){assert(fd==7);if(fault==4)assert(!strcmp(last_step,"listener-nonblocking") && last_rc==-9);closed++;return 0;}
#define socket fake_socket
#define setsockopt fake_opt
#define bind fake_bind
#define listen fake_listen
#define fcntl fake_fcntl
#define close fake_close
''' + function('static int listen_socket(void)') + r'''
int main(void){
    fault=1;assert(listen_socket()==-24 && !closed && last_rc==-24);
    fault=2;assert(listen_socket()==-48 && closed==1 && listener_attempt_fd==7);
    assert(!strcmp(last_step,"listener-bind") && last_rc==-48);
    fault=3;assert(listen_socket()==-22 && closed==2);
    assert(!strcmp(last_step,"listener-listen") && last_rc==-22);
    fault=4;assert(listen_socket()==-9 && closed==3);
    assert(!strcmp(last_step,"listener-nonblocking") && last_rc==-9);
    fault=0;assert(listen_socket()==7 && closed==3 && last_rc==0);
    return 0;
}
''')

    def test_heartbeat_raw_ownership_and_listener_rebind(self):
        self.run_c(r'''
#include <assert.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/socket.h>
#include <netinet/in.h>
/* Bind is denied by the host sandbox. Model exclusive port ownership,
 * while socket/socketpair, close and F_GETFD use real kernel descriptors. */
static int bound6000=-1,bound6001=-1,accepted=-1;
static int fake_bind(int fd,const struct sockaddr *addr,socklen_t n){
    (void)n;int *bound=ntohs(((const struct sockaddr_in*)addr)->sin_port)==6001?&bound6001:&bound6000;
    assert(*bound==-1);*bound=fd;return 0;
}
static int fake_listen(int fd,int backlog){(void)fd;(void)backlog;return 0;}
static int fake_accept(int fd,struct sockaddr *a,socklen_t *n){(void)fd;(void)a;(void)n;int c=accepted;accepted=-1;return c;}
static int real_close(int fd){if(bound6000==fd)bound6000=-1;if(bound6001==fd)bound6001=-1;return close(fd);}
#define bind fake_bind
#define listen fake_listen
#define accept fake_accept
#define close real_close
static int listener_lock,listener_fd=-1,active_sockets[32];
static void mutexLock(int *m){(void)m;}
static void mutexUnlock(int *m){(void)m;}
''' + function('int sa_close(int fd)') + '\n' + function('void sa_listener_close(int fd)') + '\n' + function('int sa_heartbeat_close(int fd)') + '\n' + function('int sa_heartbeat_listen(void)') + '\n' + function('int sa_heartbeat_accept(int fd)') + r'''
int main(void){
    int file=open("/dev/null",O_RDONLY);assert(file>=0);
    assert(sa_close(file)==0 && fcntl(file,F_GETFD)>=0);
    int hb=sa_heartbeat_listen();assert(hb>=0);
    struct sockaddr_in a={.sin_family=AF_INET,.sin_port=htons(6001),.sin_addr={htonl(INADDR_LOOPBACK)}};
    for(int i=0;i<256;i++) {
        int pair[2];assert(socketpair(AF_UNIX,SOCK_STREAM,0,pair)==0);
        int peer=pair[0];accepted=pair[1];
        int client=sa_heartbeat_accept(hb);assert(client>=0);
        assert(sa_heartbeat_close(client)==0);
        assert(fcntl(client,F_GETFD)==-1 && errno==EBADF);
        assert(fcntl(hb,F_GETFD)>=0);close(peer);
    }
    assert(sa_heartbeat_close(hb)==0);
    assert(fcntl(hb,F_GETFD)==-1 && errno==EBADF);
    hb=sa_heartbeat_listen();assert(hb>=0);sa_heartbeat_close(hb);
    a.sin_port=htons(6000);
    for(int i=0;i<128;i++) {
        listener_fd=socket(AF_INET,SOCK_STREAM,0);assert(listener_fd>=0);
        assert(bind(listener_fd,(void*)&a,sizeof(a))==0);
        assert(listen(listener_fd,8)==0);
        int old=listener_fd;
        assert(sa_close(old)==0 && fcntl(old,F_GETFD)>=0);
        sa_listener_close(old);
        assert(listener_fd==-1 && fcntl(old,F_GETFD)==-1 && errno==EBADF);
        /* Next iteration must bind only after the old owner released its fd. */
    }
    assert(fcntl(file,F_GETFD)>=0);close(file);
    return 0;
}
''')

    def test_socket_registry_releases_every_slot(self):
        self.run_c(r'''
#include <assert.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/socket.h>
static int listener_lock,active_sockets[32];
static void mutexLock(int *m){(void)m;}
static void mutexUnlock(int *m){(void)m;}
static int sa_stopping(void){return 0;}
''' + function('static int track_socket(int fd)') + '\n' + function('int sa_close(int fd)') + r'''
int main(void){
    for(int i=0;i<1024;i++){
        int fd=track_socket(socket(AF_UNIX,SOCK_STREAM,0));assert(fd>=0);
        assert(sa_close(fd)==0);assert(fcntl(fd,F_GETFD)==-1 && errno==EBADF);
    }
    int fds[32];
    for(int i=0;i<32;i++){fds[i]=track_socket(socket(AF_UNIX,SOCK_STREAM,0));assert(fds[i]>=0);}
    int extra=socket(AF_UNIX,SOCK_STREAM,0);assert(extra>=0);
    assert(track_socket(extra)==-1 && errno==ECANCELED);
    assert(fcntl(extra,F_GETFD)==-1 && errno==EBADF);
    for(int i=0;i<32;i++)assert(sa_close(fds[i])==0);
    for(int i=0;i<32;i++)assert(active_sockets[i]==0);
    return 0;
}
''')


    def test_heartbeat_nonblocking_and_health_faults(self):
        self.run_c(r'''
#include <stddef.h>
#include <assert.h>
#include <errno.h>
#include <fcntl.h>
#include <sys/socket.h>
#include <netinet/in.h>
static int closed,fail_nonblock,health_fault;
static int fake_socket(int a,int b,int c){(void)a;(void)b;(void)c;return 7;}
static int fake_opt(int a,int b,int c,const void *d,socklen_t e){(void)a;(void)b;(void)c;(void)d;(void)e;return 0;}
static int fake_bind(int a,const struct sockaddr *b,socklen_t c){(void)a;(void)b;(void)c;return 0;}
static int fake_listen(int a,int b){(void)a;(void)b;return 0;}
static int fake_accept(int a,struct sockaddr *b,socklen_t *c){(void)a;(void)b;(void)c;return 8;}
static int fake_fcntl(int a,int b,int c){(void)a;(void)b;(void)c;if(fail_nonblock){errno=13;return -1;}return 0;}
static int fake_close(int fd){assert(fd==7 || fd==8);closed++;errno=9;return 0;}
static int fake_get(int a,int b,int opt,void *p,socklen_t *n){(void)a;(void)b;(void)n;if(health_fault==1)return -1;*(int*)p=opt==SO_ACCEPTCONN ? health_fault!=2 : health_fault==3;return 0;}
#define socket fake_socket
#define setsockopt fake_opt
#define bind fake_bind
#define listen fake_listen
#define accept fake_accept
#define fcntl fake_fcntl
#define close fake_close
#define getsockopt fake_get
''' + function('int sa_heartbeat_listen(void)') + '\n' + function('int sa_heartbeat_accept(int fd)') + '\n' + function('int sa_heartbeat_health(int fd)') + r'''
int main(void){
    fail_nonblock=1;
    assert(sa_heartbeat_listen()==-13 && closed==1);
    assert(sa_heartbeat_accept(7)==-1 && errno==13 && closed==2);
    fail_nonblock=0;assert(sa_heartbeat_listen()==7 && closed==2);
    assert(sa_heartbeat_accept(7)==8 && closed==2);
    for(health_fault=1;health_fault<=3;health_fault++)assert(!sa_heartbeat_health(7));
    health_fault=0;assert(sa_heartbeat_health(7));
    return 0;
}
''')

if __name__ == '__main__':
    unittest.main()
