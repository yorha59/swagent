use crate::{
    ffi, http, install,
    io::{self, File, Socket},
    log,
    util::*,
};
use core::{
    fmt::Write,
    sync::atomic::{AtomicBool, AtomicI32, Ordering::*},
};
static RETIRING: AtomicBool = AtomicBool::new(false);
static LISTENER: AtomicI32 = AtomicI32::new(-1);
struct Session {
    fd: i32,
    id: u64,
}
static SESSION: Lock<Session> = Lock::new(Session { fd: -1, id: 0 });
#[derive(Clone, Copy)]
struct Job {
    id: u64,
    line: Text<512>,
}
const EMPTY: Job = Job {
    id: 0,
    line: Text::new(),
};
struct Queue {
    jobs: [Job; 8],
    head: usize,
    len: usize,
}
impl Queue {
    fn push(&mut self, job: Job) -> bool {
        if self.len == 8 {
            return false;
        }
        self.jobs[(self.head + self.len) % 8] = job;
        self.len += 1;
        true
    }
    fn pop(&mut self) -> Option<Job> {
        if self.len == 0 {
            return None;
        }
        let job = self.jobs[self.head];
        self.head = (self.head + 1) % 8;
        self.len -= 1;
        Some(job)
    }
}
pub struct Frame {
    line: Text<512>,
    overflow: bool,
    cr: bool,
}
impl Frame {
    pub fn new() -> Self {
        Self {
            line: Text::new(),
            overflow: false,
            cr: false,
        }
    }
    pub fn feed(&mut self, byte: u8) -> Option<Result<Text<512>>> {
        if self.cr && byte == b'\n' {
            let result = if self.overflow {
                Err(Error("invalid or oversized line", 0))
            } else {
                Ok(self.line)
            };
            *self = Self::new();
            return Some(result);
        }
        if self.cr {
            self.overflow = true;
            self.cr = false;
        }
        if byte == b'\r' {
            self.cr = true;
        } else if byte == b'\n' || byte == 0 || !byte.is_ascii() {
            self.overflow = true;
        } else if self.line.push(&[byte]).is_err() {
            self.overflow = true;
        }
        None
    }
}
static QUEUE: Lock<Queue> = Lock::new(Queue {
    jobs: [EMPTY; 8],
    head: 0,
    len: 0,
});
pub fn retiring() -> bool {
    RETIRING.load(Acquire)
}
pub fn reply(id: u64, line: &[u8]) -> Result {
    SESSION.with(|s| {
        if s.id != id || s.fd < 0 {
            return Err(Error("client disconnected", 0));
        }
        let sock = core::mem::ManuallyDrop::new(Socket(s.fd));
        sock.send(line)?;
        sock.send(b"\r\n")
    })
}
fn error(id: u64, prefix: &str, e: Error) {
    let mut s = Text::<256>::new();
    let _ = write!(s, "{}: error {} rc={}", prefix, e.0, e.1);
    log::add(s.str());
    let _ = reply(id, s.bytes());
}
fn enqueue(id: u64, line: Text<512>) {
    let ok = QUEUE.with(|q| q.push(Job { id, line }));
    if !ok {
        let _ = reply(id, b"error: busy");
    }
}
fn active(id: u64) -> bool {
    SESSION.with(|s| s.id == id && s.fd >= 0)
}
pub extern "C" fn worker(_: *mut core::ffi::c_void) {
    loop {
        if retiring() {
            forever()
        }
        let job = QUEUE.with(Queue::pop);
        if let Some(job) = job {
            if active(job.id) {
                execute(job)
            }
        } else {
            sleep(5)
        }
    }
}
fn execute(job: Job) {
    let id = job.id;
    let mut words = job.line.str().split_ascii_whitespace();
    let cmd = words.next().unwrap_or("");
    let result = (|| -> Result {
        match cmd {
            "install" => {
                let url = words
                    .next()
                    .ok_or(Error("usage: install URL [sd|nand]", 0))?;
                let storage = match words.next().unwrap_or("sd") {
                    "sd" => 5,
                    "nand" => 4,
                    _ => return Err(Error("destination must be sd or nand", 0)),
                };
                if words.next().is_some() {
                    return Err(Error("too many arguments", 0));
                }
                let title = install::install(url, storage)?;
                let mut s = Text::<64>::new();
                let _ = write!(s, "install: ok {:016x}", title);
                log::add(s.str());
                reply(id, s.bytes())?;
            }
            "log" => {
                let n = match words.next() {
                    Some(s) => s
                        .parse::<usize>()
                        .map_err(|_| Error("invalid log count", 0))?,
                    None => 100,
                };
                if words.next().is_some() {
                    return Err(Error("too many arguments", 0));
                }
                log::dump(n, |p| {
                    let _ = reply(id, p);
                });
                reply(id, b"log: end")?;
            }
            _ => {
                reply(id, b"error: unknown command")?;
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        error(id, cmd, e)
    }
}
const DEST: &str = "sdmc:/atmosphere/contents/430000000000000B/exefs.nsp";
const TMP: &str = "sdmc:/atmosphere/contents/430000000000000B/exefs.nsp.tmp";
const BACKUP: &str = "sdmc:/atmosphere/contents/430000000000000B/exefs.nsp.bak";
pub(crate) fn apply_update(url: &str) -> Result<u64> {
    let mut chunk = Buffer::new(64 * 1024)?;
    struct Clean;
    impl Drop for Clean {
        fn drop(&mut self) {
            io::remove(TMP)
        }
    }
    let _clean = Clean;
    log::add("update: downloading");
    let size = http::download(url, TMP, &mut chunk, 50 * 1024 * 1024)?;
    // Small headless builds can be below 100 KiB; validate the container below.
    if size < 16 {
        return Err(Error("update image too small", 0));
    }
    {
        let mut f = File::open(TMP, false)?;
        let p = install::Pfs::open(&mut f)?;
        let (mut nso, mut npdm) = (false, false);
        for i in 0..p.count {
            let e = p.entry(i)?;
            if e.name == "main" {
                if e.size < 0x100 {
                    return Err(Error("short update NSO", 0));
                }
                let mut magic = [0; 4];
                f.read(e.off, &mut magic)?;
                nso = &magic == b"NSO0";
            }
            if e.name == "main.npdm" {
                if e.size < 0x80 {
                    return Err(Error("short update NPDM", 0));
                }
                let mut h = [0; 0x80];
                f.read(e.off, &mut h)?;
                if &h[..4] != b"META" {
                    return Err(Error("invalid NPDM magic", 0));
                }
                let at = le32(&h[0x70..]) as u64;
                if at + 0x20 > e.size {
                    return Err(Error("invalid NPDM ACI offset", 0));
                }
                let mut aci = [0; 0x20];
                f.read(e.off + at, &mut aci)?;
                npdm = &aci[..4] == b"ACI0" && le64(&aci[0x10..]) == 0x430000000000000b;
            }
        }
        if !nso || !npdm {
            return Err(Error("update must contain main and matching NPDM", 0));
        }
    }
    // Keep a recoverable previous image until launch succeeds.
    io::remove(BACKUP);
    io::rename(DEST, BACKUP)?;
    if let Err(e) = io::rename(TMP, DEST) {
        let _ = io::rename(BACKUP, DEST);
        return Err(e);
    }
    let mut pid = 0;
    let launch = unsafe { ffi::sa_launch(&mut pid) };
    if launch != 0 {
        io::remove(DEST);
        let _ = io::rename(BACKUP, DEST);
        return Err(Error("launch replacement", launch));
    }
    Ok(pid)
}
fn update(id: u64, url: &str) -> Result {
    apply_update(url)?;
    RETIRING.store(true, Release);
    // Release server first, then client. Never terminate the old process.
    let listener = LISTENER.swap(-1, AcqRel);
    if listener >= 0 {
        unsafe {
            ffi::shutdown(listener, 2);
            ffi::close(listener);
        }
    }
    let _ = reply(id, b"update: ok");
    disconnect();
    log::add("update: retired; sleeping forever");
    forever()
}
fn disconnect() {
    SESSION.with(|s| {
        if s.fd >= 0 {
            unsafe {
                ffi::shutdown(s.fd, 2);
                ffi::close(s.fd);
            }
            s.fd = -1;
        }
    })
}
fn receive(id: u64, line: Text<512>) {
    let mut words = line.str().split_ascii_whitespace();
    match words.next() {
        Some("ping") if words.next().is_none() => {
            let _ = reply(id, b"pong");
        }
        Some("version") if words.next().is_none() => {
            let _ = reply(id, b"swagent-1.1-rs");
        }
        Some("update") => {
            let result = match (words.next(), words.next()) {
                (Some(url), None) => update(id, url),
                _ => Err(Error("usage: update URL", 0)),
            };
            if let Err(e) = result {
                error(id, "update", e)
            }
        }
        _ => enqueue(id, line),
    }
}
pub fn serve() -> ! {
    loop {
        if retiring() {
            forever()
        }
        // Initial attempt plus five EADDRINUSE retries at one-second intervals.
        let mut server = -1;
        for attempt in 0..=5 {
            server = unsafe { ffi::sa_listen() };
            if server >= 0 {
                break;
            }
            if server != -112 || attempt == 5 {
                break;
            }
            sleep(1000);
        }
        if server < 0 {
            log::add("listen failed; retrying");
            sleep(5000);
            continue;
        }
        LISTENER.store(server, Release);
        log::add("listening :6000");
        loop {
            let fd = unsafe { ffi::sa_accept(server) };
            if fd < 0 {
                break;
            }
            unsafe {
                ffi::sa_timeout(fd, 2);
            }
            let id = SESSION.with(|s| {
                s.id = s.id.wrapping_add(1);
                s.fd = fd;
                s.id
            });
            log::add("client connected");
            let mut frame = Frame::new();
            'client: loop {
                let mut input = [0; 512];
                let n = unsafe { ffi::recv(fd, input.as_mut_ptr(), input.len(), 0) };
                if n == 0 {
                    break;
                }
                if n < 0 {
                    let e = unsafe { ffi::sa_errno() };
                    if e == 11 || e == 35 || e == 116 {
                        continue;
                    }
                    break;
                }
                for byte in &input[..n as usize] {
                    if let Some(result) = frame.feed(*byte) {
                        match result {
                            Ok(line) if line.len > 0 => receive(id, line),
                            Err(_) => {
                                let _ = reply(id, b"error: invalid or oversized line");
                            }
                            _ => {}
                        }
                    }
                    if retiring() {
                        break 'client;
                    }
                }
            }
            disconnect();
            log::add("client disconnected");
        }
        LISTENER.store(-1, Release);
        unsafe {
            ffi::close(server);
        }
        sleep(1000);
    }
}

#[cfg(test)]
pub fn test_queue() {
    let mut q = Queue {
        jobs: [EMPTY; 8],
        head: 0,
        len: 0,
    };
    for id in 1..=8 {
        assert!(q.push(Job {
            id,
            line: Text::new()
        }));
    }
    assert!(!q.push(EMPTY));
    for id in 1..=4 {
        assert_eq!(q.pop().unwrap().id, id);
    }
    for id in 9..=12 {
        assert!(q.push(Job {
            id,
            line: Text::new()
        }));
    }
    for id in 5..=12 {
        assert_eq!(q.pop().unwrap().id, id);
    }
    assert!(q.pop().is_none());
}

#[cfg(test)]
pub fn test_session(fd: i32) {
    SESSION.with(|s| {
        s.fd = fd;
        s.id = 42;
    });
    QUEUE.with(|q| {
        q.head = 0;
        q.len = 0;
    });
}
#[cfg(test)]
pub fn test_receive(s: &str) {
    let mut line = Text::new();
    line.push(s.as_bytes()).unwrap();
    receive(42, line);
}
#[cfg(test)]
pub fn test_run_one() {
    if let Some(job) = QUEUE.with(Queue::pop) {
        execute(job)
    }
}
