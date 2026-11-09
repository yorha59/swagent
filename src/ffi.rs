#![allow(non_snake_case)]
use core::ffi::{c_char, c_void};
#[repr(C)]
#[derive(Default)]
pub struct Service {
    pub session: u32,
    pub owned: u32,
    pub object: u32,
    pub pointer: u16,
    pub pad: u16,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Key {
    pub id: u64,
    pub version: u32,
    pub kind: u8,
    pub install: u8,
    pub pad: [u8; 2],
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Record {
    pub key: Key,
    pub storage: u64,
}
#[repr(C)]
#[derive(Default, Clone, Copy)]
pub struct SleepSettings {
    pub flags: u32,
    pub handheld: i32,
    pub console: i32,
}
extern "C" {
    pub fn sa_dump_mkdir(path: *const c_char) -> u32;
    pub fn sa_dump_meta(title: u64, storage: *mut u32, infos: *mut u8, capacity: u32, count: *mut u32) -> u32;
    pub fn sa_dump_read(storage: u32, id: *const u8, offset: u64, bytes: *mut u8, size: usize) -> u32;
    pub fn sa_dump_ticket(storage: u32, infos: *const u8, count: u32, rights: *mut u8, ticket: *mut u8, ticket_cap: usize, tn: *mut usize, cert: *mut u8, cert_cap: usize, cn: *mut usize) -> u32;
    pub fn sa_jobs_init();
    pub fn sa_jobs_signal();
    pub fn sa_jobs_wait();
    pub fn sa_persist(p: *const u8, n: usize);
    pub fn sa_log_tail(p: *mut u8, n: usize) -> usize;
    pub fn sa_installed(offset: i32, title: *mut u64, name: *mut u8, capacity: usize, count: *mut i32) -> u32;
    pub fn sa_installed_delete(title: u64) -> u32;
    pub fn sa_version() -> *const c_char;
    pub fn sa_is_nro() -> i32;
    pub fn sa_quit();
    pub fn sa_stopping() -> i32;
    pub fn sa_listener_close(fd: i32);
    pub fn sa_wifi(join: u32, profile: *const u8, records: *mut u8, count: *mut i32) -> u32;
    pub fn sa_mkdir(p: *const c_char) -> u32;
    pub fn sa_dir_open(p: *const c_char, out: *mut Service) -> u32;
    pub fn sa_dir_read(
        d: *mut Service,
        name: *mut u8,
        size: *mut u64,
        kind: *mut i32,
        count: *mut i64,
    ) -> u32;
    pub fn fsDirClose(d: *mut Service);
    pub fn sa_power_psm(percent: *mut u32, charger: *mut u32) -> u32;
    pub fn sa_power_gauge(voltage: *mut u8, current: *mut u8) -> u32;
    pub fn setsysInitialize() -> u32;
    pub fn setsysExit();
    pub fn setsysGetSleepSettings(out: *mut SleepSettings) -> u32;
    pub fn setsysSetSleepSettings(settings: *const SleepSettings) -> u32;
    pub fn sa_setting_read(key: u32, out: *mut u8, capacity: usize) -> u32;
    pub fn malloc(n: usize) -> *mut c_void;
    pub fn free(p: *mut c_void);
    pub fn sa_net_reinit() -> u32;
    pub fn sa_init() -> u32;
    pub fn sa_nonblocking(fd: i32) -> i32;
    pub fn sa_thread_id() -> u64;
    pub fn sa_now_ms() -> u64;
    pub fn sa_spawn(slot: u32, entry: extern "C" fn(*mut c_void), arg: *mut c_void) -> u32;
    pub fn svcSleepThread(ns: i64) -> u32;
    pub fn sa_listen() -> i32;
    pub fn sa_accept(fd: i32) -> i32;
    pub fn sa_connect(host: *const c_char, port: *const c_char) -> i32;
    pub fn sa_timeout(fd: i32, secs: i32);
    pub fn sa_errno() -> i32;
    #[cfg_attr(not(test), link_name = "sa_close")]
    pub fn close(fd: i32) -> i32;
    pub fn shutdown(fd: i32, how: i32) -> i32;
    pub fn sa_recv(fd: i32, p: *mut u8, n: usize, flags: i32) -> isize;
    pub fn sa_send(fd: i32, p: *const u8, n: usize, flags: i32) -> isize;
    pub fn mkdir(p: *const c_char, mode: u32) -> i32;
    pub fn sa_open(p: *const c_char, write: i32, f: *mut Service) -> u32;
    pub fn sa_sd_space(total: *mut i64, free_space: *mut i64) -> u32;
    pub fn sa_remove(p: *const c_char) -> u32;
    pub fn sa_rename(a: *const c_char, b: *const c_char) -> u32;
    pub fn fsFileClose(f: *mut Service);
    pub fn fsFileRead(
        f: *mut Service,
        off: i64,
        p: *mut u8,
        n: u64,
        opt: u32,
        out: *mut u64,
    ) -> u32;
    pub fn fsFileWrite(f: *mut Service, off: i64, p: *const u8, n: u64, opt: u32) -> u32;
    pub fn fsFileFlush(f: *mut Service) -> u32;
    pub fn fsFileGetSize(f: *mut Service, out: *mut i64) -> u32;
    pub fn fsFsClose(s: *mut Service);
    pub fn sa_stage_open(name: *const c_char, size: i64, f: *mut Service, fs: *mut Service) -> u32;
    pub fn sa_stage_commit(fs: *mut Service) -> u32;
    pub fn sa_stage_close(fs: *mut Service);
    pub fn sa_cnmt_open(p: *const c_char, f: *mut Service, fs: *mut Service) -> u32;
    pub fn sa_ticket(t: *const u8, tn: usize, c: *const u8, cn: usize) -> u32;
    pub fn ncmInitialize() -> u32;
    pub fn ncmExit();
    pub fn ncmOpenContentStorage(s: *mut Service, id: u32) -> u32;
    pub fn ncmOpenContentMetaDatabase(s: *mut Service, id: u32) -> u32;
    pub fn ncmContentStorageClose(s: *mut Service);
    pub fn ncmContentMetaDatabaseClose(s: *mut Service);
    pub fn ncmContentStorageHas(s: *mut Service, out: *mut bool, id: *const u8) -> u32;
    pub fn ncmContentStorageGeneratePlaceHolderId(s: *mut Service, id: *mut u8) -> u32;
    pub fn ncmContentStorageCreatePlaceHolder(
        s: *mut Service,
        id: *const u8,
        ph: *const u8,
        n: i64,
    ) -> u32;
    pub fn ncmContentStorageWritePlaceHolder(
        s: *mut Service,
        ph: *const u8,
        off: u64,
        p: *const u8,
        n: usize,
    ) -> u32;
    pub fn ncmContentStorageRegister(s: *mut Service, id: *const u8, ph: *const u8) -> u32;
    pub fn ncmContentStorageDeletePlaceHolder(s: *mut Service, ph: *const u8) -> u32;
    pub fn ncmContentMetaDatabaseSet(s: *mut Service, key: *const Key, p: *const u8, n: u64)
        -> u32;
    pub fn ncmContentMetaDatabaseCommit(s: *mut Service) -> u32;
    pub fn nsInitialize() -> u32;
    pub fn nsExit();
    pub fn nsCountApplicationContentMeta(id: u64, count: *mut i32) -> u32;
    pub fn sa_ns(cmd: u32, p: *const u8, n: u32, buf: *mut u8, size: usize, count: *mut u32)
        -> u32;
    pub fn sa_request_launch(title_id: u64) -> u32;
    pub fn sa_launch(pid: *mut u64) -> u32;
    pub fn svcGetProcessId(pid: *mut u64, handle: u32) -> u32;
    pub fn svcGetProcessList(count: *mut i32, pids: *mut u64, capacity: u32) -> u32;
    pub fn pminfoInitialize() -> u32;
    pub fn pminfoExit();
    pub fn pminfoGetProgramId(program_id: *mut u64, pid: u64) -> u32;
    pub fn sa_terminate(pid: u64) -> u32;
}
