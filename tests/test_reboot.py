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
static int open_error, write_error, flush_error, close_error, closes, flushes;
static FILE *fake_open(const char *p,const char *mode){
    assert(!strcmp(p,"sdmc:/switch/swagent.reboot.log") && !strcmp(mode,"ab"));
    if(open_error){errno=open_error>0 ? open_error : 0;return NULL;}
    return (FILE*)1;
}
static size_t fake_write(const void *p,size_t size,size_t n,FILE *f){
    assert(f==(FILE*)1 && size==1 && n==18 && !memcmp(p,"reboot: requested\n",n));
    if(write_error){errno=write_error>0 ? write_error : 0;return n-1;}
    return n;
}
static int fake_flush(FILE *f){assert(f==(FILE*)1);flushes++;errno=flush_error>0 ? flush_error : 0;return flush_error ? EOF : 0;}
static int fake_close(FILE *f){assert(f==(FILE*)1);closes++;errno=close_error>0 ? close_error : 0;return close_error ? EOF : 0;}
#define fopen fake_open
#define fwrite fake_write
#define fflush fake_flush
#define fclose fake_close
''' + function('Result sa_reboot(void)') + '\n' + function('Result sa_reboot_log(void)') + r'''
int main(void){
    errno=99;assert(sa_reboot_log()==0 && flushes==1 && closes==1);
    open_error=EIO;assert(sa_reboot_log()==EIO && closes==1);
    open_error=-1;assert(sa_reboot_log()==1 && closes==1);open_error=0;
    write_error=ENOSPC;flush_error=EIO;close_error=EBADF;
    assert(sa_reboot_log()==ENOSPC && flushes==2 && closes==2);
    write_error=-1;assert(sa_reboot_log()==1 && closes==3);write_error=0;
    assert(sa_reboot_log()==EIO && closes==4);
    flush_error=-1;assert(sa_reboot_log()==1 && closes==5);flush_error=0;
    assert(sa_reboot_log()==EBADF && closes==6);
    close_error=-1;assert(sa_reboot_log()==1 && closes==7);close_error=0;
    assert(sa_reboot_log()==0 && flushes==8 && closes==8);
    stage=0;init_rc=42;assert(sa_reboot()==42 && stage==1 && exits==0);
    stage=0;init_rc=0;reboot_rc=91;assert(sa_reboot()==91 && stage==2 && exits==1);
    stage=0;reboot_rc=0;assert(sa_reboot()==0 && exits==2);
    return 0;
}
''')

if __name__ == '__main__':
    unittest.main()
