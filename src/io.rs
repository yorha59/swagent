use crate::{ffi, util::*};
pub struct File(pub ffi::Service);
impl File {
    pub fn open(path: &str, write: bool) -> Result<Self> {
        Self::open_mode(path, write as i32)
    }
    pub fn open_mode(path: &str, mode: i32) -> Result<Self> {
        let mut p = Text::<768>::new();
        p.push(path.as_bytes())?;
        let mut s = ffi::Service::default();
        rc(unsafe { ffi::sa_open(p.c(), mode, &mut s) }, "open file")?;
        Ok(Self(s))
    }
    pub fn read(&mut self, off: u64, p: &mut [u8]) -> Result {
        let mut n = 0;
        rc(
            unsafe {
                ffi::fsFileRead(
                    &mut self.0,
                    off as i64,
                    p.as_mut_ptr(),
                    p.len() as u64,
                    0,
                    &mut n,
                )
            },
            "read file",
        )?;
        if n != p.len() as u64 {
            Err(Error("short file read", 0))
        } else {
            Ok(())
        }
    }
    pub fn write(&mut self, off: u64, p: &[u8]) -> Result {
        rc(
            unsafe { ffi::fsFileWrite(&mut self.0, off as i64, p.as_ptr(), p.len() as u64, 0) },
            "write file",
        )
    }
    pub fn flush(&mut self) -> Result {
        rc(unsafe { ffi::fsFileFlush(&mut self.0) }, "flush file")
    }
    pub fn size(&mut self) -> Result<u64> {
        let mut n = 0;
        rc(
            unsafe { ffi::fsFileGetSize(&mut self.0, &mut n) },
            "file size",
        )?;
        if n < 0 {
            Err(Error("negative file size", 0))
        } else {
            Ok(n as u64)
        }
    }
}
impl Drop for File {
    fn drop(&mut self) {
        unsafe { ffi::fsFileClose(&mut self.0) }
    }
}
pub fn remove(path: &str) {
    let mut p = Text::<768>::new();
    if p.push(path.as_bytes()).is_ok() {
        unsafe {
            ffi::sa_remove(p.c());
        }
    }
}
pub fn rename(from: &str, to: &str) -> Result {
    let mut a = Text::<768>::new();
    let mut b = Text::<768>::new();
    a.push(from.as_bytes())?;
    b.push(to.as_bytes())?;
    rc(unsafe { ffi::sa_rename(a.c(), b.c()) }, "rename file")
}
pub struct Socket(pub i32, u64);
impl Socket {
    pub fn new(fd: i32) -> Self {
        Self(fd, crate::net::epoch())
    }
    pub fn connect(host: *const core::ffi::c_char, port: *const core::ffi::c_char) -> Result<Self> {
        crate::net::GATE.with(|_| {
            if !crate::net::ready() {
                return Err(Error("network unavailable", 0));
            }
            let fd = unsafe { ffi::sa_connect(host, port) };
            if fd < 0 {
                let e = unsafe { ffi::sa_errno() };
                crate::net::failed(e);
                return Err(Error("HTTP connect", e as u32));
            }
            Ok(Self::new(fd))
        })
    }
    fn valid(&self) -> Result {
        if crate::net::ready() && self.1 == crate::net::epoch() {
            Ok(())
        } else {
            Err(Error("network reset", 0))
        }
    }
    pub fn read(&self, p: &mut [u8]) -> Result<usize> {
        crate::net::GATE.with(|_| {
            self.valid()?;
            // All HTTP header/body/chunk loops share this per-receive deadline.
            unsafe { ffi::sa_timeout(self.0, 30); }
            let n = unsafe { ffi::sa_recv(self.0, p.as_mut_ptr(), p.len(), 0) };
            if n < 0 {
                let e = unsafe { ffi::sa_errno() };
                crate::net::failed(e);
                Err(Error("network receive", e as u32))
            } else if n as usize > p.len() {
                Err(Error("invalid receive length", 0))
            } else {
                Ok(n as usize)
            }
        })
    }
    pub fn send(&self, mut p: &[u8]) -> Result {
        while !p.is_empty() {
            let n = crate::net::GATE.with(|_| {
                self.valid()?;
                let n = unsafe { ffi::sa_send(self.0, p.as_ptr(), p.len(), 0) };
                if n <= 0 {
                    let e = unsafe { ffi::sa_errno() };
                    crate::net::failed(e);
                    Err(Error("network send", e as u32))
                } else {
                    Ok(n as usize)
                }
            })?;
            p = p.get(n..).ok_or(Error("invalid send length", 0))?;
        }
        Ok(())
    }
}
impl Drop for Socket {
    fn drop(&mut self) {
        crate::net::GATE.with(|_| {
            if self.valid().is_ok() {
                unsafe {
                    ffi::close(self.0);
                }
            }
        });
    }
}
