use crate::{ffi, util::*};
pub struct File(pub ffi::Service);
impl File {
    pub fn open(path: &str, write: bool) -> Result<Self> {
        Self::open_mode(path, write as i32)
    }
    pub fn open_mode(path: &str, mode: i32) -> Result<Self> {
        crate::budget::check()?;
        let mut p = Text::<768>::new();
        p.push(path.as_bytes())?;
        let mut s = ffi::Service::default();
        rc(
            unsafe {
                let _operation = crate::budget::Operation::new();
                ffi::sa_open(p.c(), mode, &mut s)
            },
            "open file",
        )?;
        Ok(Self(s))
    }
    pub fn read(&mut self, off: u64, p: &mut [u8]) -> Result {
        crate::budget::check()?;
        let mut n = 0;
        rc(
            unsafe {
                let _operation = crate::budget::Operation::new();
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
        crate::budget::check()?;
        rc(
            unsafe {
                let _operation = crate::budget::Operation::new();
                ffi::fsFileWrite(&mut self.0, off as i64, p.as_ptr(), p.len() as u64, 0)
            },
            "write file",
        )
    }
    pub fn flush(&mut self) -> Result {
        crate::budget::check()?;
        rc(
            unsafe {
                let _operation = crate::budget::Operation::new();
                ffi::fsFileFlush(&mut self.0)
            },
            "flush file",
        )
    }
    pub fn size(&mut self) -> Result<u64> {
        crate::budget::check()?;
        let mut n = 0;
        rc(
            unsafe {
                let _operation = crate::budget::Operation::new();
                ffi::fsFileGetSize(&mut self.0, &mut n)
            },
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
        unsafe {
            let _operation = crate::budget::Operation::new();
            ffi::fsFileClose(&mut self.0)
        }
    }
}
pub fn remove(path: &str) {
    let mut p = Text::<768>::new();
    if p.push(path.as_bytes()).is_ok() {
        unsafe {
            let _operation = crate::budget::Operation::new();
            ffi::sa_remove(p.c());
        }
    }
}
pub fn rename(from: &str, to: &str) -> Result {
    let mut a = Text::<768>::new();
    let mut b = Text::<768>::new();
    a.push(from.as_bytes())?;
    b.push(to.as_bytes())?;
    rc(
        unsafe {
            let _operation = crate::budget::Operation::new();
            ffi::sa_rename(a.c(), b.c())
        },
        "rename file",
    )
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
            let fd = unsafe {
                let _operation = crate::budget::Operation::new();
                ffi::sa_connect(host, port)
            };
            if fd < 0 {
                let e = unsafe {
                    let _operation = crate::budget::Operation::new();
                    ffi::sa_errno()
                };
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
        crate::budget::check()?;
        let deadline = now_ms() + 30_000;
        loop {
            let result = crate::net::GATE.with(|_| {
                self.valid()?;
                let _operation = crate::budget::Operation::new();
                unsafe {
                    ffi::sa_timeout(self.0, 2);
                }
                let n = unsafe { ffi::sa_recv(self.0, p.as_mut_ptr(), p.len(), 0) };
                if n < 0 {
                    Err(Error("network receive", unsafe { ffi::sa_errno() } as u32))
                } else if n as usize > p.len() {
                    Err(Error("invalid receive length", 0))
                } else {
                    Ok(n as usize)
                }
            });
            match result {
                Err(Error("network receive", e))
                    if crate::net::transient(e as i32)
                        && unsafe { ffi::sa_nonblocking(self.0) } != 0 =>
                {
                    crate::budget::check()?;
                    if now_ms() >= deadline {
                        return Err(Error("network receive", 110));
                    }
                    sleep(10);
                }
                other => return other,
            }
        }
    }
    pub fn send(&self, mut p: &[u8]) -> Result {
        crate::budget::check()?;
        let deadline = now_ms() + 2000;
        while !p.is_empty() {
            if now_ms() >= deadline {
                return Err(Error("network send budget", 110));
            }
            let n = crate::net::GATE.with(|_| {
                self.valid()?;
                unsafe {
                    ffi::sa_timeout(self.0, 2);
                }
                let n = unsafe {
                    let _operation = crate::budget::Operation::new();
                    ffi::sa_send(self.0, p.as_ptr(), p.len(), 0)
                };
                if n <= 0 {
                    let e = unsafe {
                        let _operation = crate::budget::Operation::new();
                        ffi::sa_errno()
                    };
                    Err(Error("network send", e as u32))
                } else {
                    Ok(n as usize)
                }
            });
            let n = match n {
                Err(Error("network send", e)) if crate::net::transient(e as i32) => {
                    crate::budget::check()?;
                    sleep(10);
                    continue;
                }
                other => other?,
            };
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
                    let _operation = crate::budget::Operation::new();
                    ffi::close(self.0);
                }
            }
        });
    }
}
