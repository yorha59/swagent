// Remote endpoint failures must not request BSD teardown: the listener retries
// its own descriptors independently. This gate is shared during normal calls;
// only explicit BSD service recovery is exclusive. Epochs prevent stale
// download descriptors from closing/reusing descriptors in a new BSD instance.
use crate::{ffi, log, util::*};
use core::{
    fmt::Write,
    sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::*},
};
// Socket calls may overlap (ping must work during a blocked download).
// Only BSD teardown excludes calls, and waits for in-flight calls to return.
pub struct Gate {
    resetting: AtomicBool,
    active: AtomicU32,
}
pub static GATE: Gate = Gate {
    resetting: AtomicBool::new(false),
    active: AtomicU32::new(0),
};
impl Gate {
    pub fn with<R>(&self, f: impl FnOnce(&mut ()) -> R) -> R {
        loop {
            while self.resetting.load(SeqCst) {
                sleep(1);
            }
            self.active.fetch_add(1, SeqCst);
            if !self.resetting.load(SeqCst) {
                break;
            }
            self.active.fetch_sub(1, SeqCst);
        }
        let result = f(&mut ());
        self.active.fetch_sub(1, SeqCst);
        result
    }
    fn exclusive<R>(&self, f: impl FnOnce() -> R) -> R {
        while self
            .resetting
            .compare_exchange(false, true, SeqCst, SeqCst)
            .is_err()
        {
            sleep(1);
        }
        while self.active.load(SeqCst) != 0 {
            sleep(1);
        }
        let result = f();
        self.resetting.store(false, SeqCst);
        result
    }
}
static EPOCH: AtomicU64 = AtomicU64::new(0);
static READY: AtomicBool = AtomicBool::new(true);
pub fn ready() -> bool {
    READY.load(Acquire)
}
static RESET: AtomicBool = AtomicBool::new(false);
pub fn epoch() -> u64 {
    EPOCH.load(Acquire)
}
pub fn transient(rc: i32) -> bool {
    matches!(rc, 4 | 11 | 35 | 116)
}
pub fn failed(rc: i32) {
    if !transient(rc) {
        let mut line = Text::<64>::new();
        let _ = write!(line, "net: socket rc={rc}");
        log::add(line.str());
        RESET.store(true, Release);
    }
}
pub fn pending() -> bool {
    RESET.load(Acquire)
}
pub fn reinit() -> u32 {
    GATE.exclusive(|| {
        READY.store(false, Release);
        EPOCH.fetch_add(1, AcqRel);
        RESET.store(false, Release);
        let rc = unsafe { ffi::sa_net_reinit() };
        READY.store(rc == 0, Release);
        let mut line = Text::<64>::new();
        let _ = write!(line, "net: reinit rc={rc}");
        log::add(line.str());
        rc
    })
}

// Do not call BSD again after a failed initialize. Retry the service session
// itself forever, allowing resume/service restart to complete in the meantime.
pub fn recover() {
    let mut delay = 1000;
    loop {
        if reinit() == 0 {
            return;
        }
        sleep(delay);
        delay = (delay * 2).min(30000);
    }
}
