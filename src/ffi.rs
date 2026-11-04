#![allow(non_snake_case)]
use core::ffi::{c_char,c_void};
#[repr(C)]
#[derive(Default)]
pub struct Service { pub session:u32, pub owned:u32, pub object:u32, pub pointer:u16, pub pad:u16 }
#[repr(C)]
#[derive(Clone,Copy,Default)]
pub struct Key { pub id:u64,pub version:u32,pub kind:u8,pub install:u8,pub pad:[u8;2] }
#[repr(C)]
#[derive(Clone,Copy,Default)]
pub struct Record { pub key:Key,pub storage:u64 }
extern "C" {
    pub fn malloc(n:usize)->*mut u8;
    pub fn free(p:*mut u8);
    pub fn sa_init()->u32;
    pub fn sa_spawn(slot:u32,entry:extern "C" fn(*mut c_void),arg:*mut c_void)->u32;
    pub fn svcSleepThread(ns:i64)->u32;
    pub fn sa_listen()->i32;
    pub fn sa_accept(fd:i32)->i32;
    pub fn sa_connect(host:*const c_char,port:*const c_char)->i32;
    pub fn sa_timeout(fd:i32,secs:i32);
    pub fn sa_errno()->i32;
    pub fn close(fd:i32)->i32;
    pub fn shutdown(fd:i32,how:i32)->i32;
    pub fn recv(fd:i32,p:*mut u8,n:usize,flags:i32)->isize;
    pub fn send(fd:i32,p:*const u8,n:usize,flags:i32)->isize;
    pub fn mkdir(p:*const c_char,mode:u32)->i32;
    pub fn sa_open(p:*const c_char,write:i32,f:*mut Service)->u32;
    pub fn sa_remove(p:*const c_char)->u32;
    pub fn sa_rename(a:*const c_char,b:*const c_char)->u32;
    pub fn fsFileClose(f:*mut Service);
    pub fn fsFileRead(f:*mut Service,off:i64,p:*mut u8,n:u64,opt:u32,out:*mut u64)->u32;
    pub fn fsFileWrite(f:*mut Service,off:i64,p:*const u8,n:u64,opt:u32)->u32;
    pub fn fsFileFlush(f:*mut Service)->u32;
    pub fn fsFileGetSize(f:*mut Service,out:*mut i64)->u32;
    pub fn fsFsClose(s:*mut Service);
    pub fn sa_cnmt_open(p:*const c_char,f:*mut Service,fs:*mut Service)->u32;
    pub fn sa_ticket(t:*const u8,tn:usize,c:*const u8,cn:usize)->u32;
    pub fn ncmInitialize()->u32;
    pub fn ncmExit();
    pub fn ncmOpenContentStorage(s:*mut Service,id:u32)->u32;
    pub fn ncmOpenContentMetaDatabase(s:*mut Service,id:u32)->u32;
    pub fn ncmContentStorageClose(s:*mut Service);
    pub fn ncmContentMetaDatabaseClose(s:*mut Service);
    pub fn ncmContentStorageHas(s:*mut Service,out:*mut bool,id:*const u8)->u32;
    pub fn ncmContentStorageGeneratePlaceHolderId(s:*mut Service,id:*mut u8)->u32;
    pub fn ncmContentStorageCreatePlaceHolder(s:*mut Service,id:*const u8,ph:*const u8,n:i64)->u32;
    pub fn ncmContentStorageWritePlaceHolder(s:*mut Service,ph:*const u8,off:u64,p:*const u8,n:usize)->u32;
    pub fn ncmContentStorageRegister(s:*mut Service,id:*const u8,ph:*const u8)->u32;
    pub fn ncmContentStorageDeletePlaceHolder(s:*mut Service,ph:*const u8)->u32;
    pub fn ncmContentMetaDatabaseSet(s:*mut Service,key:*const Key,p:*const u8,n:u64)->u32;
    pub fn ncmContentMetaDatabaseCommit(s:*mut Service)->u32;
    pub fn nsInitialize()->u32;
    pub fn nsExit();
    pub fn nsCountApplicationContentMeta(id:u64,count:*mut i32)->u32;
    pub fn sa_ns(cmd:u32,p:*const u8,n:u32,buf:*mut u8,size:usize,count:*mut u32)->u32;
    pub fn sa_hid_init()->u32;
    pub fn sa_buttons(b:u64)->u32;
    pub fn sa_touch(x:u32,y:u32,ns:u64,down:i32)->u32;
    pub fn hiddbgUnsetTouchScreenAutoPilotState()->u32;
    pub fn sa_capture(p:*mut u8,n:usize,out:*mut u64)->u32;
    pub fn sa_jpeg()->*mut c_void;
    pub fn sa_jpeg_header(h:*mut c_void,p:*const u8,n:usize,w:*mut i32,height:*mut i32)->i32;
    pub fn sa_jpeg_decode(h:*mut c_void,p:*const u8,n:usize,out:*mut u8,w:i32,height:i32)->i32;
    pub fn sa_jpeg_close(h:*mut c_void);
    pub fn sa_launch(pid:*mut u64)->u32;
}
