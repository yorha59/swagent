// One long-command lane owns this watchdog. A stuck IPC keeps its resources
// and thread; it is never killed while the kernel may own borrowed memory.
use crate::{ffi, util::*};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering::*};
static OWNER: AtomicU64 = AtomicU64::new(0);
static DEADLINE: AtomicU64 = AtomicU64::new(0);
pub const GRACE_MS: u64 = 30_000;
static LAST_LOG: AtomicU64 = AtomicU64::new(0);
static EXPIRED: AtomicBool = AtomicBool::new(false);
pub fn begin() {
    EXPIRED.store(false, Release);
    OWNER.store(unsafe { ffi::sa_thread_id() }, Release);
    let now = now_ms();
    DEADLINE.store(now + GRACE_MS, Release);
    LAST_LOG.store(now, Release);
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
    if crate::stopping() { return Err(Error("shutdown", 125)); }
    if owned() && expired() {
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
// Only completed work renews the watchdog; retries and call entry do not.
pub fn progress() -> Result {
    if !owned() { return Ok(()); }
    check()?;
    let now = now_ms();
    DEADLINE.store(now + GRACE_MS, Release);
    let last = LAST_LOG.load(Acquire);
    if now.saturating_sub(last) >= 60_000 {
        LAST_LOG.store(now, Release);
        use core::fmt::Write;
        let mut line = Text::<128>::new();
        let _ = write!(line, "debug: budget extended uptime_ms={} deadline_ms={}", now, now + GRACE_MS);
        crate::log::add(line.str());
    }
    Ok(())
}
// Retain call-site guards to observe expiry after blocking native calls.
pub struct Operation(bool);
impl Operation {
    pub fn new() -> Self { Self(owned()) }
}
impl Drop for Operation {
    fn drop(&mut self) {
        if self.0 { expired(); }
    }
}
