/* ABI adapters only: policy, sequencing and command logic live in Rust. */
#include <switch.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <netdb.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>
#include <poll.h>

#ifndef SA_NRO
u32 __nx_applet_type = AppletType_None;
#endif
const char *sa_version(void) { return "swagent-1.34-rs"; }
int sa_is_nro(void) {
#ifdef SA_NRO
    return 1;
#else
    return 0;
#endif
}
/* Process lifetime is owned by the system; commands never stop workers. */
void sa_quit(void) {}
int sa_stopping(void) { return 0; }
#ifndef SA_NRO
/* Resident heap only. hbmenu supplies its heap through the loader ABI. */
void __libnx_initheap(void) {
    static u8 heap[16 * 1024 * 1024] __attribute__((aligned(0x1000)));
    extern void *fake_heap_start, *fake_heap_end;
    fake_heap_start = heap; fake_heap_end = heap + sizeof(heap);
}
void __appInit(void) {}
void __appExit(void) {}
#endif
/* NRO retains libnx default heap and applet/HID/FS initialization. */
extern void agent_main(void);
#ifdef SA_NRO
extern void agent_nro_main(void);
/* UI reads status; only the main thread touches the console. */
#include <stdatomic.h>
static atomic_int nro_listener_status;
static void nro_agent(void *arg) { (void)arg; agent_nro_main(); for (;;) svcSleepThread(100000000); }
int main(void) {
    /* Must be the first call: stdout uses the software console framebuffer.
     * Printing before consoleInit causes ConsoleSwRenderer_drawChar to abort. */
    consoleInit(NULL);
    printf("SWAgent %s\n", sa_version());
    consoleUpdate(NULL);
    /* SD is mounted by the standard libnx __appInit before main. */
    PadState pad;
    padConfigureInput(1, HidNpadStyleSet_NpadStandard);
    padInitializeDefault(&pad);
    Thread agent;
    Result rc = threadCreate(&agent, nro_agent, NULL, NULL, 0x18000, 44, -2);
    if (!rc) { rc = threadStart(&agent); if (rc) threadClose(&agent); }
    if (rc) {
        printf("agent startup failed: 0x%x\npress HOME to exit\n", rc);
        consoleUpdate(NULL);
    }
    int announced=0;
    for (;;) {
        appletMainLoop();
        int status=atomic_load(&nro_listener_status);
        if (status != announced) {
            if (status == 1) {
                struct in_addr ip={.s_addr=gethostid()};
                printf("listening %s:6000\npress HOME to exit\n", inet_ntoa(ip));
                consoleUpdate(NULL);
            } else if (status == -EADDRINUSE) {
                printf("port 6000 busy - sysmodule already running\npress HOME to exit\n");
                consoleUpdate(NULL);
            } else if (status < 0) {
                printf("listener failed: errno=%d; retrying with backoff (up to 30s)\npress HOME to exit\n", -status);
                consoleUpdate(NULL);
            }
            announced=status;
        }
        padUpdate(&pad);
        if (padGetButtonsDown(&pad) & HidNpadButton_Plus) {
            printf("press HOME to exit\n");
            consoleUpdate(NULL);
        }
        consoleUpdate(NULL);
        svcSleepThread(100000000);
    }

}
#else
int main(void) { agent_main(); for (;;) svcSleepThread(3600000000000LL); }
#endif
/* Producers never wait for SD or another producer. One writer flushes each line.
 * The tail is a snapshot of bytes actually read/written, including prior boots. */
