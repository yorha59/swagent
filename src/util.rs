use crate::ffi;
use core::{
    cell::UnsafeCell,
    fmt,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error(pub &'static str, pub u32);
pub type Result<T = ()> = core::result::Result<T, Error>;
pub fn rc(code: u32, what: &'static str) -> Result {
    crate::budget::check()?;
    if code == 0 {
        Ok(())
    } else {
        Err(Error(what, code))
    }
}
pub fn now_ms() -> u64 {
    unsafe { ffi::sa_now_ms() }
}
pub fn sleep(ms: u64) {
    unsafe {
        ffi::svcSleepThread((ms * 1_000_000) as i64);
    }
}
pub fn forever() -> ! {
    loop {
        sleep(3_600_000)
    }
}
pub struct Buffer {
    p: *mut u8,
    n: usize,
}
impl Buffer {
    pub fn new(n: usize) -> Result<Self> {
        let p = unsafe { ffi::malloc(n).cast::<u8>() };
        if p.is_null() {
            Err(Error("out of memory", 0))
        } else {
            Ok(Self { p, n })
        }
    }
}
impl Deref for Buffer {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.p, self.n) }
    }
}
impl DerefMut for Buffer {
    fn deref_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.p, self.n) }
    }
}
impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe { ffi::free(self.p.cast()) }
    }
}
#[derive(Clone, Copy)]
pub struct Text<const N: usize> {
    pub data: [u8; N],
    pub len: usize,
}
impl<const N: usize> Text<N> {
    pub const fn new() -> Self {
        Self {
            data: [0; N],
            len: 0,
        }
    }
    pub fn bytes(&self) -> &[u8] {
        self.data.get(..self.len).unwrap_or(&[])
    }
    pub fn str(&self) -> &str {
        core::str::from_utf8(self.bytes()).unwrap_or("")
    }
    pub fn c(&self) -> *const core::ffi::c_char {
        self.data.as_ptr().cast()
    }
    pub fn push(&mut self, s: &[u8]) -> Result {
        let end = self
            .len
            .checked_add(s.len())
            .ok_or(Error("text too long", 0))?;
        if end >= N {
            return Err(Error("text too long", 0));
        }
        self.data
            .get_mut(self.len..end)
            .ok_or(Error("text bounds", 0))?
            .copy_from_slice(s);
        self.len += s.len();
        Ok(())
    }
}
impl<const N: usize> fmt::Write for Text<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push(s.as_bytes()).map_err(|_| fmt::Error)
    }
}
pub struct Lock<T> {
    busy: AtomicBool,
    value: UnsafeCell<T>,
}
unsafe impl<T: Send> Sync for Lock<T> {}
impl<T> Lock<T> {
    pub const fn new(v: T) -> Self {
        Self {
            busy: AtomicBool::new(false),
            value: UnsafeCell::new(v),
        }
    }
    pub fn with<R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        while self
            .busy
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            sleep(1)
        }
        let r = f(unsafe { &mut *self.value.get() });
        self.busy.store(false, Ordering::Release);
        r
    }
}
pub fn le16(p: &[u8]) -> Result<u16> {
    Ok(u16::from_le_bytes(
        p.get(..2)
            .ok_or(Error("short integer", 0))?
            .try_into()
            .map_err(|_| Error("short integer", 0))?,
    ))
}
pub fn le32(p: &[u8]) -> Result<u32> {
    Ok(u32::from_le_bytes(
        p.get(..4)
            .ok_or(Error("short integer", 0))?
            .try_into()
            .map_err(|_| Error("short integer", 0))?,
    ))
}
pub fn le64(p: &[u8]) -> Result<u64> {
    Ok(u64::from_le_bytes(
        p.get(..8)
            .ok_or(Error("short integer", 0))?
            .try_into()
            .map_err(|_| Error("short integer", 0))?,
    ))
}
pub fn hex(bytes: &[u8]) -> Text<80> {
    let mut s = Text::new();
    for b in bytes {
        let _ = s.push(&[
            b"0123456789abcdef"[(b >> 4) as usize],
            b"0123456789abcdef"[(b & 15) as usize],
        ]);
    }
    s
}
pub fn unhex(s: &[u8]) -> Result<[u8; 16]> {
    if s.len() != 32 {
        return Err(Error("invalid content id", 0));
    }
    let mut out = [0; 16];
    for (i, c) in s.iter().enumerate() {
        let v = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => return Err(Error("invalid content id", 0)),
        };
        out[i / 2] = (out[i / 2] << 4) | v;
    }
    Ok(out)
}
