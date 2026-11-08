"""Execute the production NRO lifecycle with host thread/console adapters."""
import pathlib
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
SOURCE = (ROOT / 'native/shim.c').read_text()


def function(signature):
    start = SOURCE.index(signature + ' {')
    end = SOURCE.index('{', start) + 1
    depth = 1
    while depth:
        depth += (SOURCE[end] == '{') - (SOURCE[end] == '}')
        end += 1
    return SOURCE[start:end]


class ShutdownTest(unittest.TestCase):
    def test_spawn_join_and_flag_propagation(self):
        prelude = r'''
#include <assert.h>
#include <stdatomic.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <pthread.h>
#include <unistd.h>
#include <sys/socket.h>
#include <time.h>
typedef unsigned u32;
typedef uint64_t u64;
typedef int Result;
typedef pthread_mutex_t Mutex;
typedef struct { pthread_t handle; void (*fn)(void*); void *arg; bool joined; } Thread;
#define Module_Libnx 0
#define LibnxError_BadInput 1
#define MAKERESULT(a,b) (b)
static atomic_bool nro_quit;
static Thread threads[6], log_thread;
static bool thread_started[6], log_started;
static Mutex listener_lock=PTHREAD_MUTEX_INITIALIZER;
static int listener_fd=-1;
static int active_sockets[32];
static atomic_int stopped, joins, closes, ready;
static int refuse;
static int blocked_pair[2];
static void mutexLock(Mutex *m) { pthread_mutex_lock(m); }
static void mutexUnlock(Mutex *m) { pthread_mutex_unlock(m); }
static void *run(void *p) { Thread *t=p; t->fn(t->arg); return NULL; }
static Result threadCreate(Thread *t,void (*fn)(void*),void *arg,void *stack,size_t size,int priority,int core) {
    (void)stack;(void)size;(void)priority;(void)core;t->fn=fn;t->arg=arg;return 0;
}
static Result threadStart(Thread *t) { return pthread_create(&t->handle,NULL,run,t); }
static Result waitSingleHandle(pthread_t h,u64 timeout) { (void)h;(void)timeout;return refuse ? 1 : 0; }
static Result threadWaitForExit(Thread *t) { int rc=pthread_join(t->handle,NULL); t->joined=!rc; atomic_fetch_add(&joins,1);return rc; }
static Result threadClose(Thread *t) { assert(t->joined);atomic_fetch_add(&closes,1);return 0; }
static u64 sa_now_ms(void) { struct timespec t;clock_gettime(CLOCK_MONOTONIC,&t);return t.tv_sec*1000+t.tv_nsec/1000000; }
static void sa_persist(const char *s,size_t n) { (void)n; if(refuse) { assert(strstr(s,"keeping NRO mapped"));puts("timeout logged");fflush(stdout); } }
static void sa_trace(const char *s,int rc) { (void)s;(void)rc; }
static void consoleUpdate(void *p) { (void)p; }
static void svcSleepThread(uint64_t ns) { usleep(ns/1000); }
'''
        body = '\n'.join(function(s) for s in [
            'void sa_quit(void)', 'int sa_stopping(void)',
            'u32 sa_spawn(unsigned slot, void (*fn)(void*), void *arg)',
            'static void close_all_sockets(void)',
            'static bool join_until(Thread *thread, const char *name, u64 deadline)',
            'static void nro_shutdown(Thread *agent, bool started)',
        ])
        trailer = r'''
static void worker(void *p) { if(p) { char b;assert(recv(blocked_pair[0],&b,1,0)<=0); } else while(!sa_stopping())usleep(1000);atomic_fetch_add(&stopped,1); }
static void listener(void *p) {
    (void)p;
    for(unsigned i=0;i<6;i++)assert(!sa_spawn(i,worker,i==0 ? blocked_pair : NULL));
    assert(!threadCreate(&log_thread,worker,NULL,NULL,0,0,0));
    assert(!threadStart(&log_thread));log_started=true;atomic_store(&ready,1);
    while(!sa_stopping())usleep(1000);
    atomic_fetch_add(&stopped,1);
}
int main(int argc,char **argv) {
    (void)argv; refuse=argc>1;
    assert(!socketpair(AF_UNIX,SOCK_STREAM,0,blocked_pair));active_sockets[0]=blocked_pair[0]+1;
    Thread agent={0};assert(!threadCreate(&agent,listener,NULL,NULL,0,0,0));assert(!threadStart(&agent));
    while(!atomic_load(&ready))usleep(1000);
    nro_shutdown(&agent,true);
    char byte;assert(read(blocked_pair[1],&byte,1)==0);close(blocked_pair[1]);
    assert(!active_sockets[0]);
    assert(!refuse);assert(sa_stopping());
    assert(atomic_load(&stopped)==8);
    assert(atomic_load(&joins)==8 && atomic_load(&closes)==8);
    puts("all joined before main return");return 0;
}
'''
        with tempfile.TemporaryDirectory() as tmp:
            source = pathlib.Path(tmp) / 'shutdown.c'
            binary = pathlib.Path(tmp) / 'shutdown'
            source.write_text(prelude + body + trailer)
            subprocess.run(['cc', '-Wall', '-Wextra', '-Werror', '-pthread', str(source), '-o', str(binary)], check=True)
            result = subprocess.run([str(binary)], capture_output=True, text=True, check=True, timeout=5)
            self.assertIn('all joined before main return', result.stdout)
            proc = subprocess.Popen([str(binary), 'refuse'], stdout=subprocess.PIPE, text=True)
            try:
                with self.assertRaises(subprocess.TimeoutExpired):
                    proc.communicate(timeout=0.3)
            finally:
                proc.kill()
                output, _ = proc.communicate()
            self.assertIn('timeout logged', output)

    def test_entry_waits_before_console_exit(self):
        main = function('int main(void)')
        self.assertLess(main.index('nro_shutdown('), main.index('consoleExit('))
        self.assertLess(main.index('consoleExit('), main.index('return 0;'))


if __name__ == '__main__':
    unittest.main()