#include <stdatomic.h>
static atomic_flag log_lock=ATOMIC_FLAG_INIT;
static char log_queue[128][768], log_tail[16384];
static size_t log_head, log_count, log_tail_len;
static Thread log_thread;
static bool log_started;
u64 sa_now_ms(void);
static atomic_ullong log_beat;
int sa_log_alive(void) { u64 beat=atomic_load(&log_beat); return beat && sa_now_ms()-beat<60000; }
void sa_persist(const char *p, size_t n) {
    if (atomic_flag_test_and_set(&log_lock)) return;
    if (log_count<128) {
        size_t slot=(log_head+log_count)%128;
        if(n>766)n=766;
        memcpy(log_queue[slot],p,n); log_queue[slot][n]='\n'; log_queue[slot][n+1]=0; log_count++;
    }
    atomic_flag_clear(&log_lock);
}
size_t sa_log_tail(char *p,size_t n) {
    if(atomic_flag_test_and_set(&log_lock)) return 0;
    if(n>log_tail_len)n=log_tail_len;
    memcpy(p,log_tail+log_tail_len-n,n);
    atomic_flag_clear(&log_lock); return n;
}
static void log_cache(const char *p,size_t n) {
    if(atomic_flag_test_and_set(&log_lock)) return;
    if(n>sizeof(log_tail)) { p+=n-sizeof(log_tail); n=sizeof(log_tail); }
    if(log_tail_len+n>sizeof(log_tail)) {
        size_t drop=log_tail_len+n-sizeof(log_tail);
        memmove(log_tail,log_tail+drop,log_tail_len-drop); log_tail_len-=drop;
    }
    memcpy(log_tail+log_tail_len,p,n); log_tail_len+=n;
    atomic_flag_clear(&log_lock);
}
static void log_writer(void *arg) {
    (void)arg;
    mkdir("sdmc:/switch",0777);
    FILE *old=fopen("sdmc:/switch/swagent.log","rb");
    if(old) {
        char bytes[16384]; fseek(old,0,SEEK_END); long size=ftell(old);
        if(size>0) { fseek(old,size>16384?size-16384:0,SEEK_SET); size_t n=fread(bytes,1,sizeof(bytes),old); log_cache(bytes,n); }
        fclose(old);
    }
    FILE *f=NULL;
    unsigned failures=0;
    bool reopened=false;
    for(;;) {
        atomic_store(&log_beat,sa_now_ms());
        char line[768]={0};
        if(!atomic_flag_test_and_set(&log_lock)) {
            if(log_count) { memcpy(line,log_queue[log_head],sizeof(line)); log_head=(log_head+1)%128; log_count--; }
            atomic_flag_clear(&log_lock);
        }
        if(!line[0]) { if (sa_stopping()) { if(f) fclose(f); return; } svcSleepThread(10000000); continue; }
        if(!f) f=fopen("sdmc:/switch/swagent.log","ab+");
        size_t n=strlen(line);
        bool ok=false;
        if(f) {
            bool positioned=fseek(f,0,SEEK_END)==0;
            long size=positioned ? ftell(f) : -1;
            if(size>=0 && (unsigned long)size+n>1024*1024) {
                fclose(f); f=fopen("sdmc:/switch/swagent.log","wb");
                if(!atomic_flag_test_and_set(&log_lock)) { log_tail_len=0; atomic_flag_clear(&log_lock); }
            }
            if(f && size>=0) ok=fwrite(line,1,n,f)==n && fflush(f)==0;
        }
        if(ok) { log_cache(line,n); failures=0; reopened=false; }
        else if(++failures>=10 && !reopened) {
            if(f) fclose(f);
            f=fopen("sdmc:/switch/swagent.log","ab+");
            reopened=true;
            /* Retry this line once after reopening; later lines still reach
             * the Rust ring even if SD remains unavailable. */
            if(f && fwrite(line,1,n,f)==n && fflush(f)==0) {
                log_cache(line,n); failures=0; reopened=false;
            }
        }
    }
}
static void sa_trace(const char *step,int rc) {
    char line[192]; snprintf(line,sizeof(line),"boot uptime_ms=%llu step=%s %s rc=%d",(unsigned long long)sa_now_ms(),step,rc?"fail":"ok",rc);
    sa_persist(line,strlen(line));
}
static Semaphore jobs_ready;
void sa_jobs_init(void) { semaphoreInit(&jobs_ready, 0); }
void sa_jobs_signal(void) { semaphoreSignal(&jobs_ready); }
void sa_jobs_wait(void) {
#ifdef SA_NRO
    while (!sa_stopping() && !semaphoreTryWait(&jobs_ready)) svcSleepThread(10000000);
#else
    semaphoreWait(&jobs_ready);
#endif
}
static Thread threads[8];
static bool thread_started[8];
static Mutex listener_lock;
static int listener_fd=-1;
/* Includes sockets still connecting, so exit can interrupt every network lane. */
static int active_sockets[32]; /* fd+1; zero is an unused slot */
static int track_socket(int fd) {
    if (fd < 0) return fd;
    mutexLock(&listener_lock);
    if (!sa_stopping()) for (unsigned i=0;i<32;i++) if (!active_sockets[i]) {
        active_sockets[i]=fd+1;
        mutexUnlock(&listener_lock);
        return fd;
    }
    close(fd);
    mutexUnlock(&listener_lock);
    errno=ECANCELED;
    return -1;
}
int sa_close(int fd) {
    mutexLock(&listener_lock);
    int result=0;
    for (unsigned i=0;i<32;i++) if (active_sockets[i]==fd+1) {
        active_sockets[i]=0;
        result=close(fd);
        break;
    }
    mutexUnlock(&listener_lock);
    return result;
}
void sa_listener_close(int fd) {
    mutexLock(&listener_lock);
    if (listener_fd==fd) { listener_fd=-1; close(fd); }
    mutexUnlock(&listener_lock);
}
u32 sa_spawn(unsigned slot, void (*fn)(void*), void *arg) {
    if (slot >= 8) return MAKERESULT(Module_Libnx,LibnxError_BadInput);
    if (sa_stopping()) return MAKERESULT(Module_Libnx,LibnxError_BadInput);
    sa_trace("thread-spawn-start",0);
    Result rc = threadCreate(&threads[slot], fn, arg, NULL, 0x18000, slot==6 ? 48 : 44, -2);
    if (!rc) { rc = threadStart(&threads[slot]); if (rc) threadClose(&threads[slot]); }
    if (!rc) thread_started[slot]=true;
    char step[40]; snprintf(step,sizeof(step),"thread-%u",slot); sa_trace(step,rc);
    return rc;
}
static bool socket_ok;
static u32 sa_net_init(void) {
    SocketInitConfig cfg = *socketGetDefaultInitConfig();
    cfg.tcp_tx_buf_size = cfg.tcp_rx_buf_size = 0x10000;
    cfg.tcp_tx_buf_max_size = cfg.tcp_rx_buf_max_size = 0x10000;
    cfg.num_bsd_sessions = 3;
    Result rc = socketInitialize(&cfg);
    socket_ok = R_SUCCEEDED(rc);
    return rc;
}
static void reset_network_sockets(void) {
    mutexLock(&listener_lock);
    if (listener_fd>=0) { shutdown(listener_fd,SHUT_RDWR); close(listener_fd); listener_fd=-1; }
    for (unsigned i=0;i<32;i++) if (active_sockets[i]) {
        int fd=active_sockets[i]-1;
        shutdown(fd,SHUT_RDWR);
        close(fd);
        active_sockets[i]=0;
    }
    mutexUnlock(&listener_lock);
}
u32 sa_net_reinit(void) {
    reset_network_sockets();
    if (socket_ok) socketExit();
    socket_ok = false;
    return sa_net_init();
}
u32 sa_init(void) {
#ifdef SA_NRO
    /* libnx already initialized SM/FS and mounted SD. No resident recovery. */
    if (sa_stopping()) return 1;
    Result lr=threadCreate(&log_thread,log_writer,NULL,NULL,0x10000,44,-2);
    if (!lr) { lr=threadStart(&log_thread); if(lr)threadClose(&log_thread); }
    log_started=!lr;
    return sa_net_init();
#else
    static bool sm_ok, fs_ok, sd_ok;
    Result rc;
    if (!sm_ok) { rc=smInitialize(); sa_trace("sm-init",rc); if(rc) return rc; sm_ok=true; }
    if (!hosversionGet() && !setsysInitialize()) {
        SetSysFirmwareVersion fw;
        if (!setsysGetFirmwareVersion(&fw)) hosversionSet(MAKEHOSVERSION(fw.major,fw.minor,fw.micro));
        setsysExit();
    }
    if (!fs_ok) { sa_trace("fs-init-start",0); rc=fsInitialize(); sa_trace("fs-init",rc); if(rc) return rc; fs_ok=true; }
    if (!sd_ok) { sa_trace("fs-mount-start",0); rc=fsdevMountSdmc(); sa_trace("fs-mount",rc); if(rc) return rc; sd_ok=true;
        Result lr=threadCreate(&log_thread,log_writer,NULL,NULL,0x10000,44,-2);
        if(!lr) { lr=threadStart(&log_thread); if(lr)threadClose(&log_thread); }
        log_started=!lr;
        sa_trace("log-thread",lr);
    }
    sa_trace("bsd-init-start",0); rc=sa_net_init(); sa_trace("bsd-init",rc); return rc;
#endif
}
ssize_t sa_send(int fd, const void *p, size_t n, int flags) { return send(fd,p,n,flags); }
ssize_t sa_recv(int fd, void *p, size_t n, int flags) { return recv(fd,p,n,flags); }
int sa_errno(void) { return errno; }
void sa_timeout(int fd, int seconds) {
    struct timeval t = {seconds,0};
    setsockopt(fd,SOL_SOCKET,SO_RCVTIMEO,&t,sizeof(t));
    setsockopt(fd,SOL_SOCKET,SO_SNDTIMEO,&t,sizeof(t));
}
static int listen_socket(void) {
    sa_trace("listener-bind-start",0);
    int fd = socket(AF_INET,SOCK_STREAM,0); if (fd < 0) return -errno;
    int one=1; setsockopt(fd,SOL_SOCKET,SO_REUSEADDR,&one,sizeof(one));
    struct sockaddr_in a = {.sin_family=AF_INET,.sin_port=htons(6000),.sin_addr={INADDR_ANY}};
    sa_trace("listener-socket",0);
    if (bind(fd,(void*)&a,sizeof(a)) < 0) {
        int e=errno; sa_trace("listener-bind",-e); close(fd); return -e;
    }
    sa_trace("listener-bind",0);
    if (listen(fd,4) < 0) {
        int e=errno; sa_trace("listener-listen",-e); close(fd); return -e;
    }
    sa_trace("listener-listen",0);
    /* Poll accept so worker connect failures can wake the recovery loop even
     * when no client is connected. Do not rely on accept honoring RCVTIMEO. */
    if (fcntl(fd,F_SETFL,O_NONBLOCK) < 0) { int e=errno; close(fd); return -e; }
    sa_trace("listener-bind-listen",0); return fd;
}
int sa_listen(void) {
    mutexLock(&listener_lock);
    int result=sa_stopping() ? -ECANCELED : listen_socket();
    if (result>=0) listener_fd=result;
    mutexUnlock(&listener_lock);
#ifdef SA_NRO
    atomic_store(&nro_listener_status,result < 0 ? result : 1);
#endif
    return result;
}
/* Probe listener metadata, not recv(0): recv on a listening socket is invalid.
 * Only close the registered listener under its ownership lock. */
