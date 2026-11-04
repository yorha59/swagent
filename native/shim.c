/* ABI adapters only: policy, sequencing and command logic live in Rust. */
#include <switch.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <netdb.h>
#include <errno.h>
#include <unistd.h>
#include <stdlib.h>
#include <string.h>
#include <turbojpeg.h>

u32 __nx_applet_type = AppletType_None;
void __libnx_initheap(void) {
    static u8 heap[16 * 1024 * 1024] __attribute__((aligned(0x1000)));
    extern void *fake_heap_start, *fake_heap_end;
    fake_heap_start = heap; fake_heap_end = heap + sizeof(heap);
}
void __appInit(void) {}
void __appExit(void) {}
extern void agent_main(void);
int main(void) { agent_main(); for (;;) svcSleepThread(3600000000000LL); }
static Thread threads[2];
u32 sa_spawn(unsigned slot, void (*fn)(void*), void *arg) {
    Result rc = threadCreate(&threads[slot], fn, arg, NULL, 0x18000, 44, -2);
    if (!rc) { rc = threadStart(&threads[slot]); if (rc) threadClose(&threads[slot]); }
    return rc;
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
    SocketInitConfig cfg = *socketGetDefaultInitConfig();
    cfg.tcp_tx_buf_size = cfg.tcp_rx_buf_size = 0x10000;
    cfg.tcp_tx_buf_max_size = cfg.tcp_rx_buf_max_size = 0x10000;
    cfg.num_bsd_sessions = 3;
    return socketInitialize(&cfg);
}
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
    return fd;
}
int sa_accept(int fd) { return accept(fd,NULL,NULL); }
int sa_connect(const char *host, const char *port) {
    struct addrinfo hint = {.ai_family=AF_INET,.ai_socktype=SOCK_STREAM}, *list=NULL;
    if (getaddrinfo(host,port,&hint,&list)) return -1;
    int fd=-1;
    for (struct addrinfo *p=list;p;p=p->ai_next) {
        fd=socket(p->ai_family,p->ai_socktype,p->ai_protocol);
        if (fd<0) continue;
        sa_timeout(fd,10);
        if (!connect(fd,p->ai_addr,p->ai_addrlen)) break;
        close(fd); fd=-1;
    }
    freeaddrinfo(list); return fd;
}
/* File objects and all service objects are 16-byte libnx Service wrappers. */
_Static_assert(sizeof(FsFile)==16 && sizeof(NcmContentStorage)==16, "service ABI");
_Static_assert(sizeof(NcmContentMetaKey)==16 && sizeof(NcmContentInfo)==24, "NCM ABI");
u32 sa_open(const char *path, int write, FsFile *file) {
    FsFileSystem *fs; char p[FS_MAX_PATH];
    if (fsdevTranslatePath(path,&fs,p)) return MAKERESULT(Module_Libnx,LibnxError_BadInput);
    if (write) { fsFsDeleteFile(fs,p); Result rc=fsFsCreateFile(fs,p,0,0); if (rc) return rc; }
    return fsFsOpenFile(fs,p,write ? FsOpenMode_Write|FsOpenMode_Append : FsOpenMode_Read,file);
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
static HiddbgHdlsHandle pad;
static HiddbgHdlsState pad_state;
static u8 hid_work[0x1000] __attribute__((aligned(0x1000)));
static int hid_ready;
u32 sa_hid_init(void) {
    if(hid_ready) return 0;
    Result rc=hiddbgInitialize(); if(rc) return rc;
    HiddbgHdlsSessionId session;
    rc=hiddbgAttachHdlsWorkBuffer(&session,hid_work,sizeof(hid_work));
    if(rc) { hiddbgExit(); return rc; }
    HiddbgHdlsDeviceInfo info={.deviceType=HidDeviceType_FullKey3,.npadInterfaceType=HidNpadInterfaceType_Bluetooth,
        .singleColorBody=0xff000000,.singleColorButtons=0xffffffff};
    rc=hiddbgAttachHdlsVirtualDevice(&pad,&info);
    if(rc) { hiddbgReleaseHdlsWorkBuffer(session); hiddbgExit(); return rc; }
    pad_state.battery_level=4; hid_ready=1; return 0;
}
u32 sa_buttons(u64 buttons) { pad_state.buttons=buttons; return hiddbgSetHdlsState(pad,&pad_state); }
u32 sa_touch(u32 x,u32 y,u64 ns,int down) {
    HidTouchState t={.delta_time=ns,.x=x,.y=y,.diameter_x=10,.diameter_y=10};
    return hiddbgSetTouchScreenAutoPilotState(down?&t:NULL,down?1:0);
}
u32 sa_capture(void *jpeg,size_t size,u64 *out) {
    Result rc=capsscInitialize(); if(rc) return rc;
    rc=capsscCaptureJpegScreenShot(out,jpeg,size,ViLayerStack_Screenshot,1000000000LL);
    capsscExit(); return rc;
}
/* TurboJPEG gives a nonfatal error return, unlike libjpeg's default exit handler. */
void *sa_jpeg(void) { return tjInitDecompress(); }
int sa_jpeg_header(void *h,const u8 *p,size_t n,int *w,int *height) {
    int sub,colors; return tjDecompressHeader3(h,p,n,w,height,&sub,&colors);
}
int sa_jpeg_decode(void *h,const u8 *p,size_t n,u8 *out,int w,int height) {
    return tjDecompress2(h,p,n,out,w,0,height,TJPF_RGB,TJFLAG_FASTDCT);
}
void sa_jpeg_close(void *h) { tjDestroy(h); }
u32 sa_launch(u64 *pid) {
    Result rc=pmshellInitialize(); if(rc) return rc;
    NcmProgramLocation loc={.program_id=0x430000000000000B,.storageID=NcmStorageId_None};
    rc=pmshellLaunchProgram(0,&loc,pid); pmshellExit(); return rc;
}

/* Guard third-party libc/JPEG fatal fallbacks against process teardown. */
extern void agent_park(void);
void __wrap_exit(int status) { (void)status; agent_park(); __builtin_unreachable(); }
void __wrap__exit(int status) { (void)status; agent_park(); __builtin_unreachable(); }
void __wrap_abort(void) { agent_park(); __builtin_unreachable(); }
