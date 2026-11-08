/* Host-only libnx ABI doubles. Not compiled into the NSP. */
#include <sys/types.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <netdb.h>
#include <netinet/in.h>
#include <fcntl.h>
#include <unistd.h>
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <stdatomic.h>
typedef struct {uint32_t fd,rest[3];} Service;
static void path(const char *p,char *out) {
    if(p[0]=='/'){snprintf(out,1024,"%s",p);return;}
    const char *root=getenv("SWAGENT_TEST_ROOT");if(!root)root="build/host-sd";
    const char *colon=strchr(p,':');snprintf(out,1024,"%s%s",root,colon?colon+1:p);
}
uint32_t svcSleepThread(int64_t n){if(n!=5000000000LL)usleep(n/1000);return 0;}
static _Thread_local const char *receive_fixture;
static _Thread_local size_t receive_offset, receive_length;
static _Thread_local char fixture_request[2048];
const char *host_fixture_request(void){return fixture_request;}
static _Thread_local int receive_error, connect_failure, fixture_connects;
int host_fixture_connects(void){return fixture_connects;}
void host_connect_failure(int error){connect_failure=error;}
void host_receive_bytes(const void *p,size_t n,int error){receive_fixture=p;receive_length=n;receive_offset=0;fixture_connects=0;receive_error=error;}
void host_receive_fixture(const char *p,int error){host_receive_bytes(p,p?strlen(p):0,error);}
static _Thread_local uint64_t receive_step, receive_elapsed;
void host_clock_offset(uint64_t ms);
void host_receive_step(uint64_t ms){receive_step=ms;receive_elapsed=0;}
static _Thread_local const char *reconnect_fixture;
void host_reconnect_fixture(const char *p){reconnect_fixture=p;}
static _Thread_local size_t storm_at;
static _Thread_local int storm_left;
void host_receive_storm(size_t at,int count){storm_at=at;storm_left=count;}
ssize_t sa_recv(int fd,void *p,size_t n,int flags){
    if(!receive_fixture)return recv(fd,p,n,flags);
    if(storm_left && receive_offset>=storm_at){int errors[]={11,115,4,35,116};errno=errors[storm_left%5];storm_left--;if(receive_step){receive_elapsed+=receive_step;host_clock_offset(receive_elapsed);}return -1;}
    size_t left=receive_length-receive_offset;
    if(!left){errno=receive_error;return -1;}
    if(n>left)n=left;
    if(n>1 && receive_step){receive_elapsed+=receive_step;host_clock_offset(receive_elapsed);}
    memcpy(p,receive_fixture+receive_offset,n);receive_offset+=n;return n;
}
ssize_t sa_send(int fd,const void *p,size_t n,int flags){if(receive_fixture){size_t len=n<sizeof(fixture_request)-1?n:sizeof(fixture_request)-1;memcpy(fixture_request,p,len);fixture_request[len]=0;return n;}return send(fd,p,n,flags);}
int sa_errno(void){return errno;}
static atomic_int connect_blocked,connect_entered;
void host_connect_block(int block){atomic_store(&connect_blocked,block);if(block)atomic_store(&connect_entered,0);}
int host_connect_entered(void){return atomic_load(&connect_entered);}
extern int host_test_connect(uint16_t port);
int sa_connect(const char *host,const char *port){
    if(atomic_load(&connect_blocked)){atomic_store(&connect_entered,1);while(atomic_load(&connect_blocked))usleep(1000);errno=ECONNREFUSED;return -1;}
    if(connect_failure){errno=connect_failure;return -1;}
    if(receive_fixture){if(fixture_connects && reconnect_fixture){receive_fixture=reconnect_fixture;receive_length=strlen(receive_fixture);}fixture_connects++;receive_offset=0;return open("/dev/null",O_RDONLY);}
    if(!strcmp(host,"127.0.0.1")){int fd=host_test_connect((uint16_t)atoi(port));if(fd<0)errno=ECONNREFUSED;return fd;}
    struct addrinfo hints={.ai_family=AF_INET,.ai_socktype=SOCK_STREAM},*ai=NULL;
    if(getaddrinfo(host,port,&hints,&ai))return -1;
    int fd=socket(AF_INET,SOCK_STREAM,0);int rc=connect(fd,ai->ai_addr,ai->ai_addrlen);
    freeaddrinfo(ai);if(rc){close(fd);return -1;}return fd;
}
uint32_t sa_open(const char *p,int write,Service *s){char out[1024];path(p,out);int fd=open(out,write?(O_CREAT|O_RDWR|(write==1?O_TRUNC:0)):O_RDONLY,0600);if(fd<0)return errno;s->fd=fd;return 0;}
uint32_t sa_remove(const char *p){char out[1024];path(p,out);return unlink(out)?errno:0;}
void fsFileClose(Service *s){close(s->fd);}
uint32_t fsFileRead(Service *s,int64_t off,uint8_t *p,uint64_t n,uint32_t opt,uint64_t *out){(void)opt;ssize_t r=pread(s->fd,p,n,off);if(r<0)return errno;*out=r;return 0;}
static _Thread_local uint64_t install_write_step, install_write_elapsed;
void host_install_write_step(uint64_t ms){install_write_step=ms;install_write_elapsed=0;}
static void install_written(void){if(install_write_step){install_write_elapsed+=install_write_step;host_clock_offset(install_write_elapsed);}}
uint64_t host_install_write_elapsed(void){return install_write_elapsed;}
uint32_t fsFileWrite(Service *s,int64_t off,const uint8_t *p,uint64_t n,uint32_t opt){(void)opt;if(pwrite(s->fd,p,n,off)!=(ssize_t)n)return errno;install_written();return 0;}
uint32_t fsFileFlush(Service *s){return fsync(s->fd)?errno:0;}
uint32_t fsFileGetSize(Service *s,int64_t *n){struct stat st;if(fstat(s->fd,&st))return errno;*n=st.st_size;return 0;}
void fsFsClose(Service *s){(void)s;}
/* Test fixture CNMT NCA is plain bytes. Real NCA decryption belongs to FSP. */
uint32_t sa_stage_open(const char *name,int64_t size,Service *file,Service *fs){
    (void)name;(void)size;memset(fs,0,sizeof(*fs));
    return sa_open("sdmc:/switch/.host-stage.nca",1,file);
}
uint32_t sa_stage_commit(Service *fs){(void)fs;return 0;}
void sa_stage_close(Service *fs){(void)fs;sa_remove("sdmc:/switch/.host-stage.nca");}
uint32_t sa_cnmt_open(const char *p,Service *file,Service *fs){
    (void)fs;
    if(strncmp(p,"@SystemContent://swagent-install/",33))return 999;
    return sa_open("sdmc:/switch/.host-stage.nca",0,file);
}
static uint32_t fail_write,max_write,registered,deleted,tickets,ns_commands;
void host_reset(uint32_t fail){fail_write=fail;max_write=registered=deleted=tickets=ns_commands=0;}
uint32_t host_stat(uint32_t n){switch(n){case 0:return max_write;case 1:return registered;case 2:return deleted;case 3:return tickets;default:return ns_commands;}}
uint32_t sa_ticket(const void *t,size_t tn,const void *c,size_t cn){(void)c;(void)cn;if(tn<0x350||(((const uint8_t*)t)[0x140+0x146]&0x10))return 999;tickets++;return 0;}
uint32_t ncmInitialize(void){return 0;}void ncmExit(void){}
uint32_t ncmOpenContentStorage(Service *s,uint32_t id){s->fd=id;return 0;}
uint32_t ncmOpenContentMetaDatabase(Service *s,uint32_t id){s->fd=id;return 0;}
void ncmContentStorageClose(Service *s){(void)s;}void ncmContentMetaDatabaseClose(Service *s){(void)s;}
uint32_t ncmContentStorageHas(Service *s,_Bool *out,const void *id){(void)s;(void)id;*out=0;return 0;}
uint32_t ncmContentStorageGeneratePlaceHolderId(Service *s,void *id){(void)s;memset(id,1,16);return 0;}
uint32_t ncmContentStorageCreatePlaceHolder(Service *s,const void *id,const void *ph,int64_t size){(void)s;(void)id;(void)ph;(void)size;return 0;}
uint32_t ncmContentStorageWritePlaceHolder(Service *s,const void *ph,uint64_t off,const void *p,size_t n){(void)s;(void)ph;(void)off;(void)p;if(n>max_write)max_write=n;if(!fail_write)install_written();return fail_write;}
uint32_t ncmContentStorageRegister(Service *s,const void *id,const void *ph){(void)s;(void)id;(void)ph;registered++;return 0;}
uint32_t ncmContentStorageDeletePlaceHolder(Service *s,const void *ph){(void)s;(void)ph;deleted++;return 0;}
uint32_t ncmContentMetaDatabaseSet(Service *s,const void *key,const void *p,uint64_t n){(void)s;(void)key;if(n<8||((const uint8_t*)p)[2]!=2)return 999;return 0;}
uint32_t ncmContentMetaDatabaseCommit(Service *s){(void)s;return 0;}
uint32_t nsInitialize(void){return 0;}void nsExit(void){}
uint32_t nsCountApplicationContentMeta(uint64_t app,int32_t *out){(void)app;*out=0;return 0x410;}
uint32_t sa_ns(uint32_t cmd,const void *in,uint32_t n,void *p,size_t size,uint32_t *count){(void)in;(void)n;(void)p;(void)size;(void)count;ns_commands|=1u<<(cmd==16?0:cmd==17?1:2);return 0;}

