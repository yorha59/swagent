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
static int stage, init_rc, reboot_rc, exits, fail_write, fail_flush;
static Result bpcInitialize(void){assert(stage==0);stage=1;return init_rc;}
static Result bpcRebootSystem(void){assert(stage==1);stage=2;return reboot_rc;}
static void bpcExit(void){assert(stage==2);exits++;}
static int fake_mkdir(const char *p,int mode){(void)p;(void)mode;return 0;}
static FILE *fake_open(const char *p,const char *m){assert(strstr(p,"swagent.log"));assert(!strcmp(m,"ab"));return (FILE*)1;}
static size_t fake_write(const void *p,size_t s,size_t n,FILE *f){(void)s;(void)f;assert(!memcmp(p,"reboot: requested\n",n));assert(stage==0);stage=1;return fail_write?0:n;}
static int fake_flush(FILE *f){(void)f;assert(stage==1);stage=2;return fail_flush?-1:0;}
static int fake_close(FILE *f){(void)f;assert(stage==2);stage=3;return 0;}
#define mkdir fake_mkdir
#define fopen fake_open
#define fwrite fake_write
#define fflush fake_flush
#define fclose fake_close
''' + function('Result sa_reboot(void)') + '\n' + function('Result sa_reboot_log(void)') + r'''
int main(void){
    assert(sa_reboot_log()==0 && stage==3);
    stage=0;fail_write=1;assert(sa_reboot_log()!=0 && stage==3);
    stage=0;fail_write=0;fail_flush=1;assert(sa_reboot_log()!=0 && stage==3);
    stage=0;init_rc=42;assert(sa_reboot()==42 && stage==1 && exits==0);
    stage=0;init_rc=0;reboot_rc=91;assert(sa_reboot()==91 && stage==2 && exits==1);
    stage=0;reboot_rc=0;assert(sa_reboot()==0 && exits==2);
    return 0;
}
''')

if __name__ == '__main__':
    unittest.main()