int sa_listener_health(int fd, int stale) {
    mutexLock(&listener_lock);
    int bad=listener_fd!=fd;
    if (!bad) {
        struct sockaddr_in addr; socklen_t len=sizeof(addr);
        int accepting=0, error=0; socklen_t opt=sizeof(int);
        bad=stale || getsockname(fd,(void*)&addr,&len)<0 ||
            getsockopt(fd,SOL_SOCKET,SO_ACCEPTCONN,&accepting,&opt)<0 || !accepting;
        opt=sizeof(int);
        if (!bad) bad=getsockopt(fd,SOL_SOCKET,SO_ERROR,&error,&opt)<0 || error;
        if (bad) {
            sa_trace("listener-watchdog-shutdown-start",0);
            shutdown(fd,SHUT_RDWR);
            sa_trace("listener-watchdog-close-start",0);
            close(fd); listener_fd=-1;
            sa_trace("listener-watchdog-closed",0);
        }
    }
    mutexUnlock(&listener_lock);
    return bad;
}
int sa_accept(int fd) {
    int client=accept(fd,NULL,NULL);
    if (client>=0 && fcntl(client,F_SETFL,O_NONBLOCK)<0) {
        int e=errno; close(client); errno=e; return -1;
    }
    if (client>=0) sa_timeout(client,2);
    return track_socket(client);
}
int sa_connect(const char *host, const char *port) {
    struct addrinfo hint = {.ai_family=AF_INET,.ai_socktype=SOCK_STREAM}, *list=NULL;
    if (getaddrinfo(host,port,&hint,&list)) { sa_persist("connect DNS fail rc=-1 errno=113",strlen("connect DNS fail rc=-1 errno=113")); errno=EHOSTUNREACH; return -1; }
    int saved=EHOSTUNREACH; unsigned attempt=0;
    int fd=-1;
    struct addrinfo *p=list;
    for (attempt=1;p && attempt<=3 && !sa_stopping();attempt++) {
        char trace[256], ip[INET_ADDRSTRLEN];
        struct sockaddr_in *dest=(struct sockaddr_in*)p->ai_addr;
        inet_ntop(AF_INET,&dest->sin_addr,ip,sizeof(ip));
        snprintf(trace,sizeof(trace),"connect retry attempt=%u destination=%s:%u",attempt,ip,ntohs(dest->sin_port)); sa_persist(trace,strlen(trace));
        fd=track_socket(socket(p->ai_family,p->ai_socktype,p->ai_protocol));
        if (fd<0) { saved=errno; goto next; }
        sa_timeout(fd,2);
        int flags=fcntl(fd,F_GETFL);
        if (flags<0 || fcntl(fd,F_SETFL,flags|O_NONBLOCK)<0) { saved=errno; goto failed; }
        snprintf(trace,sizeof(trace),"connect start destination=%s:%u",ip,ntohs(dest->sin_port)); sa_persist(trace,strlen(trace));
        u64 deadline=sa_now_ms()+15000;
        int cr=connect(fd,p->ai_addr,p->ai_addrlen);
        int ce=cr ? errno : 0;
        while (cr && (ce==EINPROGRESS || ce==EALREADY || ce==EINTR || ce==EWOULDBLOCK)) {
            if (sa_stopping()) { ce=ECANCELED; break; }
            u64 now=sa_now_ms();
            if (now>=deadline) {
                ce=ETIMEDOUT;
                snprintf(trace,sizeof(trace),"connect timeout attempt=%u",attempt); sa_persist(trace,strlen(trace));
                break;
            }
            struct pollfd pollfd={.fd=fd,.events=POLLOUT};
            int ready=poll(&pollfd,1,(int)((deadline-now)>100 ? 100 : deadline-now));
            if (ready<0) { if(errno==EINTR) continue; ce=errno; break; }
            if (!ready) continue;
            socklen_t len=sizeof(ce);
            if (getsockopt(fd,SOL_SOCKET,SO_ERROR,&ce,&len)<0) ce=errno;
            if (!ce) cr=0;
            break;
        }
        snprintf(trace,sizeof(trace),"connect rc=%d errno=%d destination=%s:%u",cr,ce,ip,ntohs(dest->sin_port)); sa_persist(trace,strlen(trace));
        if (!cr) break; /* Retain nonblocking mode for bounded send/recv. */
        saved=ce;
failed:
        sa_close(fd); fd=-1;
next:
        p=p->ai_next ? p->ai_next : list;
    }
    freeaddrinfo(list); if (fd<0) errno=saved; return fd;
}
/* File objects and all service objects are 16-byte libnx Service wrappers. */
_Static_assert(sizeof(FsFile)==16 && sizeof(NcmContentStorage)==16, "service ABI");
_Static_assert(sizeof(NcmContentMetaKey)==16 && sizeof(NcmContentInfo)==24, "NCM ABI");
u32 sa_open(const char *path, int write, FsFile *file) {
    FsFileSystem *fs; char p[FS_MAX_PATH];
    if (fsdevTranslatePath(path,&fs,p)) return MAKERESULT(Module_Libnx,LibnxError_BadInput);
    if (write == 1) { fsFsDeleteFile(fs,p); Result rc=fsFsCreateFile(fs,p,0,0); if (rc) return rc; }
    return fsFsOpenFile(fs,p,write ? FsOpenMode_Write|FsOpenMode_Append : FsOpenMode_Read,file);
}
u32 sa_sd_space(s64 *total, s64 *free_space) {
    FsFileSystem *fs = fsdevGetDeviceFileSystem("sdmc");
    if (!fs) return MAKERESULT(Module_Libnx,LibnxError_NotFound);
    Result rc = fsFsGetTotalSpace(fs,"/",total);
    if (!rc) rc = fsFsGetFreeSpace(fs,"/",free_space);
    return rc;
}
u32 sa_remove(const char *path) {
    FsFileSystem *fs; char p[FS_MAX_PATH];
    if (fsdevTranslatePath(path,&fs,p)) return MAKERESULT(Module_Libnx,LibnxError_BadInput);
    return fsFsDeleteFile(fs,p);
}
u32 sa_rename(const char *from, const char *to) {
    FsFileSystem *fs,*other; char a[FS_MAX_PATH],b[FS_MAX_PATH];
    if (fsdevTranslatePath(from,&fs,a) || fsdevTranslatePath(to,&other,b) || fs!=other)
        return MAKERESULT(Module_Libnx,LibnxError_BadInput);
    return fsFsRenameFile(fs,a,b);
}
/* Goldleaf stages metadata on BIS SYSTEM, addressed through SystemContent.
 * Keep the original content-id filename; @Sdcard is not a content mount. */