uint32_t sa_rename(const char *a,const char *b){char from[1024],to[1024];path(a,from);path(b,to);return rename(from,to)?errno:0;}
static uint32_t launch_result, launch_calls;
static uint64_t launched_title;
uint64_t host_launched_title(void){return launched_title;}
uint32_t host_launch_calls(void){return launch_calls;}
void host_launch_result(uint32_t rc){launch_result=rc;launch_calls=0;launched_title=0;}
uint32_t sa_request_launch(uint64_t title){launched_title=title;launch_calls++;return launch_result;}
uint32_t sa_launch(uint64_t *pid){*pid=123;return launch_result;}
static uint32_t process_failure, terminated, info_exits;
void host_process_reset(uint32_t fail){process_failure=fail;terminated=info_exits=0;}
uint32_t host_process_stat(uint32_t n){return n?info_exits:terminated;}
uint32_t svcGetProcessId(uint64_t *pid,uint32_t handle){if(handle!=0xffff8001)return 999;*pid=456;return process_failure==1?77:0;}
uint32_t sa_terminate(uint64_t pid){terminated=(uint32_t)pid;return process_failure==2?88:0;}
uint32_t svcGetProcessList(int32_t *count,uint64_t *pids,uint32_t capacity){
    if(process_failure==3)return 99;
    *count=process_failure==6?1025:2;
    if(capacity>=2){pids[0]=123;pids[1]=456;}
    return 0;
}
uint32_t pminfoInitialize(void){return process_failure==4?100:0;}
void pminfoExit(void){info_exits++;}
uint32_t pminfoGetProgramId(uint64_t *program_id,uint64_t pid){
    if(process_failure==5 || (process_failure==7 && pid==123))return 527;
    *program_id=pid==456?0x430000000000000bULL:0x0100123400000000ULL;return 0;
}

