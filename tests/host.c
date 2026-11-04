/* Host-only libnx ABI doubles. Not compiled into the NSP. */
#include <sys/types.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <netdb.h>
#include <fcntl.h>
#include <unistd.h>
#include <stdint.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <errno.h>
typedef struct {uint32_t fd,rest[3];} Service;
static void path(const char *p,char *out) {
    const char *root=getenv("SWAGENT_TEST_ROOT");if(!root)root="build/host-sd";
    const char *colon=strchr(p,':');snprintf(out,1024,"%s%s",root,colon?colon+1:p);
}
uint32_t svcSleepThread(int64_t n){usleep(n/1000);return 0;}
int sa_errno(void){return errno;}
int sa_connect(const char *host,const char *port){
    struct addrinfo hints={.ai_family=AF_INET,.ai_socktype=SOCK_STREAM},*ai=NULL;
    if(getaddrinfo(host,port,&hints,&ai))return -1;
    int fd=socket(AF_INET,SOCK_STREAM,0);int rc=connect(fd,ai->ai_addr,ai->ai_addrlen);
    freeaddrinfo(ai);if(rc){close(fd);return -1;}return fd;
}
uint32_t sa_open(const char *p,int write,Service *s){char out[1024];path(p,out);int fd=open(out,write?O_CREAT|O_TRUNC|O_RDWR:O_RDONLY,0600);if(fd<0)return errno;s->fd=fd;return 0;}
uint32_t sa_remove(const char *p){char out[1024];path(p,out);return unlink(out)?errno:0;}
void fsFileClose(Service *s){close(s->fd);}
uint32_t fsFileRead(Service *s,int64_t off,uint8_t *p,uint64_t n,uint32_t opt,uint64_t *out){(void)opt;ssize_t r=pread(s->fd,p,n,off);if(r<0)return errno;*out=r;return 0;}
uint32_t fsFileWrite(Service *s,int64_t off,const uint8_t *p,uint64_t n,uint32_t opt){(void)opt;return pwrite(s->fd,p,n,off)==(ssize_t)n?0:errno;}
uint32_t fsFileFlush(Service *s){return fsync(s->fd)?errno:0;}
uint32_t fsFileGetSize(Service *s,int64_t *n){struct stat st;if(fstat(s->fd,&st))return errno;*n=st.st_size;return 0;}
void fsFsClose(Service *s){(void)s;}
/* Test fixture CNMT NCA is plain bytes. Real NCA decryption belongs to FSP. */
uint32_t sa_cnmt_open(const char *p,Service *file,Service *fs){(void)fs;return sa_open(p,0,file);}
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
uint32_t ncmContentStorageWritePlaceHolder(Service *s,const void *ph,uint64_t off,const void *p,size_t n){(void)s;(void)ph;(void)off;(void)p;if(n>max_write)max_write=n;return fail_write;}
uint32_t ncmContentStorageRegister(Service *s,const void *id,const void *ph){(void)s;(void)id;(void)ph;registered++;return 0;}
uint32_t ncmContentStorageDeletePlaceHolder(Service *s,const void *ph){(void)s;(void)ph;deleted++;return 0;}
uint32_t ncmContentMetaDatabaseSet(Service *s,const void *key,const void *p,uint64_t n){(void)s;(void)key;if(n<8||((const uint8_t*)p)[2]!=2)return 999;return 0;}
uint32_t ncmContentMetaDatabaseCommit(Service *s){(void)s;return 0;}
uint32_t nsInitialize(void){return 0;}void nsExit(void){}
uint32_t nsCountApplicationContentMeta(uint64_t app,int32_t *out){(void)app;*out=0;return 0x410;}
uint32_t sa_ns(uint32_t cmd,const void *in,uint32_t n,void *p,size_t size,uint32_t *count){(void)in;(void)n;(void)p;(void)size;(void)count;ns_commands|=1u<<(cmd==16?0:cmd==17?1:2);return 0;}

uint32_t sa_rename(const char *a,const char *b){char from[1024],to[1024];path(a,from);path(b,to);return rename(from,to)?errno:0;}
static uint32_t launch_result;
void host_launch_result(uint32_t rc){launch_result=rc;}
uint32_t sa_launch(uint64_t *pid){*pid=123;return launch_result;}
