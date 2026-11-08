// One long-command lane owns this watchdog. A stuck IPC keeps its resources
// and thread; it is never killed while the kernel may own borrowed memory.
use crate::{ffi, util::*};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering::*};
static OWNER: AtomicU64 = AtomicU64::new(0);
static DEADLINE: AtomicU64 = AtomicU64::new(0);
static EXPIRED: AtomicBool = AtomicBool::new(false);
pub fn begin() {
    EXPIRED.store(false, Release);
    OWNER.store(unsafe { ffi::sa_thread_id() }, Release);
}
pub fn cancel() {
    EXPIRED.store(true, Release);
}
pub fn end() {
    DEADLINE.store(0, Release);
    OWNER.store(0, Release);
}
fn owned() -> bool {
    OWNER.load(Acquire) == unsafe { ffi::sa_thread_id() }
}
pub fn check() -> Result {
    if owned() && EXPIRED.load(Acquire) {
        Err(Error("operation wall-clock budget", 110))
    } else {
        Ok(())
    }
}
pub fn expired() -> bool {
    let d = DEADLINE.load(Acquire);
    if d != 0 && now_ms() >= d {
        EXPIRED.store(true, Release);
    }
    EXPIRED.load(Acquire)
}
pub struct Operation(bool);
impl Operation {
    pub fn new() -> Self {
        let ours = owned();
        if ours {
            DEADLINE.store(now_ms() + 30_000, Release);
        }
        Self(ours)
    }
}
impl Drop for Operation {
    fn drop(&mut self) {
        if self.0 {
            expired();
            DEADLINE.store(0, Release);
        }
    }
}