#define SA_STAGE_DIR "/Contents/swagent-install"
u32 sa_stage_open(const char *name, s64 size, FsFile *out, FsFileSystem *fs) {
    Result rc=fsOpenBisFileSystem(fs,FsBisPartitionId_System,"");
    if (rc) return rc;
    fsFsDeleteDirectoryRecursively(fs,SA_STAGE_DIR);
    rc=fsFsCreateDirectory(fs,SA_STAGE_DIR);
    char path[FS_MAX_PATH];
    snprintf(path,sizeof(path),SA_STAGE_DIR "/%s",name);
    if (!rc) rc=fsFsCreateFile(fs,path,size,0);
    if (!rc) rc=fsFsOpenFile(fs,path,FsOpenMode_Write,out);
    if (rc) {
        fsFsDeleteDirectoryRecursively(fs,SA_STAGE_DIR);
        fsFsCommit(fs);
        fsFsClose(fs);
    }
    return rc;
}
u32 sa_stage_commit(FsFileSystem *fs) { return fsFsCommit(fs); }
void sa_stage_close(FsFileSystem *fs) {
    fsFsDeleteDirectoryRecursively(fs,SA_STAGE_DIR);
    fsFsCommit(fs);
    fsFsClose(fs);
}
u32 sa_cnmt_open(const char *nca, FsFile *out, FsFileSystem *outfs) {
    Result rc = fsOpenFileSystemWithId(outfs,0,FsFileSystemType_ContentMeta,nca,FsContentAttributes_All);
    if (rc) return rc;
    FsDir dir; rc=fsFsOpenDirectory(outfs,"/",FsDirOpenMode_ReadFiles,&dir);
    if (rc) { fsFsClose(outfs); return rc; }
    FsDirectoryEntry e; s64 n; int found=0;
    while (!(rc=fsDirRead(&dir,&n,1,&e)) && n) {
        size_t len=strlen(e.name);
        if (len>=5 && !strcmp(e.name+len-5,".cnmt")) { found=1; break; }
    }
    fsDirClose(&dir);
    if (!rc && found) {
        char p[FS_MAX_PATH]; p[0]='/'; strncpy(p+1,e.name,sizeof(p)-2); p[sizeof(p)-1]=0;
        rc=fsFsOpenFile(outfs,p,FsOpenMode_Read,out);
    } else if (!rc) rc=MAKERESULT(Module_Libnx,LibnxError_NotFound);
    if (rc) fsFsClose(outfs);
    return rc;
}
u32 sa_ticket(const void *tik,size_t tn,const void *cert,size_t cn) {
    Service es; Result rc=smGetService(&es,"es"); if (rc) return rc;
    rc=serviceDispatch(&es,1,
        .buffer_attrs={SfBufferAttr_In|SfBufferAttr_HipcMapAlias,SfBufferAttr_In|SfBufferAttr_HipcMapAlias},
        .buffers={{tik,tn},{cert,cn}});
    serviceClose(&es); return rc;
}
/* NS raw commands 16/17/27, payload layouts constructed in Rust. */
u32 sa_ns(unsigned cmd,const void *in,u32 in_size,void *buf,size_t size,u32 *count) {
    Service owned={0},*s; Result rc;
    if (hosversionAtLeast(3,0,0)) {
        rc=nsGetApplicationManagerInterface(&owned); if(rc) return rc; s=&owned;
    } else s=nsGetServiceSession_ApplicationManagerInterface();
    SfDispatchParams p={0};
    if(size) { p.buffer_attrs.attr0=(cmd==17?SfBufferAttr_Out:SfBufferAttr_In)|SfBufferAttr_HipcMapAlias; p.buffers[0]=(SfBuffer){buf,size}; }
    rc=serviceDispatchImpl(s,cmd,in,in_size,cmd==17?count:NULL,cmd==17?4:0,p);
    if (s==&owned) serviceClose(&owned);
    return rc;
}
/* Foreground launch requests go through AM's debug interface, not PM.
 * This root interface does not require an applet proxy. AM caches the
 * am.debug!dev_function setting at boot; see LAUNCH_RESEARCH.md. */
