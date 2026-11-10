//! Dedicated bounded socket lane; never uses command state, logging or locks.
use crate::{ffi, util::{now_ms, sleep}};
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering::*};
static RESTART: AtomicBool = AtomicBool::new(false);
static LISTENER: AtomicI32 = AtomicI32::new(-1);
static BEAT: AtomicU64 = AtomicU64::new(0);
/// The watchdog probes without taking the command listener's ownership lock.
/// Only the heartbeat owner closes descriptors, avoiding close/reuse races.
pub fn check() {
    let fd=LISTENER.load(Acquire);
    if fd>=0 && (now_ms().saturating_sub(BEAT.load(Acquire))>=60000 || unsafe { ffi::sa_heartbeat_health(fd) }==0) {
        RESTART.store(true,Release);
    }
}
pub(crate) fn reply(fd: i32) {
    let deadline=now_ms()+2000;
    let mut byte=[0u8;1];
    while !crate::stopping() && now_ms()<deadline {
        let n=unsafe { ffi::sa_recv(fd,byte.as_mut_ptr(),1,0) };
        if n>0 {
            let mut sent=0;
            while sent<5 && now_ms()<deadline {
                let n=unsafe { ffi::sa_send(fd,b"pong\n"[sent..].as_ptr(),5-sent,0) };
                if n>0 { sent+=n as usize; }
                else if n==0 || !crate::net::transient(unsafe { ffi::sa_errno() }) { break; }
                else { sleep(5); }
            }
            break;
        }
        if n==0 || !crate::net::transient(unsafe { ffi::sa_errno() }) { break; }
        sleep(5);
    }
    unsafe { ffi::close(fd); }
}
pub extern "C" fn serve(_: *mut core::ffi::c_void) {
    let mut fd=-1;
    let mut delay=if crate::is_nro() {5000} else {250};
    while !crate::stopping() {
        BEAT.store(now_ms(),Release);
        if RESTART.swap(false,AcqRel) && fd>=0 {
            LISTENER.store(-1,Release);
            unsafe { ffi::close(fd); } fd=-1;
        }
        if fd<0 {
            fd=unsafe { ffi::sa_heartbeat_listen() };
            if fd<0 {
                let mut left=delay;
                while left>0 && !crate::stopping() { let n=left.min(100); sleep(n); left-=n; }
                delay=(delay*2).min(30000); continue;
            }
            delay=if crate::is_nro() {5000} else {250};
            LISTENER.store(fd,Release);
        }
        let client=unsafe { ffi::sa_heartbeat_accept(fd) };
        if client>=0 { reply(client); }
        else if crate::net::transient(unsafe { ffi::sa_errno() }) { sleep(20); }
        else { LISTENER.store(-1,Release); unsafe { ffi::close(fd); } fd=-1; }
    }
    LISTENER.store(-1,Release);
    if fd>=0 { unsafe { ffi::close(fd); } }
}
