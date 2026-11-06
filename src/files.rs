use crate::{io::File, util::*};
use core::fmt::Write;
pub const MAX: usize = 256 * 1024;

pub fn path(input: &str, write: bool) -> Result<Text<768>> {
    let p = input
        .strip_prefix("sdmc:")
        .unwrap_or(input)
        .trim_start_matches('/');
    if p.is_empty()
        || !p.is_ascii()
        || p.contains("..")
        || p.contains(':')
        || p.contains('\\')
        || p.bytes().any(|b| b.is_ascii_control())
        || (write && !(p.starts_with("atmosphere/config/") || p.starts_with("switch/")))
    {
        return Err(Error("path not allowed", 0));
    }
    let mut out = Text::new();
    out.push(b"sdmc:/")?;
    out.push(p.as_bytes())?;
    Ok(out)
}
pub trait Vfs {
    fn read(&self, path: &str) -> Result<Buffer>;
    fn put(&self, path: &str, bytes: &[u8]) -> Result;
    fn mkdir(&self, path: &str) -> Result;
    fn list(&self, path: &str, emit: &mut dyn FnMut(bool, u64, &str) -> Result) -> Result;
}
pub struct Sd;
impl Vfs for Sd {
    fn read(&self, p: &str) -> Result<Buffer> {
        let mut f = File::open(p, false)?;
        let n = f.size()?;
        if n > MAX as u64 {
            return Err(Error("too large", 0));
        }
        let mut bytes = Buffer::new(n as usize)?;
        f.read(0, &mut bytes)?;
        Ok(bytes)
    }
    fn put(&self, p: &str, bytes: &[u8]) -> Result {
        let mut f = File::open(p, true)?;
        f.write(0, bytes)?;
        f.flush()?;
        drop(f);
        Ok(())
    }
    fn mkdir(&self, p: &str) -> Result {
        let p = path(p, true)?;
        rc(unsafe { crate::ffi::sa_mkdir(p.c()) }, "mkdir")
    }
    fn list(&self, p: &str, emit: &mut dyn FnMut(bool, u64, &str) -> Result) -> Result {
        let p = path(p, false)?;
        let mut dir = crate::ffi::Service::default();
        rc(
            unsafe { crate::ffi::sa_dir_open(p.c(), &mut dir) },
            "open directory",
        )?;
        struct Dir(crate::ffi::Service);
        impl Drop for Dir {
            fn drop(&mut self) {
                unsafe { crate::ffi::fsDirClose(&mut self.0) }
            }
        }
        let mut dir = Dir(dir);
        loop {
            let mut name = [0u8; 0x301];
            let (mut size, mut kind, mut count) = (0, 0, 0);
            rc(
                unsafe {
                    crate::ffi::sa_dir_read(
                        &mut dir.0,
                        name.as_mut_ptr(),
                        &mut size,
                        &mut kind,
                        &mut count,
                    )
                },
                "read directory",
            )?;
            if count == 0 {
                return Ok(());
            }
            let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
            let name = core::str::from_utf8(&name[..end]).map_err(|_| Error("invalid name", 0))?;
            if name != "." && name != ".." {
                emit(kind == 0, size, name)?;
            }
        }
    }
}
pub fn run(
    vfs: &impl Vfs,
    cmd: &str,
    input: &str,
    body: &[u8],
    send: &mut dyn FnMut(&[u8]) -> Result,
) -> Result {
    let result = (|| {
        let p = path(input, cmd == "put" || cmd == "mkdir")?;
        match cmd {
            "cat" => {
                let bytes = vfs.read(p.str())?;
                if bytes.len() > MAX {
                    return Err(Error("too large", 0));
                }
                let mut line = Text::<64>::new();
                let _ = write!(line, "cat: begin {}\r\n", bytes.len());
                send(line.bytes())?;
                send(&bytes)?;
                send(b"cat: end\r\n")
            }
            "put" => {
                if body.len() > MAX {
                    return Err(Error("too large", 0));
                }
                vfs.put(p.str(), body)?;
                let mut line = Text::<64>::new();
                let _ = write!(line, "put: ok {}\r\n", body.len());
                send(line.bytes())
            }
            "mkdir" => {
                vfs.mkdir(p.str())?;
                send(b"mkdir: ok\r\n")
            }
            "ls" => {
                vfs.list(p.str(), &mut |dir, size, name| {
                    let mut line = Text::<1024>::new();
                    let _ = write!(line, "ls: {} {} ", if dir { 'd' } else { 'f' }, size);
                    for b in name.bytes() {
                        line.push(&[if b.is_ascii_control() { b'?' } else { b }])?;
                    }
                    line.push(b"\r\n")?;
                    send(line.bytes())
                })?;
                send(b"ls: end\r\n")
            }
            _ => Err(Error("usage", 0)),
        }
    })();
    if let Err(e) = result {
        let mut line = Text::<128>::new();
        if e.0 == "path not allowed" || e.0 == "too large" {
            let _ = write!(line, "{}: error {}\r\n", cmd, e.0);
        } else {
            let _ = write!(line, "{}: error rc={}\r\n", cmd, e.1);
        }
        send(line.bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Host(std::path::PathBuf);
    fn err(e: std::io::Error) -> Error {
        Error("io", e.raw_os_error().unwrap_or(0) as u32)
    }
    impl Host {
        fn p(&self, p: &str) -> std::path::PathBuf {
            self.0.join(p.strip_prefix("sdmc:/").unwrap())
        }
    }
    impl Vfs for Host {
        fn read(&self, p: &str) -> Result<Buffer> {
            use std::io::Read;
            let mut f = std::fs::File::open(self.p(p)).map_err(err)?;
            let n = f.metadata().map_err(err)?.len();
            if n > MAX as u64 {
                return Err(Error("too large", 0));
            }
            let mut b = Buffer::new(n as usize)?;
            f.read_exact(&mut b).map_err(err)?;
            Ok(b)
        }
        fn put(&self, p: &str, b: &[u8]) -> Result {
            use std::io::Write;
            let mut f = std::fs::File::create(self.p(p)).map_err(err)?;
            f.write_all(b).map_err(err)?;
            f.flush().map_err(err)?;
            drop(f);
            Ok(())
        }
        fn mkdir(&self, p: &str) -> Result {
            std::fs::create_dir(self.p(p)).map_err(err)
        }
        fn list(&self, p: &str, emit: &mut dyn FnMut(bool, u64, &str) -> Result) -> Result {
            for entry in std::fs::read_dir(self.p(p)).map_err(err)? {
                let e = entry.map_err(err)?;
                let m = e.metadata().map_err(err)?;
                emit(
                    m.is_dir(),
                    if m.is_dir() { 0 } else { m.len() },
                    e.file_name().to_str().unwrap(),
                )?;
            }
            Ok(())
        }
    }
    #[test]
    fn whitelist() {
        for p in [
            "atmosphere/config/x.ini",
            "switch/y",
            "/switch/y",
            "///atmosphere/config/x",
            "sdmc:/switch/y",
        ] {
            assert!(path(p, true).is_ok(), "{p}");
        }
        for p in [
            "",
            "é",
            "switch/é",
            "atmosphere/",
            "sdmc:/atmosphere/hosts",
            "/../switch/x",
            "//switch/../../x",
            "switch/a..b",
            "switch/../x",
            "Switch/x",
            "/sdmc:/switch/x",
            "switch/\\x",
        ] {
            assert!(path(p, true).is_err(), "{p}");
        }
    }
    #[test]
    fn real_fs_commands() {
        let root = std::env::temp_dir().join(format!("swagent-files-{}", std::process::id()));
        std::fs::create_dir_all(root.join("switch")).unwrap();
        let fs = Host(root.clone());
        let mut out = Vec::new();
        let body = b"\0\r\nversion\r\n\xff";
        run(&fs, "put", "switch/test", body, &mut |b| {
            out.extend_from_slice(b);
            Ok(())
        })
        .unwrap();
        assert_eq!(out, format!("put: ok {}\r\n", body.len()).as_bytes());
        out.clear();
        run(&fs, "cat", "switch/test", &[], &mut |b| {
            out.extend_from_slice(b);
            Ok(())
        })
        .unwrap();
        let mut expected = format!("cat: begin {}\r\n", body.len()).into_bytes();
        expected.extend_from_slice(body);
        expected.extend_from_slice(b"cat: end\r\n");
        assert_eq!(out, expected);
        out.clear();
        run(&fs, "ls", "switch", &[], &mut |b| {
            out.extend_from_slice(b);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            out,
            format!("ls: f {} test\r\nls: end\r\n", body.len()).as_bytes()
        );
        std::fs::write(root.join("switch/test"), vec![0; MAX + 1]).unwrap();
        out.clear();
        run(&fs, "cat", "switch/test", &[], &mut |b| {
            out.extend_from_slice(b);
            Ok(())
        })
        .unwrap();
        assert_eq!(out, b"cat: error too large\r\n");
        out.clear();
        run(&fs, "mkdir", "switch/sub", &[], &mut |b| {
            out.extend_from_slice(b);
            Ok(())
        })
        .unwrap();
        assert_eq!(out, b"mkdir: ok\r\n");
        out.clear();
        run(&fs, "ls", "switch/sub", &[], &mut |b| {
            out.extend_from_slice(b);
            Ok(())
        })
        .unwrap();
        assert_eq!(out, b"ls: end\r\n");
        out.clear();
        run(&fs, "put", "switch/test", &[], &mut |b| {
            out.extend_from_slice(b);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            std::fs::metadata(root.join("switch/test")).unwrap().len(),
            0
        );
        assert_eq!(out, b"put: ok 0\r\n");
        out.clear();
        run(&fs, "cat", "switch/missing", &[], &mut |b| {
            out.extend_from_slice(b);
            Ok(())
        })
        .unwrap();
        assert!(out.starts_with(b"cat: error rc="));
        std::fs::remove_dir_all(root).unwrap();
    }
}