uint32_t sa_sd_space(int64_t *total,int64_t *free_space){
    if(process_failure==8)return 102;
    *total=64000000000LL;*free_space=512500000LL;return 0;
}

static uint32_t reinit_failures;
void host_reinit_failures(uint32_t n){reinit_failures=n;}
uint32_t sa_net_reinit(void){if(reinit_failures){reinit_failures--;return 123;}return 0;}

typedef struct { uint32_t flags; int32_t handheld, console; } SleepSettings;
static SleepSettings host_sleep = {0xa5, 2, 3};
static uint32_t settings_fail;
void host_settings_fail(uint32_t fail) { settings_fail = fail; }
uint32_t setsysInitialize(void) { return settings_fail == 1 ? 91 : 0; }
void setsysExit(void) {}
uint32_t setsysGetSleepSettings(SleepSettings *out) { *out = host_sleep; return settings_fail == 2 ? 92 : 0; }
uint32_t setsysSetSleepSettings(const SleepSettings *in) {
    if (settings_fail == 3) return 93;
    if (in->flags != 0xa5) return 94;
    host_sleep = *in;
    if (settings_fail == 4) host_sleep.console = 5;
    return 0;
}
uint32_t sa_setting_read(uint32_t key, char *out, size_t capacity) {
    const char *values[] = {"23.0.0", "Switch\r\nname", "XAW000000", "87%", "42 C"};
    snprintf(out, capacity, "%s", values[key]); return 0;
}

static uint32_t power_fail, power_charger;
void host_power_fixture(uint32_t fail, uint32_t charger) { power_fail=fail; power_charger=charger; }
uint32_t sa_power_psm(uint32_t *percent, uint32_t *charger) {
    *percent=87; *charger=power_charger; return power_fail==2?92:0;
}
uint32_t sa_power_gauge(uint8_t *voltage, uint8_t *current) {
    voltage[0]=0x00; voltage[1]=0xb9; /* 3700mV */
    current[0]=0x00; current[1]=0xe7; /* -1000mA */
    return power_fail==1?91:0;
}