u32 sa_request_launch(u64 title_id) {
    if (hosversionBefore(6,0,0))
        return MAKERESULT(Module_Libnx,LibnxError_IncompatSysVer);
    Service root={0},debug={0};
    Result rc=smGetService(&root,"appletAE");
    if (rc) return rc;
    rc=serviceConvertToDomain(&root);
    if (!rc) rc=serviceDispatch(&root,1000);
    /* Check the actual object count before reading an object ID. AM may
     * succeed without returning an interface when debug functions are off. */
    if (!rc) {
        HipcResponse response=hipcParseResponse(armGetTls());
        const CmifDomainOutHeader *domain=cmifGetAlignedDataStart(response.data_words,armGetTls());
        if (domain->num_out_objects!=1) {
            rc=MAKERESULT(Module_Libnx,LibnxError_NotInitialized);
        } else {
            CmifResponse parsed={0};
            rc=cmifParseResponse(&parsed,armGetTls(),true,0);
            if (!rc) {
                u32 object_id=cmifResponseGetObject(&parsed);
                if (object_id) serviceCreateDomainSubservice(&debug,&root,object_id);
                else rc=MAKERESULT(Module_Libnx,LibnxError_NotInitialized);
            }
        }
    }
    if (!rc) {
        const struct { u8 flag; u64 application_id; } in={0,title_id};
        _Static_assert(sizeof(in)==16,"AM launch request ABI");
        rc=serviceDispatchIn(&debug,30,in,
            .buffer_attrs={SfBufferAttr_In|SfBufferAttr_HipcMapAlias,
                           SfBufferAttr_In|SfBufferAttr_HipcMapAlias},
            .buffers={{NULL,0},{NULL,0}});
    }
    if (serviceGetObjectId(&debug)) serviceClose(&debug);
    serviceClose(&root);
    return rc;
}
/* Resident self-update still launches a background program through PM. */
u32 sa_launch(u64 *pid) {
    Result rc=pmshellInitialize(); if(rc) return rc;
    NcmProgramLocation loc={.program_id=0x430000000000000B,.storageID=NcmStorageId_None};
    rc=pmshellLaunchProgram(PmLaunchFlag_None,&loc,pid);
    pmshellExit(); return rc;
}
/* Process termination belongs to pm:shell, not pm:dmnt, in libnx. */
u32 sa_terminate(u64 pid) {
    Result rc=pmshellInitialize(); if(rc) return rc;
    rc=pmshellTerminateProcess(pid); pmshellExit(); return rc;
}

/* Guard third-party libc fatal fallbacks against process teardown. */
extern void agent_park(void);
void __wrap_exit(int status) { (void)status; agent_park(); __builtin_unreachable(); }
void __wrap__exit(int status) { (void)status; agent_park(); __builtin_unreachable(); }
void __wrap_abort(void) { agent_park(); __builtin_unreachable(); }

/* Fixed read-only adapters; no arbitrary settings item IDs or writers. */
u32 sa_setting_read(u32 key, char *out, size_t capacity) {
    Result rc;
    if (key <= 2) {
        if (key == 1) {
            rc = setInitialize();
            if (R_FAILED(rc)) return rc;
            SetSysDeviceNickName name = {0};
            rc = setGetDeviceNickname(&name);
            if (R_SUCCEEDED(rc)) snprintf(out, capacity, "%.*s", (int)sizeof(name.nickname), name.nickname);
            setExit();
            return rc;
        }
        rc = setsysInitialize();
        if (R_FAILED(rc)) return rc;
        if (key == 0) {
            SetSysFirmwareVersion fw = {0};
            rc = setsysGetFirmwareVersion(&fw);
            if (R_SUCCEEDED(rc)) snprintf(out, capacity, "%u.%u.%u", fw.major, fw.minor, fw.micro);
        } else {
            SetSysSerialNumber serial = {0};
            rc = setsysGetSerialNumber(&serial);
            if (R_SUCCEEDED(rc)) snprintf(out, capacity, "%.*s", (int)sizeof(serial.number), serial.number);
        }
        setsysExit();
        return rc;
    }
    if (key == 3) {
        rc = psmInitialize();
        if (R_FAILED(rc)) return rc;
        u32 percent = 0;
        rc = psmGetBatteryChargePercentage(&percent);
        if (R_SUCCEEDED(rc)) snprintf(out, capacity, "%u%%", percent);
        psmExit();
        return rc;
    }
    if (key == 4) {
        rc = tsInitialize();
        if (R_FAILED(rc)) return rc;
        TsSession session = {0};
        rc = tsOpenSession(&session, TsDeviceCode_LocationExternal);
        if (R_SUCCEEDED(rc)) {
            float temperature = 0;
            rc = tsSessionGetTemperature(&session, &temperature);
            if (R_SUCCEEDED(rc)) snprintf(out, capacity, "%d C", (int)temperature);
            tsSessionClose(&session);
        }
        tsExit();
        return rc;
    }
    return MAKERESULT(Module_Libnx, LibnxError_BadInput);
}

/* Read-only power telemetry. Sessions live only for this worker command. */
u32 sa_power_psm(u32 *percent, u32 *charger) {
    Result rc = psmInitialize();
    if (R_FAILED(rc)) return rc;
    rc = psmGetBatteryChargePercentage(percent);
    PsmChargerType type;
    *charger = UINT32_MAX;
    if (R_SUCCEEDED(psmGetChargerType(&type))) *charger = (u32)type;
    psmExit();
    return rc;
}

/* Hekate bdk/power/max17050.h: VCELL=0x09, Current=0x0A.
 * bdk/power/max17050.c: I2C_1, address 0x36, little-endian words.
 * Horizon's device ID selects that bus/address; no direct MMIO is needed. */
static Result sa_gauge_reg(I2cSession *session, u8 reg, u8 *out) {
    Result rc = i2csessionSendAuto(session, &reg, 1, I2cTransactionOption_Start);
    if (R_SUCCEEDED(rc))
        rc = i2csessionReceiveAuto(session, out, 2, I2cTransactionOption_All);
    return rc;
}
u32 sa_power_gauge(u8 *voltage, u8 *current) {
    Result rc = i2cInitialize();
    if (R_FAILED(rc)) return rc;
    I2cSession session = {0};
    rc = i2cOpenSession(&session, I2cDevice_Max17050);
    if (R_SUCCEEDED(rc)) {
        rc = sa_gauge_reg(&session, 0x09, voltage);
        if (R_SUCCEEDED(rc)) rc = sa_gauge_reg(&session, 0x0A, current);
        i2csessionClose(&session);
    }
    i2cExit();
    return rc;
}

u32 sa_mkdir(const char *path) {
    FsFileSystem *fs; char p[FS_MAX_PATH];
    if (fsdevTranslatePath(path,&fs,p)) return MAKERESULT(Module_Libnx,LibnxError_BadInput);
    return fsFsCreateDirectory(fs,p);
}
u32 sa_dir_open(const char *path, FsDir *out) {
    FsFileSystem *fs; char p[FS_MAX_PATH];
    if (fsdevTranslatePath(path,&fs,p)) return MAKERESULT(Module_Libnx,LibnxError_BadInput);
    return fsFsOpenDirectory(fs,p,FsDirOpenMode_ReadDirs|FsDirOpenMode_ReadFiles,out);
}
u32 sa_dir_read(FsDir *dir, char *name, u64 *size, int *type, s64 *count) {
    FsDirectoryEntry entry;
    Result rc=fsDirRead(dir,count,1,&entry);
    if (!rc && *count) {
        memcpy(name,entry.name,FS_MAX_PATH);
        *size=entry.file_size; *type=entry.type;
    }
    return rc;
}

