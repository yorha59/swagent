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
const char *sa_version(void) { return "swagent-1.17-rs"; }
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
static Thread threads[6];
u32 sa_spawn(unsigned slot, void (*fn)(void*), void *arg) {
    if (slot >= 6) return MAKERESULT(Module_Libnx,LibnxError_BadInput);
    Result rc = threadCreate(&threads[slot], fn, arg, NULL, 0x18000, 44, -2);
    if (!rc) { rc = threadStart(&threads[slot]); if (rc) threadClose(&threads[slot]); }
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
    if (!sm_ok) { rc=smInitialize(); if(rc) return rc; sm_ok=true; }
    if (!hosversionGet() && !setsysInitialize()) {
        SetSysFirmwareVersion fw;
        if (!setsysGetFirmwareVersion(&fw)) hosversionSet(MAKEHOSVERSION(fw.major,fw.minor,fw.micro));
        setsysExit();
    }
    if (!fs_ok) { rc=fsInitialize(); if(rc) return rc; fs_ok=true; }
    if (!sd_ok) { rc=fsdevMountSdmc(); if(rc) return rc; sd_ok=true; }
    return sa_net_init();
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
    int fd = socket(AF_INET,SOCK_STREAM,0); if (fd < 0) return -errno;
    int one=1; setsockopt(fd,SOL_SOCKET,SO_REUSEADDR,&one,sizeof(one));
    struct sockaddr_in a = {.sin_family=AF_INET,.sin_port=htons(6000),.sin_addr={INADDR_ANY}};
    if (bind(fd,(void*)&a,sizeof(a)) < 0 || listen(fd,4) < 0) {
        int e=errno; close(fd); return -e;
    }
    /* Poll accept so worker connect failures can wake the recovery loop even
     * when no client is connected. Do not rely on accept honoring RCVTIMEO. */
    if (fcntl(fd,F_SETFL,O_NONBLOCK) < 0) { int e=errno; close(fd); return -e; }
    struct sockaddr_in bound;
    socklen_t bound_len = sizeof(bound);
    if (getsockname(fd, (struct sockaddr *)&bound, &bound_len) == 0) {
        printf("TCP listener bound %s:%u\n", inet_ntoa(bound.sin_addr), ntohs(bound.sin_port));
#ifdef SA_NRO
        struct in_addr local = {.s_addr = gethostid()};
        printf("Console IP: %s (listener accepts all interfaces)\n", inet_ntoa(local));
#endif
        fflush(stdout);
    }
    return fd;
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
    if (getaddrinfo(host,port,&hint,&list)) { errno=EHOSTUNREACH; return -1; }
    int saved=EHOSTUNREACH, fd=-1;
    u64 deadline=armGetSystemTick()+armNsToTicks(10000000000ULL);
    for (struct addrinfo *p=list;p && armGetSystemTick()<deadline;p=p->ai_next) {
        fd=socket(p->ai_family,p->ai_socktype,p->ai_protocol);
        if (fd<0) { saved=errno; continue; }
        if (fcntl(fd,F_SETFL,O_NONBLOCK)<0) { saved=errno; close(fd); fd=-1; break; }
        int result=connect(fd,p->ai_addr,p->ai_addrlen);
        saved=errno;
        if (result<0 && (saved==EINPROGRESS || saved==EALREADY || saved==EWOULDBLOCK)) {
            while (armGetSystemTick()<deadline) {
                struct pollfd wait={.fd=fd,.events=POLLOUT};
                int ready=poll(&wait,1,0);
                if (ready<0) { saved=errno; break; }
                if (ready>0) {
                    socklen_t len=sizeof(saved);
                    if (getsockopt(fd,SOL_SOCKET,SO_ERROR,&saved,&len)<0) saved=errno;
                    result=saved ? -1 : 0;
                    break;
                }
                svcSleepThread(10000000);
            }
            if (result<0 && armGetSystemTick()>=deadline) saved=ETIMEDOUT;
        }
        if (result==0) break;
        close(fd); fd=-1;
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

/* Experimental NIFM: private non-domain admin sessions, never libnx's global
 * nifm session (gethostid may have initialized it as User). All potentially
 * blocking IPC, including SM acquisition, runs on this single bounded lane.
 * A timed-out lane is quarantined, never killed/reused while IPC owns memory.
 * No serviceClose: raw owned-handle closure cannot issue another blocking IPC.
 * Step IDs: 1=open admin, 2=general, 3=enumerate, 4=set profile,
 * 5=create request, 6=state, 7=select UUID, 8=submit, 9=deadline, 10=thread, 11=events.
 */
#include <stdatomic.h>
extern void sa_wifi_log(u32 step, u32 rc);
static Thread wifi_thread;
static bool wifi_thread_live;
static atomic_bool wifi_abandoned;
static Service wifi_request;
static Mutex wifi_state_lock;
static void wifi_drop_request(void) {
    mutexLock(&wifi_state_lock);
    if (wifi_request.session) svcCloseHandle(wifi_request.session);
    memset(&wifi_request,0,sizeof(wifi_request));
    mutexUnlock(&wifi_state_lock);
}
static struct {
    u32 join, rc;
    u8 profile[0x17c], records[0x75 * 256];
    s32 count;
} wifi_job;
#define WIFI_TIMEOUT MAKERESULT(Module_Kernel, 117)
#define WIFI_BAD MAKERESULT(Module_Libnx, LibnxError_BadInput)
static void wifi_close(Service *s) {
    if (s->session) svcCloseHandle(s->session);
    memset(s,0,sizeof(*s));
}
static bool wifi_step(u32 step, Result rc) {
    sa_wifi_log(step,rc);
    if (!rc && atomic_load_explicit(&wifi_abandoned,memory_order_acquire)) rc=WIFI_TIMEOUT;
    wifi_job.rc=rc;
    return rc==0;
}
static void wifi_run(void *unused) {
    (void)unused;
    Service root={0}, general={0}, request={0};
    if (wifi_job.join) wifi_drop_request();
    if (!wifi_step(1,smGetService(&root,"nifm:a"))) goto out;
    u64 reserved=0;
    if (!wifi_step(2,serviceDispatchIn(&root,5,reserved,
        .in_send_pid=true,.out_num_objects=1,.out_objects=&general))) goto out;
    if (!wifi_job.join) {
        /* SfNetworkProfileBasicInfo is packed, stride 0x75. All profile types
         * (User|SsidList|Temporary); admin is required for the latter two. */
        u8 types=7;
        if (!wifi_step(3,serviceDispatchInOut(&general,7,types,wifi_job.count,
            .buffer_attrs={SfBufferAttr_Out|SfBufferAttr_HipcMapAlias},
            .buffers={{wifi_job.records,sizeof(wifi_job.records)}}))) goto out;
        if (wifi_job.count<0 || wifi_job.count>256) wifi_job.rc=WIFI_BAD;
        goto out;
    }
    Uuid uuid={0};
    if (!wifi_step(4,serviceDispatchOut(&general,9,uuid,
        .buffer_attrs={SfBufferAttr_In|SfBufferAttr_HipcPointer|SfBufferAttr_FixedSize},
        .buffers={{wifi_job.profile,sizeof(wifi_job.profile)}}))) goto out;
    s32 preset=2;
    if (!wifi_step(5,serviceDispatchIn(&general,4,preset,
        .out_num_objects=1,.out_objects=&request))) goto out;
    Handle events[2]={0};
    Result event_rc=serviceDispatch(&request,2,
        .out_handle_attrs={SfOutHandleAttr_HipcCopy,SfOutHandleAttr_HipcCopy},
        .out_handles=events);
    for (unsigned i=0;i<2;i++) if (events[i]) svcCloseHandle(events[i]);
    if (!wifi_step(11,event_rc)) goto out;
    u32 state=0;
    if (!wifi_step(6,serviceDispatchOut(&request,0,state))) goto out;
    if (state!=1) { wifi_step(6,WIFI_BAD); goto out; }
    if (!wifi_step(7,serviceDispatchIn(&request,9,uuid))) goto out;
    if (!wifi_step(8,serviceDispatch(&request,4))) goto out;
    /* Retain the request for the connection lifetime; submission isn't DHCP
     * or association success. Never invoke an applet or SubmitAndWait. */
    mutexLock(&wifi_state_lock);
    if (!atomic_load_explicit(&wifi_abandoned,memory_order_acquire)) {
        wifi_request=request;
        memset(&request,0,sizeof(request));
    }
    mutexUnlock(&wifi_state_lock);
out:
    memset(wifi_job.profile,0,sizeof(wifi_job.profile));
    wifi_close(&request);
    wifi_close(&general);
    wifi_close(&root);
    if (wifi_job.rc || atomic_load_explicit(&wifi_abandoned,memory_order_acquire))
        wifi_drop_request();
}
u32 sa_wifi(u32 join,const u8 *profile,u8 *records,s32 *count) {
    /* Worker is the sole caller. Poll only; a hung old lane costs one thread,
     * never a second thread or a reusable caller-owned output buffer. */
    if (wifi_thread_live) {
        Result rc=waitSingleHandle(wifi_thread.handle,0);
        if (rc) { sa_wifi_log(9,WIFI_TIMEOUT); return WIFI_TIMEOUT; }
        threadClose(&wifi_thread);
        wifi_thread_live=false;
    }
    memset(&wifi_job,0,sizeof(wifi_job));
    wifi_job.join=join;
    memcpy(wifi_job.profile,profile,sizeof(wifi_job.profile));
    atomic_store_explicit(&wifi_abandoned,false,memory_order_release);
    Result rc=threadCreate(&wifi_thread,wifi_run,NULL,NULL,0x8000,44,-2);
    if (rc) { sa_wifi_log(10,rc); return rc; }
    rc=threadStart(&wifi_thread);
    if (rc) { threadClose(&wifi_thread); sa_wifi_log(10,rc); return rc; }
    wifi_thread_live=true;
    rc=waitSingleHandle(wifi_thread.handle,5000000000LL);
    if (rc) {
        mutexLock(&wifi_state_lock);
        atomic_store_explicit(&wifi_abandoned,true,memory_order_release);
        if (wifi_request.session) svcCloseHandle(wifi_request.session);
        memset(&wifi_request,0,sizeof(wifi_request));
        mutexUnlock(&wifi_state_lock);
        sa_wifi_log(9,rc);
        return rc;
    }
    threadClose(&wifi_thread);
    wifi_thread_live=false;
    rc=wifi_job.rc;
    if (!rc) {
        *count=wifi_job.count;
        if (!join) memcpy(records,wifi_job.records,sizeof(wifi_job.records));
    }
    return rc;
}

_Static_assert(sizeof(NifmSfNetworkProfileData)==0x17c, "NIFM profile wire size");
_Static_assert(__builtin_offsetof(NifmSfNetworkProfileData,wireless_setting_data)==0x116, "NIFM wireless offset");
_Static_assert(__builtin_offsetof(NifmSfWirelessSettingData,passphrase)==0x24, "NIFM shared key offset");

u64 sa_now_ms(void) { return armTicksToNs(armGetSystemTick()) / 1000000; }

u64 sa_thread_id(void) { u64 id=0; svcGetThreadId(&id,CUR_THREAD_HANDLE); return id; }

int sa_nonblocking(int fd) { int flags=fcntl(fd,F_GETFL); return flags>=0 && (flags&O_NONBLOCK); }