unsigned sa_mkdir(const char *p) { (void)p; return 1; }
unsigned sa_dir_open(const char *p, void *d) { (void)p; (void)d; return 1; }
unsigned sa_dir_read(void *d, char *n, void *s, void *t, void *c) { (void)d; (void)n; (void)s; (void)t; (void)c; return 1; }
void fsDirClose(void *d) { (void)d; }
uint32_t sa_wifi(uint32_t join, const uint8_t *profile, uint8_t *records, int32_t *count) {
    (void)join; (void)profile; (void)records; *count=0; return 123;
}

void sa_timeout(int fd,int seconds) {
    struct timeval t={seconds,0};
    setsockopt(fd,SOL_SOCKET,SO_RCVTIMEO,&t,sizeof(t));
    setsockopt(fd,SOL_SOCKET,SO_SNDTIMEO,&t,sizeof(t));
}

#include <time.h>
static atomic_uint_fast64_t clock_offset;
void host_clock_offset(uint64_t ms){atomic_store(&clock_offset,ms);}
uint64_t sa_now_ms(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC,&t); return (uint64_t)t.tv_sec*1000+t.tv_nsec/1000000+atomic_load(&clock_offset); }
static _Thread_local int mock_listen_errno;
void host_listen_failure(int error){mock_listen_errno=error;}
static _Thread_local int mock_listener, mock_accept=-1, mock_accept_errno;
void host_listener_fixture(int enabled,int client,int error) { mock_listener=enabled; mock_accept=client; mock_accept_errno=error; }
int sa_listen(void) { if(mock_listen_errno)return -mock_listen_errno; if(mock_listener)return open("/dev/null",O_RDONLY); int fd=socket(AF_INET,SOCK_STREAM,0),one=1; setsockopt(fd,SOL_SOCKET,SO_REUSEADDR,&one,sizeof(one)); struct sockaddr_in a={.sin_family=AF_INET,.sin_port=htons(6000),.sin_addr={htonl(INADDR_LOOPBACK)}}; if(bind(fd,(void*)&a,sizeof(a))||listen(fd,4)){int e=errno;close(fd);return -e;} fcntl(fd,F_SETFL,O_NONBLOCK);return fd; }
int sa_accept(int fd) { if(mock_listener) { if(mock_accept<0) { errno=mock_accept_errno;return -1; } int client=mock_accept;mock_accept=-1;fcntl(client,F_SETFL,O_NONBLOCK);return client; } return accept(fd,NULL,NULL); }

#include <pthread.h>
uint64_t sa_thread_id(void) { return (uint64_t)(uintptr_t)pthread_self(); }

int sa_nonblocking(int fd) { int flags=fcntl(fd,F_GETFL); return flags>=0 && (flags&O_NONBLOCK); }

const char *sa_version(void) { return "swagent-1.30-rs"; }
static _Thread_local int host_nro, host_quit;
void host_edition(int nro) { host_nro=nro; host_quit=0; }
int host_did_quit(void) { return host_quit; }
int sa_is_nro(void) { return host_nro; }
void sa_quit(void) { host_quit=1; }

void sa_persist(const uint8_t *p,size_t n){(void)p;(void)n;}
size_t sa_log_tail(uint8_t *p,size_t n){const char *s="previous boot evidence\n";size_t len=strlen(s);if(n<len)return 0;memcpy(p,s,len);return len;}
uint32_t sa_installed(int32_t offset,uint64_t *title,uint8_t *name,size_t capacity,int32_t *count){(void)capacity;*count=offset==0;if(*count){*title=0x430000000000000bULL;memcpy(name,"stale swagent",14);}return 0;}
uint32_t sa_installed_delete(uint64_t title){(void)title;return 0;}

static pthread_mutex_t jobs_mutex = PTHREAD_MUTEX_INITIALIZER;
static pthread_cond_t jobs_cond = PTHREAD_COND_INITIALIZER;
static unsigned jobs_count;
void sa_jobs_init(void) { pthread_mutex_lock(&jobs_mutex); jobs_count=0; pthread_mutex_unlock(&jobs_mutex); }
void sa_jobs_signal(void) { pthread_mutex_lock(&jobs_mutex); jobs_count++; pthread_cond_signal(&jobs_cond); pthread_mutex_unlock(&jobs_mutex); }
void sa_jobs_wait(void) { pthread_mutex_lock(&jobs_mutex); while(!jobs_count) pthread_cond_wait(&jobs_cond,&jobs_mutex); jobs_count--; pthread_mutex_unlock(&jobs_mutex); }

int sa_stopping(void) { return 0; }
void sa_listener_close(int fd) { close(fd); }