/* NIFM is command-scoped: no global session, request, or background IPC.
 * A command returns only after Cancel and serviceClose have completed. IPC
 * itself may block in Horizon; never return leaving a quarantined NIFM lane. */
extern void sa_wifi_log(u32 step, u32 rc);
#define WIFI_TIMEOUT MAKERESULT(Module_Kernel, 117)
#define WIFI_BAD MAKERESULT(Module_Libnx, LibnxError_BadInput)
#define WIFI_STEP(step, expression) (rc=(expression), sa_wifi_log(step,rc), rc==0)
u32 sa_wifi(u32 join,const u8 *profile,u8 *records,s32 *count) {
    Result rc=0;
    *count=0;
    Service root={0}, general={0}, request={0};
    if (!WIFI_STEP(1,smGetService(&root,"nifm:a"))) goto out;
    u64 reserved=0;
    if (!WIFI_STEP(2,serviceDispatchIn(&root,5,reserved,
        .in_send_pid=true,.out_num_objects=1,.out_objects=&general))) goto out;
    if (!join) {
        /* SfNetworkProfileBasicInfo is packed, stride 0x75. All profile types
         * (User|SsidList|Temporary); admin is required for the latter two. */
        u8 types=7;
        if (!WIFI_STEP(3,serviceDispatchInOut(&general,7,types,*count,
            .buffer_attrs={SfBufferAttr_Out|SfBufferAttr_HipcMapAlias},
            .buffers={{records,0x75 * 256}}))) goto out;
        if (*count<0 || *count>256) rc=WIFI_BAD;
        goto out;
    }
    Uuid uuid={0};
    if (!WIFI_STEP(4,serviceDispatchOut(&general,9,uuid,
        .buffer_attrs={SfBufferAttr_In|SfBufferAttr_HipcPointer|SfBufferAttr_FixedSize},
        .buffers={{profile,0x17c}}))) goto out;
    s32 preset=2;
    if (!WIFI_STEP(5,serviceDispatchIn(&general,4,preset,
        .out_num_objects=1,.out_objects=&request))) goto out;
    Handle events[2]={0};
    Result event_rc=serviceDispatch(&request,2,
        .out_handle_attrs={SfOutHandleAttr_HipcCopy,SfOutHandleAttr_HipcCopy},
        .out_handles=events);
    for (unsigned i=0;i<2;i++) if (events[i]) svcCloseHandle(events[i]);
    if (!WIFI_STEP(11,event_rc)) goto out;
    u32 state=0;
    if (!WIFI_STEP(6,serviceDispatchOut(&request,0,state))) goto out;
    if (state!=1) { rc=WIFI_BAD; sa_wifi_log(6,rc); goto out; }
    if (!WIFI_STEP(7,serviceDispatchIn(&request,9,uuid))) goto out;
    if (!WIFI_STEP(8,serviceDispatch(&request,4))) goto out;
    /* Observe availability before discarding the request. SetNetworkProfile
     * above persists the profile; this request must never outlive the command. */
    u64 deadline=armGetSystemTick()+armNsToTicks(5000000000ULL);
    for (;;) {
        if (!WIFI_STEP(6,serviceDispatchOut(&request,0,state))) goto out;
        if (state==NifmRequestState_Available) break;
        if (state==NifmRequestState_Invalid || state==NifmRequestState_Unknown1) {
            rc=WIFI_BAD;
            goto out;
        }
        if (armGetSystemTick()>=deadline) { rc=WIFI_TIMEOUT; goto out; }
        svcSleepThread(10000000);
    }
out:
    if (serviceIsActive(&request)) {
        Result cancel_rc=serviceDispatch(&request,3); /* IRequest::Cancel */
        sa_wifi_log(12,cancel_rc);
        if (!rc) rc=cancel_rc;
    }
    serviceClose(&request);
    serviceClose(&general);
    serviceClose(&root);
    return rc;
}

#undef WIFI_STEP

_Static_assert(sizeof(NifmSfNetworkProfileData)==0x17c, "NIFM profile wire size");
_Static_assert(__builtin_offsetof(NifmSfNetworkProfileData,wireless_setting_data)==0x116, "NIFM wireless offset");
_Static_assert(__builtin_offsetof(NifmSfWirelessSettingData,passphrase)==0x24, "NIFM shared key offset");

u64 sa_now_ms(void) { return armTicksToNs(armGetSystemTick()) / 1000000; }

u64 sa_thread_id(void) { u64 id=0; svcGetThreadId(&id,CUR_THREAD_HANDLE); return id; }

int sa_nonblocking(int fd) { int flags=fcntl(fd,F_GETFL); return flags>=0 && (flags&O_NONBLOCK); }

/* Use the install path's application-manager interface directly. ns:su is
 * SystemUpdate and does not implement ListApplicationRecord. Avoid nsInitialize's
 * service-selection loop and optional control-data IPC during enumeration. */
u32 sa_installed(s32 offset,u64 *title,char *name,size_t capacity,s32 *count) {
    Service root={0}, manager={0};
    Handle handle=INVALID_HANDLE;
    *count=0;
    if(capacity) name[0]=0;
    Result rc=smGetServiceOriginal(&handle,smEncodeName(hosversionBefore(3,0,0)?"ns:am":"ns:am2"));
    sa_trace("installed-open",rc);
    if(rc) return rc;
    serviceCreate(&root,handle);
    Service *session=&root;
    if(hosversionAtLeast(3,0,0)) {
        rc=serviceDispatch(&root,7996,.out_num_objects=1,.out_objects=&manager);
        session=&manager;
    }
    NsApplicationRecord record={0};
    if(!rc) rc=serviceDispatchInOut(session,0,offset,*count,
        .buffer_attrs={SfBufferAttr_Out|SfBufferAttr_HipcMapAlias},
        .buffers={{&record,sizeof(record)}});
    if(!rc && *count) *title=record.application_id;
    sa_trace("installed-list",rc);
    serviceClose(&manager);
    serviceClose(&root);
    return rc;
}
u32 sa_installed_delete(u64 title) {
    Result rc=nsInitialize(); if(rc)return rc;
    rc=nsDeleteApplicationCompletely(title); nsExit(); return rc;
}

extern u32 agent_dump_progress(void);

