"""Execute the production bpc shim and synchronous audit path with host mocks."""
import unittest
from test_shutdown import function
import test_recovery

class RebootTest(unittest.TestCase):
    run_c = test_recovery.RecoveryTest.run_c
    def test_bpc_and_audit(self):
        self.run_c(r'''
#include <assert.h>
#include <stdbool.h>
#include <stdio.h>
#include <string.h>
#include <errno.h>
typedef unsigned Result;
#define R_FAILED(rc) ((rc)!=0)
static int stage, init_rc, reboot_rc, exits;
static Result bpcInitialize(void){assert(stage==0);stage=1;return init_rc;}
static Result bpcRebootSystem(void){assert(stage==1);stage=2;return reboot_rc;}
static void bpcExit(void){assert(stage==2);exits++;}
typedef long long s64;
typedef struct { int unused; } FsFileSystem;
typedef struct { int unused; } FsFile;
static FsFileSystem sd;
static int no_sd, missing, create_rc, creates, open_rc, size_rc, write_rc, closes;
#define Module_Libnx 1
#define LibnxError_NotFound 2
#define MAKERESULT(m,d) (((m)<<9)|(d))
#define FsOpenMode_Write 2
#define FsWriteOption_Flush 1
#define FS_MAX_PATH 769
typedef int FsDirEntryType;
static int fsdevTranslatePath(const char *p,FsFileSystem **fs,char *out){assert(!strcmp(p,"sdmc:/switch/swagent.reboot.log"));*fs=&sd;strcpy(out,"/switch/swagent.reboot.log");return no_sd;}
static Result fsFsGetEntryType(FsFileSystem *fs,const char *p,FsDirEntryType *type){assert(fs==&sd && !strcmp(p,"/switch/swagent.reboot.log"));*type=1;return missing;}
static Result fsFsCreateFile(FsFileSystem *fs,const char *p,s64 size,int flags){assert(fs==&sd && !strcmp(p,"/switch/swagent.reboot.log") && size==0 && flags==0);creates++;return create_rc;}
/* Append explicitly at the size returned below; Switch FS requires Write-only. */
static Result fsFsOpenFile(FsFileSystem *fs,const char *p,int mode,FsFile *f){(void)f;assert(fs==&sd && !strcmp(p,"/switch/swagent.reboot.log") && mode==FsOpenMode_Write);return open_rc;}
static Result fsFileGetSize(FsFile *f,s64 *size){(void)f;*size=123;return size_rc;}
static Result fsFileWrite(FsFile *f,s64 offset,const void *p,size_t n,int flags){(void)f;assert(offset==123 && n==18 && !memcmp(p,"reboot: requested\n",n) && flags==FsWriteOption_Flush);assert(stage==0);stage=1;return write_rc;}
static void fsFileClose(FsFile *f){(void)f;closes++;}
''' + function('Result sa_reboot(void)') + '\n' + function('Result sa_reboot_log(void)') + r'''
int main(void){
    assert(sa_reboot_log()==0 && stage==1 && closes==1);
    stage=0;write_rc=73;assert(sa_reboot_log()==73 && closes==2);
    stage=0;write_rc=0;size_rc=74;assert(sa_reboot_log()==0xF004 && stage==0 && closes==3);
    size_rc=0;open_rc=75;assert(sa_reboot_log()==0xF003 && stage==0 && closes==3);
    open_rc=0;no_sd=1;assert(sa_reboot_log()==0xF001 && closes==3);
    no_sd=0;missing=1;create_rc=76;assert(sa_reboot_log()==0xF002 && creates==1 && closes==3);
    create_rc=0;assert(sa_reboot_log()==0 && creates==2 && closes==4);
    stage=0;init_rc=42;assert(sa_reboot()==42 && stage==1 && exits==0);
    stage=0;init_rc=0;reboot_rc=91;assert(sa_reboot()==91 && stage==2 && exits==1);
    stage=0;reboot_rc=0;assert(sa_reboot()==0 && exits==2);
    return 0;
}
''')

if __name__ == '__main__':
    unittest.main()
