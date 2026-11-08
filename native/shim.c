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
const char *sa_version(void) { return "swagent-1.19-rs"; }
int sa_is_nro(void) {
#ifdef SA_NRO
    return 1;
#else
    return 0;
#endif
}
#ifdef SA_NRO
#include <stdatomic.h>
static atomic_bool nro_quit;
void sa_quit(void) { atomic_store(&nro_quit, true); }
#else
void sa_quit(void) {}
#endif
void __libnx_initheap(void) {
    static u8 heap[16 * 1024 * 1024] __attribute__((aligned(0x1000)));
    extern void *fake_heap_start, *fake_heap_end;
    fake_heap_start = heap; fake_heap_end = heap + sizeof(heap);
}
void __appInit(void) {
#ifdef SA_NRO
    smInitialize();
    appletInitialize();
    hidInitialize();
#endif
}
void __appExit(void) {}
extern void agent_main(void);
#ifdef SA_NRO
static void nro_agent(void *arg) { (void)arg; agent_main(); }
int main(void) {
    consoleInit(NULL);
    printf("swagent-nro %s\n", sa_version());
    printf("PLUS or TCP quit: exit to hbmenu\n");
    PadState pad;
    padConfigureInput(1, HidNpadStyleSet_NpadStandard);
    padInitializeDefault(&pad);
    Thread agent;
    Result rc = threadCreate(&agent, nro_agent, NULL, NULL, 0x18000, 44, -2);
    if (!rc) rc = threadStart(&agent);
    if (rc) printf("agent startup failed: 0x%x\n", rc);
    while (appletMainLoop() && !atomic_load(&nro_quit)) {
        padUpdate(&pad);
        if (padGetButtonsDown(&pad) & HidNpadButton_Plus) break;
        consoleUpdate(NULL);
        svcSleepThread(16000000);
    }
    /* All service/socket/thread resources belong to this process. Terminate
     * together, including a worker potentially blocked in service IPC. */
    svcExitProcess();
    return 0;
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
u64 sa_now_ms(void);
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
    for(;;) {
        char line[768]={0};
        if(!atomic_flag_test_and_set(&log_lock)) {
            if(log_count) { memcpy(line,log_queue[log_head],sizeof(line)); log_head=(log_head+1)%128; log_count--; }
            atomic_flag_clear(&log_lock);
        }
        if(!line[0]) { svcSleepThread(10000000); continue; }
        FILE *f=fopen("sdmc:/switch/swagent.log","ab+");
        if(!f) continue;
        fseek(f,0,SEEK_END); long size=ftell(f);
        if(size>256*1024) {
            fclose(f); f=fopen("sdmc:/switch/swagent.log","wb");
            if(!f)continue;
            if(!atomic_flag_test_and_set(&log_lock)) { log_tail_len=0; atomic_flag_clear(&log_lock); }
        }
        size_t n=strlen(line);
        if(fwrite(line,1,n,f)==n && fflush(f)==0) log_cache(line,n);
        fclose(f);
    }
}
static void sa_trace(const char *step,int rc) {
    char line[192]; snprintf(line,sizeof(line),"boot uptime_ms=%llu step=%s %s rc=%d",(unsigned long long)sa_now_ms(),step,rc?"fail":"ok",rc);
    sa_persist(line,strlen(line));
}
static Thread threads[6];
u32 sa_spawn(unsigned slot, void (*fn)(void*), void *arg) {
    if (slot >= 6) return MAKERESULT(Module_Libnx,LibnxError_BadInput);
    sa_trace("thread-spawn-start",0);
    Result rc = threadCreate(&threads[slot], fn, arg, NULL, 0x18000, 44, -2);
    if (!rc) { rc = threadStart(&threads[slot]); if (rc) threadClose(&threads[slot]); }
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
u32 sa_net_reinit(void) {
    if (socket_ok) socketExit();
    socket_ok = false;
    return sa_net_init();
}
u32 sa_init(void) {
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
        if(!lr)lr=threadStart(&log_thread);
        sa_trace("log-thread",lr);
    }
    sa_trace("bsd-init-start",0); rc=sa_net_init(); sa_trace("bsd-init",rc); return rc;
}
ssize_t sa_send(int fd, const void *p, size_t n, int flags) { return send(fd,p,n,flags); }
ssize_t sa_recv(int fd, void *p, size_t n, int flags) { return recv(fd,p,n,flags); }
int sa_errno(void) { return errno; }
void sa_timeout(int fd, int seconds) {
    struct timeval t = {seconds,0};
    setsockopt(fd,SOL_SOCKET,SO_RCVTIMEO,&t,sizeof(t));
    setsockopt(fd,SOL_SOCKET,SO_SNDTIMEO,&t,sizeof(t));
}
int sa_listen(void) {
    sa_trace("listener-bind-start",0);
    int fd = socket(AF_INET,SOCK_STREAM,0); if (fd < 0) return -errno;
    int one=1; setsockopt(fd,SOL_SOCKET,SO_REUSEADDR,&one,sizeof(one));
    struct sockaddr_in a = {.sin_family=AF_INET,.sin_port=htons(6000),.sin_addr={INADDR_ANY}};
    if (bind(fd,(void*)&a,sizeof(a)) < 0 || listen(fd,4) < 0) {
        int e=errno; sa_trace("listener-bind",-e); close(fd); return -e;
    }
    /* Poll accept so worker connect failures can wake the recovery loop even
     * when no client is connected. Do not rely on accept honoring RCVTIMEO. */
    if (fcntl(fd,F_SETFL,O_NONBLOCK) < 0) { int e=errno; close(fd); return -e; }
    struct sockaddr_in bound;
    socklen_t bound_len = sizeof(bound);
    if (getsockname(fd, (struct sockaddr *)&bound, &bound_len) == 0) {
        printf("TCP listener bound %s:%u\n", inet_ntoa(bound.sin_addr), ntohs(bound.sin_port));
        fflush(stdout);
    }
    sa_trace("listener-bind",0); return fd;
}
int sa_accept(int fd) {
    int client=accept(fd,NULL,NULL);
    if (client>=0 && fcntl(client,F_SETFL,O_NONBLOCK)<0) {
        int e=errno; close(client); errno=e; return -1;
    }
    return client;
}
int sa_connect(const char *host, const char *port) {
    struct addrinfo hint = {.ai_family=AF_INET,.ai_socktype=SOCK_STREAM}, *list=NULL;
    if (getaddrinfo(host,port,&hint,&list)) { sa_persist("connect DNS fail rc=-1 errno=113",strlen("connect DNS fail rc=-1 errno=113")); errno=EHOSTUNREACH; return -1; }
    int saved=EHOSTUNREACH; unsigned attempt=0;
    int fd=-1;
    for (struct addrinfo *p=list;p;p=p->ai_next) {
        char trace[256], ip[INET_ADDRSTRLEN];
        struct sockaddr_in *dest=(struct sockaddr_in*)p->ai_addr;
        inet_ntop(AF_INET,&dest->sin_addr,ip,sizeof(ip));
        snprintf(trace,sizeof(trace),"connect retry attempt=%u destination=%s:%u",++attempt,ip,ntohs(dest->sin_port)); sa_persist(trace,strlen(trace));
        fd=socket(p->ai_family,p->ai_socktype,p->ai_protocol);
        int socket_errno=fd<0?errno:0;
        snprintf(trace,sizeof(trace),"socket create rc=%d errno=%d",fd,socket_errno); sa_persist(trace,strlen(trace));
        if (fd<0) { saved=errno; continue; }
        sa_timeout(fd,10);
        snprintf(trace,sizeof(trace),"connect start destination=%s:%u",ip,ntohs(dest->sin_port)); sa_persist(trace,strlen(trace));
        int cr=connect(fd,p->ai_addr,p->ai_addrlen); int ce=cr?errno:0;
        snprintf(trace,sizeof(trace),"connect rc=%d errno=%d destination=%s:%u",cr,ce,ip,ntohs(dest->sin_port)); sa_persist(trace,strlen(trace));
        if (!cr) break;
        saved=ce; close(fd); fd=-1;
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

/* nsInitialize selects ns:am2 on modern firmware; AM provides records and
 * deletion without an applet proxy, including records lacking control data. */
u32 sa_installed(s32 offset,u64 *title,char *name,size_t capacity,s32 *count) {
    sa_persist("installed: ns init start",24);
    Result rc=nsInitialize(); if(rc)return rc;
    NsApplicationRecord record={0};
    rc=nsListApplicationRecord(&record,1,offset,count);
    if(!rc && *count) {
        *title=record.application_id;
        name[0]=0;
        NsApplicationControlData *data=calloc(1,sizeof(*data));
        if(data) {
            u64 actual=0;
            if(!nsGetApplicationControlData(NsApplicationControlSource_Storage,*title,data,sizeof(*data),&actual)) {
                for(unsigned i=0;i<16;i++) if(data->nacp.lang[i].name[0]) {
                    snprintf(name,capacity,"%.*s",(int)sizeof(data->nacp.lang[i].name),data->nacp.lang[i].name); break;
                }
            }
            free(data);
        }
    }
    nsExit(); return rc;
}
u32 sa_installed_delete(u64 title) {
    Result rc=nsInitialize(); if(rc)return rc;
    rc=nsDeleteApplicationCompletely(title); nsExit(); return rc;
}