/* Dump adapters: NCM direct, never ns:dev. Callers serialize the long lane. */
u32 sa_dump_meta(u64 title,u32 *storage,NcmContentInfo *infos,u32 capacity,u32 *count) {
    if (!capacity || capacity>1026) return MAKERESULT(Module_Libnx,LibnxError_BadInput);
    Result rc=ncmInitialize(); if(rc)return rc;
    const NcmStorageId ids[]={NcmStorageId_SdCard,NcmStorageId_BuiltInUser,NcmStorageId_BuiltInSystem};
    bool found=false; NcmContentMetaKey best={0}; u32 selected=0;
    for(unsigned i=0;i<3;i++) {
        NcmContentMetaDatabase db; rc=ncmOpenContentMetaDatabase(&db,ids[i]); if(rc)continue;
        NcmContentMetaKey key={0}; rc=ncmContentMetaDatabaseGetLatestContentMetaKey(&db,&key,title);
        ncmContentMetaDatabaseClose(&db);
        if(!rc && key.id==title && (!found || key.version>best.version)) { best=key; selected=ids[i]; found=true; }
    }
    if(!found) { ncmExit(); return MAKERESULT(Module_Libnx,LibnxError_NotFound); }
    if((best.type!=NcmContentMetaType_Application && best.type!=NcmContentMetaType_Patch) || best.install_type!=NcmContentInstallType_Full) {
        ncmExit(); return MAKERESULT(Module_Libnx,LibnxError_BadInput);
    }
    NcmContentMetaDatabase db; rc=ncmOpenContentMetaDatabase(&db,selected);
    if(!rc) {
        s32 n=0; rc=ncmContentMetaDatabaseListContentInfo(&db,&n,infos,capacity,&best,0);
        if(!rc && (n<=0 || (u32)n>=capacity)) rc=MAKERESULT(Module_Libnx,LibnxError_BadInput);
        if(!rc) { *storage=selected; *count=n; }
        ncmContentMetaDatabaseClose(&db);
    }
    ncmExit(); return rc;
}
u32 sa_dump_read(u32 storage,const NcmContentId *id,u64 offset,void *bytes,size_t size) {
    Result rc=ncmInitialize(); if(rc)return rc;
    NcmContentStorage cs; rc=ncmOpenContentStorage(&cs,storage);
    if(!rc) { rc=ncmContentStorageReadContentIdFile(&cs,bytes,size,id,offset); ncmContentStorageClose(&cs); }
    ncmExit(); return rc;
}
u32 sa_dump_mkdir(const char *path) {
    FsFileSystem *fs; char p[FS_MAX_PATH];
    if(fsdevTranslatePath(path,&fs,p))return MAKERESULT(Module_Libnx,LibnxError_BadInput);
    FsDirEntryType kind;
    Result rc=fsFsGetEntryType(fs,p,&kind);
    if(!rc)return kind==FsDirEntryType_Dir ? 0 : MAKERESULT(Module_Libnx,LibnxError_BadInput);
    return fsFsCreateDirectory(fs,p);
}
static Result dump_save(FsFileSystem *fs,u64 id) {
    FsSaveDataAttribute attr={0}; attr.system_save_data_id=id; attr.save_data_type=FsSaveDataType_System;
    return fsOpenReadOnlySaveDataFileSystem(fs,FsSaveDataSpaceId_System,&attr);
}
static Result dump_read_exact(FsFile *f,u64 offset,void *p,size_t size) {
    u64 n=0; Result rc=fsFileRead(f,offset,p,size,FsReadOption_None,&n);
    return rc ? rc : n==size ? 0 : MAKERESULT(Module_Libnx,LibnxError_BadInput);
}
/* Common tickets/certificates use the ES export IPC (22/23). Personalized
 * plaintext records use read-only savedata; encrypted records fail explicitly. */
u32 sa_dump_ticket(u32 storage,const NcmContentInfo *infos,u32 count,u8 *rights,void *ticket,size_t tcap,size_t *tn,void *cert,size_t ccap,size_t *cn) {
    *tn=*cn=0; memset(rights,0,16);
    Result rc=ncmInitialize(); if(rc)return rc;
    NcmContentStorage cs; rc=ncmOpenContentStorage(&cs,storage);
    if(!rc) {
        for(u32 i=0;i<count;i++) {
            NcmRightsId rid={0}; rc=ncmContentStorageGetRightsIdFromContentId(&cs,&rid,&infos[i].content_id,FsContentAttributes_All);
            if(!rc)rc=agent_dump_progress();
            if(rc)break;
            const u8 zero[16]={0};
            if(memcmp(&rid.rights_id,zero,16)) {
                if(memcmp(rights,zero,16) && memcmp(rights,&rid.rights_id,16)) { rc=MAKERESULT(Module_Libnx,LibnxError_BadInput); break; }
                memcpy(rights,&rid.rights_id,16);
            }
        }
        ncmContentStorageClose(&cs);
    }
    ncmExit(); if(rc)return rc;
    const u8 zero[16]={0}; if(!memcmp(rights,zero,16))return 0;
    Service es; rc=smGetService(&es,"es"); if(rc)return rc;
    int type=-1;
    for(int kind=0;kind<2 && type<0 && !rc;kind++) {
        s32 total=0; rc=serviceDispatchOut(&es,9+kind,total);
        if(rc)break;
        if(total<0 || total>1048576) { rc=MAKERESULT(Module_Libnx,LibnxError_BadInput); break; }
        for(s32 offset=0;offset<total;) {
            FsRightsId ids[64]; s32 written=0;
            rc=serviceDispatchInOut(&es,11+kind,offset,written,
                .buffer_attrs={SfBufferAttr_Out|SfBufferAttr_HipcMapAlias},.buffers={{ids,sizeof(ids)}});
            if(rc)break;
            if(written<=0 || written>64) { rc=MAKERESULT(Module_Libnx,LibnxError_BadInput); break; }
            for(s32 i=0;i<written;i++) if(!memcmp(rights,&ids[i],16))type=kind;
            offset+=written;
            rc=agent_dump_progress(); if(rc)break;
        }
    }
    if(!rc && type==0) {
        FsRightsId input; memcpy(&input,rights,16);
        struct { u64 ticket_size,cert_size; } sizes={0},actual={0};
        rc=serviceDispatchInOut(&es,22,input,sizes);
        if(!rc && (!sizes.ticket_size || sizes.ticket_size>tcap || !sizes.cert_size || sizes.cert_size>ccap)) rc=MAKERESULT(Module_Libnx,LibnxError_BadInput);
        if(!rc) rc=serviceDispatchInOut(&es,23,input,actual,
            .buffer_attrs={SfBufferAttr_Out|SfBufferAttr_HipcMapAlias,SfBufferAttr_Out|SfBufferAttr_HipcMapAlias},
            .buffers={{ticket,sizes.ticket_size},{cert,sizes.cert_size}});
        if(!rc && (actual.ticket_size!=sizes.ticket_size || actual.cert_size!=sizes.cert_size)) rc=MAKERESULT(Module_Libnx,LibnxError_BadInput);
        if(!rc) { *tn=actual.ticket_size; *cn=actual.cert_size; }
        serviceClose(&es); return rc;
    }
    serviceClose(&es); if(rc || type<0)return rc;
    FsFileSystem fs; rc=dump_save(&fs,0x80000000000000e1ULL+type); if(rc)return rc;
    FsFile list; rc=fsFsOpenFile(&fs,"/ticket_list.bin",FsOpenMode_Read,&list);
    u64 ticket_offset=UINT64_MAX;
    if(!rc) {
        s64 size=0; rc=fsFileGetSize(&list,&size);
        if(!rc && (size<0 || size%32))rc=MAKERESULT(Module_Libnx,LibnxError_BadInput);
        for(s64 off=0;!rc && off<size;) {
            u8 entries[4096]; size_t n=(size-off)>(s64)sizeof(entries) ? sizeof(entries) : (size_t)(size-off);
            rc=dump_read_exact(&list,off,entries,n);
            if(!rc)rc=agent_dump_progress();
            for(size_t i=0;!rc && i<n;i+=32) if(!memcmp(entries+i,rights,16)) {ticket_offset=((u64)off+i)*32;break;}
            if(ticket_offset!=UINT64_MAX)break;
            off+=n;
        }
        fsFileClose(&list);
    }
    if(!rc && ticket_offset==UINT64_MAX)rc=MAKERESULT(Module_Libnx,LibnxError_NotFound);
    FsFile tik;
    if(!rc) {
        rc=fsFsOpenFile(&fs,"/ticket.bin",FsOpenMode_Read,&tik);
        if(!rc) { if(tcap<0x400)rc=MAKERESULT(Module_Libnx,LibnxError_BadInput); else rc=dump_read_exact(&tik,ticket_offset,ticket,0x400); fsFileClose(&tik); }
    }
    fsFsClose(&fs); if(rc)return rc;
    const u8 *t=ticket; u32 sig=((u32)t[0]<<24)|((u32)t[1]<<16)|((u32)t[2]<<8)|t[3];
    size_t body;
    switch(sig) { case 0x10000: case 0x10003: body=0x240;break; case 0x10001:case 0x10004:body=0x140;break; case 0x10002:case 0x10005:body=0x80;break; case 0x10006:body=0x40;break; default:return MAKERESULT(Module_Libnx,LibnxError_IncompatSysVer); }
    if(body+0x180>0x400 || memcmp(t+body+0x160,rights,16))return MAKERESULT(Module_Libnx,LibnxError_IncompatSysVer);
    /* Preserve optional v2 sections inside the 0x400-byte record. */
    u32 section_size; memcpy(&section_size,t+body+0x174,4);
    if(section_size>0x400-body-0x180)return MAKERESULT(Module_Libnx,LibnxError_BadInput);
    *tn=body+0x180+section_size;
    char issuer[65]; memcpy(issuer,t+body,64);issuer[64]=0;
    if(!memchr(issuer,0,64) || strncmp(issuer,"Root-",5))return MAKERESULT(Module_Libnx,LibnxError_BadInput);
    rc=dump_save(&fs,0x80000000000000e0ULL); if(rc)return rc;
    /* Issuer certificates: leaf first, then its CA. Root is a trust anchor. */
    char *name=strrchr(issuer,'-');
    while(!rc && name && name>issuer) {
        char path[FS_MAX_PATH]; snprintf(path,sizeof(path),"/certificate/%s",name+1);
        FsFile f; rc=fsFsOpenFile(&fs,path,FsOpenMode_Read,&f);
        if(!rc) { s64 n=0; rc=fsFileGetSize(&f,&n); if(!rc && (n<=0 || (u64)n>ccap-*cn))rc=MAKERESULT(Module_Libnx,LibnxError_BadInput); if(!rc) { rc=dump_read_exact(&f,0,(u8*)cert+*cn,n); if(!rc)*cn+=n; } fsFileClose(&f); }
        *name=0;name=strrchr(issuer,'-');
    }
    fsFsClose(&fs); return rc;
}

/* Heartbeat owns its descriptors exclusively: no SD, logging or ownership mutex. */
int sa_heartbeat_listen(void) {
    int fd=socket(AF_INET,SOCK_STREAM,0); if(fd<0)return -errno;
    int one=1; setsockopt(fd,SOL_SOCKET,SO_REUSEADDR,&one,sizeof(one));
    struct sockaddr_in a={.sin_family=AF_INET,.sin_port=htons(6001),.sin_addr={INADDR_ANY}};
    if(bind(fd,(void*)&a,sizeof(a))<0 || listen(fd,8)<0 || fcntl(fd,F_SETFL,O_NONBLOCK)<0) {
        int e=errno; close(fd); return -e;
    }
    return fd;
}
int sa_heartbeat_accept(int fd) {
    int c=accept(fd,NULL,NULL);
    if(c>=0 && fcntl(c,F_SETFL,O_NONBLOCK)<0) { close(c); return -1; }
    return c;
}
int sa_heartbeat_health(int fd) {
    int accepting=0,error=0; socklen_t n=sizeof(int);
    if(getsockopt(fd,SOL_SOCKET,SO_ACCEPTCONN,&accepting,&n)<0 || !accepting)return 0;
    n=sizeof(int); return getsockopt(fd,SOL_SOCKET,SO_ERROR,&error,&n)==0 && !error;
}

/* Administrative reboot audit bypasses the best-effort queue, including in NRO.
 * Append mode keeps the writer's existing descriptor valid. */
Result sa_reboot_log(void) {
    /* Same path translation and open mode as put (sa_open).
     * Diagnostic results: F001=translate/fs, F002=create, F003=open,
     * F004=get size, F005=write. Never delete the existing audit log. */
    FsFileSystem *fs; char p[FS_MAX_PATH];
    if(fsdevTranslatePath("sdmc:/switch/swagent.reboot.log",&fs,p) || !fs) return 0xF001;
    FsDirEntryType type;
    Result rc=fsFsGetEntryType(fs,p,&type);
    if(rc) {
        rc=fsFsCreateFile(fs,p,0,0);
        if(rc) return 0xF002;
    }
    FsFile f;
    rc=fsFsOpenFile(fs,p,FsOpenMode_Write,&f);
    if(rc) return 0xF003;
    s64 offset=0;
    rc=fsFileGetSize(&f,&offset);
    if(rc) { fsFileClose(&f); return 0xF004; }
    const char line[]="reboot: requested\n";
    rc=fsFileWrite(&f,offset,line,sizeof(line)-1,FsWriteOption_Flush);
    fsFileClose(&f);
    return rc ? 0xF005 : 0;
}
Result sa_reboot(void) {
    Result rc=bpcInitialize();
    if(R_FAILED(rc)) return rc;
    rc=bpcRebootSystem();
    bpcExit();
    return rc;
}
