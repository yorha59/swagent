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
const CLIENTS: usize = 4;
static SESSIONS: [Lock<Session>; CLIENTS] =
    [const { Lock::new(Session { fd: -1, id: 0 }) }; CLIENTS];
fn session(id: u64) -> &'static Lock<Session> {
    &SESSIONS[id as usize % CLIENTS]
}
static NEXT_ID: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(4);
static CLIENT_READY: [AtomicBool; CLIENTS] = [const { AtomicBool::new(false) }; CLIENTS];
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
        let Some(slot) = self.jobs.get_mut((self.head + self.len) % 8) else {
            return false;
        };
        *slot = job;
        self.len += 1;
        true
    }
    fn pop(&mut self) -> Option<Job> {
        if self.len == 0 {
            return None;
        }
        let job = *self.jobs.get(self.head)?;
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
    session(id).with(|s| {
        if s.id != id || s.fd < 0 {
            return Err(Error("client disconnected", 0));
        }
        let sock = core::mem::ManuallyDrop::new(Socket::new(s.fd));
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
    session(id).with(|s| s.id == id && s.fd >= 0)
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
fn title_arg(words: &mut core::str::SplitAsciiWhitespace<'_>) -> Result<u64> {
    let title = words
        .next()
        .ok_or(Error("expected 16-hex-digit title ID", 0))?;
    if title.len() != 16 || !title.bytes().all(|b| b.is_ascii_hexdigit()) || words.next().is_some()
    {
        return Err(Error("expected 16-hex-digit title ID", 0));
    }
    u64::from_str_radix(title, 16).map_err(|_| Error("invalid title ID", 0))
}
fn process_list(pids: &mut [u64; 1024]) -> Result<usize> {
    // Installed libnx has no PM enumeration API. The NPDM allows
    // svcGetProcessList, which includes all resident processes.
    let mut count = 0;
    rc(
        unsafe { ffi::svcGetProcessList(&mut count, pids.as_mut_ptr(), pids.len() as u32) },
        "list processes",
    )?;
    if count < 0 || count as usize > pids.len() {
        return Err(Error("process list exceeds capacity", 0));
    }
    Ok(count as usize)
}
struct Info;
impl Drop for Info {
    fn drop(&mut self) {
        unsafe { ffi::pminfoExit() }
    }
}
fn process_info() -> Result<Info> {
    rc(unsafe { ffi::pminfoInitialize() }, "initialize pm:info")?;
    Ok(Info)
}
fn quit_title(title: u64) -> Result<Option<u64>> {
    let mut pids = [0u64; 1024];
    let count = process_list(&mut pids)?;
    let _info = process_info()?;
    for &pid in &pids[..count] {
        let mut program = 0;
        // Processes can exit between enumeration and lookup, as with ps.
        if unsafe { ffi::pminfoGetProgramId(&mut program, pid) } != 0 || program != title {
            continue;
        }
        let mut own_pid = 0;
        rc(
            unsafe { ffi::svcGetProcessId(&mut own_pid, 0xffff8001) },
            "get own pid",
        )?;
        if pid == own_pid {
            return Err(Error("refusing self", 0));
        }
        rc(unsafe { ffi::sa_terminate(pid) }, "terminate process")?;
        return Ok(Some(pid));
    }
    Ok(None)
}
pub(crate) struct Settings;
impl Drop for Settings {
    fn drop(&mut self) {
        unsafe { ffi::setsysExit() }
    }
}
static SLEEP_PLANS: Lock<(i32, i32)> = Lock::new((3, 0));
pub(crate) fn format_sleep(settings: &ffi::SleepSettings) -> &'static str {
    match (settings.handheld == 5, settings.console == 5) {
        (true, true) => "off",
        (false, false) => "on",
        _ => "mixed",
    }
}
fn sleep_settings(state: Option<bool>) -> Result<ffi::SleepSettings> {
    rc(unsafe { ffi::setsysInitialize() }, "initialize set:sys")?;
    let _guard = Settings;
    let mut settings = ffi::SleepSettings::default();
    rc(
        unsafe { ffi::setsysGetSleepSettings(&mut settings) },
        "read sleep settings",
    )?;
    if let Some(on) = state {
        SLEEP_PLANS.with(|plans| {
            if (0..5).contains(&settings.handheld) {
                plans.0 = settings.handheld;
            }
            if (0..5).contains(&settings.console) {
                plans.1 = settings.console;
            }
            settings.handheld = if on { plans.0 } else { 5 };
            settings.console = if on { plans.1 } else { 5 };
        });
        rc(
            unsafe { ffi::setsysSetSleepSettings(&settings) },
            "write sleep settings",
        )?;
        rc(
            unsafe { ffi::setsysGetSleepSettings(&mut settings) },
            "read back sleep settings",
        )?;
    }
    Ok(settings)
}
struct JobState {
    id: u64,
    deadline: u64,
    timed_out: bool,
    install: bool,
}
static JOB_STATE: Lock<JobState> = Lock::new(JobState {
    id: 0,
    deadline: 0,
    timed_out: false,
    install: false,
});
pub extern "C" fn watchdog(_: *mut core::ffi::c_void) {
    loop {
        check_budget();
        sleep(20);
    }
}
pub(crate) fn check_budget() {
    let expired = JOB_STATE.with(|job| {
        if job.deadline == 0 || job.timed_out {
            return None;
        }
        if now_ms() < job.deadline && !crate::budget::expired() {
            return None;
        }
        job.timed_out = true;
        crate::budget::cancel();
        Some((job.id, job.install))
    });
    if let Some((id, install)) = expired {
        log::add("worker: wall-clock budget expired; lane quarantined until return");
        error(
            id,
            if install { "install" } else { "command" },
            Error("wall-clock budget", 110),
        );
    }
}
fn execute(job: Job) {
    JOB_STATE.with(|state| {
        crate::budget::begin();
        *state = JobState {
            id: job.id,
            // Absolute ceiling, independent of byte progress and retry count.
            deadline: now_ms() + 30 * 60 * 1000,
            timed_out: false,
            install: job.line.str().split_ascii_whitespace().next() == Some("install"),
        };
    });
    execute_inner(job);
    JOB_STATE.with(|state| {
        state.deadline = 0;
        crate::budget::end();
    });
}
fn execute_inner(job: Job) {
    let id = job.id;
    let mut words = job.line.str().split_ascii_whitespace();
    let cmd = words.next().unwrap_or("");
    let result = (|| -> Result {
        match cmd {
            "update" => {
                let url = words.next().ok_or(Error("usage: update URL", 0))?;
                if words.next().is_some() {
                    return Err(Error("usage: update URL", 0));
                }
                update(id, url)?;
            }
            "wifi" => crate::wifi::command(
                id,
                job.line
                    .str()
                    .trim_start()
                    .strip_prefix("wifi")
                    .unwrap_or("")
                    .trim(),
            )?,
            "power" => {
                // Always queued; telemetry and failures do not enter the ring log.
                let mut s = Text::<256>::new();
                if words.next().is_some() {
                    let _ = s.push(b"power: error usage: power rc=0");
                } else {
                    match crate::power::read() {
                        Ok(value) => s = value,
                        Err(e) => {
                            let _ = write!(s, "power: error {} rc={}", e.0, e.1);
                        }
                    }
                }
                let _ = reply(id, s.bytes());
            }
            "sleep" => {
                let on = match (words.next(), words.next()) {
                    (Some("on"), None) => true,
                    (Some("off"), None) => false,
                    _ => return Err(Error("usage: sleep <on|off>", 0)),
                };
                let settings = sleep_settings(Some(on))?;
                crate::sleep_guard::cancel();
                let mut s = Text::<64>::new();
                let _ = write!(s, "sleep: ok {}", format_sleep(&settings));
                reply(id, s.bytes())?;
            }
            "set" => {
                let key = words.next().ok_or(Error("usage: set KEY", 0))?;
                if words.next().is_some() {
                    return Err(Error("usage: set KEY", 0));
                }
                let mut s = Text::<256>::new();
                if key == "sleep" {
                    let settings = sleep_settings(None)?;
                    let _ = write!(
                        s,
                        "set: sleep {} handheld={} console={}",
                        format_sleep(&settings),
                        settings.handheld,
                        settings.console
                    );
                } else {
                    let code = match key {
                        "firmware" => 0,
                        "hostname" => 1,
                        "serial" => 2,
                        "battery" => 3,
                        "temperature" => 4,
                        _ => return Err(Error("unknown setting key", 0)),
                    };
                    let mut value = [0u8; 128];
                    rc(
                        unsafe { ffi::sa_setting_read(code, value.as_mut_ptr(), value.len()) },
                        "read setting",
                    )?;
                    let end = value.iter().position(|&b| b == 0).unwrap_or(value.len());
                    // Keep device-provided strings within one protocol line.
                    for b in &mut value[..end] {
                        if b.is_ascii_control() {
                            *b = b' ';
                        }
                    }
                    let value = core::str::from_utf8(&value[..end])
                        .map_err(|_| Error("invalid setting text", 0))?;
                    let _ = write!(s, "set: {} {}", key, value);
                }
                reply(id, s.bytes())?;
            }
            "launch" | "quit" => {
                let title = title_arg(&mut words)?;
                let mut s = Text::<96>::new();
                if cmd == "launch" {
                    let _ = write!(s, "launch: attempt {:016x}", title);
                    log::add(s.str());
                    s = Text::new();
                    let result = unsafe { ffi::sa_request_launch(title) };
                    if result == 0 {
                        let _ = write!(s, "launch: ok {:016x}", title);
                    } else {
                        let _ = write!(s, "launch: error rc={}", result);
                    }
                } else {
                    match quit_title(title) {
                        Ok(Some(pid)) => {
                            let _ = write!(s, "quit: ok {}", pid);
                        }
                        Ok(None) => {
                            let _ = write!(s, "quit: not running");
                        }
                        Err(Error("refusing self", _)) => {
                            let _ = write!(s, "quit: refusing self");
                        }
                        Err(e) => {
                            let _ = write!(s, "quit: error rc={}", e.1);
                        }
                    }
                }
                log::add(s.str());
                reply(id, s.bytes())?;
            }
            "df" => {
                if words.next().is_some() {
                    return Err(Error("usage: df", 0));
                }
                let (mut total, mut free) = (0i64, 0i64);
                rc(
                    unsafe { ffi::sa_sd_space(&mut total, &mut free) },
                    "get SD space",
                )?;
                let mut s = Text::<192>::new();
                let _ = write!(s, "df: sd total={} free={}", total, free);
                for (label, bytes) in [("total", total), ("free", free)] {
                    // Decimal MB/GB, with two fractional digits and no floating point.
                    let (unit, divisor) = if bytes >= 1_000_000_000 {
                        ("GB", 1_000_000_000)
                    } else {
                        ("MB", 1_000_000)
                    };
                    let _ = write!(
                        s,
                        " {}={}.{:02} {}",
                        label,
                        bytes / divisor,
                        (bytes % divisor) * 100 / divisor,
                        unit
                    );
                }
                reply(id, s.bytes())?;
                reply(id, b"df: end")?;
            }
            "ps" => {
                if words.next().is_some() {
                    return Err(Error("usage: ps", 0));
                }
                let mut pids = [0u64; 1024];
                let count = process_list(&mut pids)?;
                let _info = process_info()?;
                for &pid in pids
                    .get(..count as usize)
                    .ok_or(Error("process list exceeds capacity", 0))?
                {
                    let mut program_id = 0;
                    let result = unsafe { ffi::pminfoGetProgramId(&mut program_id, pid) };
                    let mut s = Text::<64>::new();
                    if result == 0 {
                        let _ = write!(s, "ps: {} {:016x}", pid, program_id);
                    } else {
                        let _ = write!(s, "ps: {} ?", pid);
                    }
                    reply(id, s.bytes())?;
                }
                reply(id, b"ps: end")?;
            }
            "kill" => {
                let pid = words
                    .next()
                    .ok_or(Error("usage: kill PID", 0))?
                    .parse::<u64>()
                    .map_err(|_| Error("invalid pid", 0))?;
                if words.next().is_some() {
                    return Err(Error("usage: kill PID", 0));
                }
                let mut own_pid = 0;
                rc(
                    unsafe { ffi::svcGetProcessId(&mut own_pid, 0xffff8001) },
                    "get own pid",
                )?;
                if pid == own_pid {
                    reply(id, b"kill: refusing self")?;
                } else {
                    rc(unsafe { ffi::sa_terminate(pid) }, "terminate process")?;
                    let mut s = Text::<64>::new();
                    let _ = write!(s, "kill: ok {}", pid);
                    reply(id, s.bytes())?;
                }
            }
            "hash" => {
                let input = words.next().ok_or(Error("usage: hash SD-PATH", 0))?;
                if words.next().is_some() {
                    return Err(Error("usage: hash SD-PATH", 0));
                }
                let path = crate::files::path(input, false)?;
                let mut chunk = Buffer::new(4 * 1024 * 1024)?;
                let digest = crate::md5::file(path.str(), &mut chunk, 8 * 1024 * 1024 * 1024)?;
                let mut line = Text::<64>::new();
                let _ = write!(line, "hash: md5 {}", digest.str());
                reply(id, line.bytes())?;
            }
            "install" => {
                let url = words
                    .next()
                    .ok_or(Error("usage: install URL [sd|nand] [md5=HEX]", 0))?;
                let storage = match words.next().unwrap_or("sd") {
                    "sd" => 5,
                    "nand" => 4,
                    _ => return Err(Error("destination must be sd or nand", 0)),
                };
                let expected = words.next().map(crate::md5::expected).transpose()?;
                if words.next().is_some() {
                    return Err(Error("too many arguments", 0));
                }
                let title = match install::install_checked(
                    url,
                    storage,
                    expected.as_ref().map(|v| v.str()),
                    &mut |line| {
                        let _ = reply(id, line.as_bytes());
                    },
                ) {
                    Err(Error("hash mismatch", _)) => return Ok(()),
                    other => other?,
                };
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
        if !JOB_STATE.with(|state| state.timed_out) {
            error(id, cmd, e)
        }
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
                let at = le32(&h[0x70..])? as u64;
                if at + 0x20 > e.size {
                    return Err(Error("invalid NPDM ACI offset", 0));
                }
                let mut aci = [0; 0x20];
                f.read(e.off + at, &mut aci)?;
                npdm = &aci[..4] == b"ACI0" && le64(&aci[0x10..])? == 0x430000000000000b;
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
    // Release server first, then client, before asking PM to terminate us.
    let listener = LISTENER.swap(-1, AcqRel);
    if listener >= 0 {
        unsafe {
            ffi::shutdown(listener, 2);
            ffi::close(listener);
        }
    }
    let _ = reply(id, b"update: ok");
    disconnect(id);
    sleep(500);
    loop {
        let mut pid = 0;
        // libnx CUR_PROCESS_HANDLE: query our PID, never the replacement's PID.
        let mut rc = unsafe { ffi::svcGetProcessId(&mut pid, 0xffff8001) };
        if rc == 0 {
            let mut line = Text::<192>::new();
            let _ = write!(line, "update: retiring, terminating self pid={pid}");
            log::add(line.str());
            rc = unsafe { ffi::sa_terminate(pid) };
        }
        // A successful self-termination does not return. Retry PM failures
        // rather than leaving another permanently resident retired instance.
        let mut line = Text::<192>::new();
        let _ = write!(line, "update: self termination returned rc={rc}; retrying");
        log::add(line.str());
        sleep(1000);
    }
}
fn disconnect(id: u64) {
    session(id).with(|s| {
        if s.id == id && s.fd >= 0 {
            unsafe {
                ffi::shutdown(s.fd, 2);
                ffi::close(s.fd);
            }
            s.fd = -1;
        }
    })
}
// Pure formatter; u128 arithmetic keeps percentages safe for every u64 input.
pub(crate) fn format_status(phase: u8, rx: u64, total: u64) -> Text<128> {
    let name = match phase {
        1 => "downloading",
        2 => "writing",
        3 => "done",
        4 => "error",
        _ => "idle",
    };
    let mut s = Text::new();
    let _ = write!(s, "status: {} rx={} total={}", name, rx, total);
    if total > 0 {
        let tenths = rx as u128 * 1000 / total as u128;
        let _ = write!(s, " pct={}.{}", tenths / 10, tenths % 10);
    }
    s
}
fn receive(id: u64, line: Text<512>) {
    let mut words = line.str().split_ascii_whitespace();
    match words.next() {
        Some("ping") if words.next().is_none() => {
            let _ = reply(id, b"pong");
        }
        Some("status") if words.next().is_none() => {
            let (phase, rx, total) = install::progress();
            let s = format_status(phase, rx, total);
            let _ = reply(id, s.bytes());
        }
        Some("version") if words.next().is_none() => {
            let _ = reply(id, b"swagent-1.16-rs");
        }
        _ => enqueue(id, line),
    }
}
fn file_reply(id: u64, cmd: &str, path: &str, body: &[u8]) {
    let _ = session(id).with(|s| {
        if s.id != id || s.fd < 0 {
            return Err(Error("disconnected", 0));
        }
        let sock = core::mem::ManuallyDrop::new(Socket::new(s.fd));
        crate::files::run(&crate::files::Sd, cmd, path, body, &mut |b| sock.send(b))
    });
}
const PUT_LIMIT: u64 = 268435456;
struct Upload {
    path: Text<768>,
    file: Option<File>,
    bytes: [u8; 512],
    buffered: usize,
    used: u64,
    size: u64,
    complete: bool,
}
impl Upload {
    fn push(&mut self, byte: u8) -> Result<bool> {
        self.bytes[self.buffered] = byte;
        self.buffered += 1;
        self.used += 1;
        if self.buffered == self.bytes.len() || self.used == self.size {
            self.file.as_mut().unwrap().write(
                self.used - self.buffered as u64,
                &self.bytes[..self.buffered],
            )?;
            self.buffered = 0;
        }
        if self.used == self.size {
            self.file.as_mut().unwrap().flush()?;
            self.file.take();
            self.complete = true;
        }
        Ok(self.complete)
    }
}
impl Drop for Upload {
    fn drop(&mut self) {
        self.file.take();
        if !self.complete {
            io::remove(self.path.str());
        }
    }
}
fn put_size(input: &str) -> Result<u64> {
    let n = input.parse::<u64>().map_err(|_| Error("usage", 0))?;
    if n > PUT_LIMIT {
        return Err(Error("size limit", PUT_LIMIT as u32));
    }
    Ok(n)
}
fn file_error(id: u64, cmd: &str, e: Error) {
    let mut msg = Text::<128>::new();
    let _ = write!(msg, "{}: error {}", cmd, e.0);
    if e.0 == "size limit" {
        let _ = write!(msg, " {}", e.1);
    }
    let _ = reply(id, msg.bytes());
}
fn file_line(id: u64, line: &str) -> Result<Option<Upload>> {
    let mut words = line.split_ascii_whitespace();
    let cmd = words.next().unwrap_or("");
    let Some(path) = words.next() else {
        return Err(Error("usage", 0));
    };
    if cmd == "put" {
        // Invalid uploads close the connection: their unconsumed body must never
        // be interpreted as commands.
        let n = put_size(words.next().ok_or(Error("usage", 0))?)?;
        if words.next().is_some() {
            return Err(Error("usage", 0));
        }
        let p = crate::files::path(path, true)?;
        let mut upload = Upload {
            file: Some(File::open(p.str(), true)?),
            path: p,
            bytes: [0; 512],
            buffered: 0,
            used: 0,
            size: n,
            complete: false,
        };
        if n == 0 {
            upload.file.as_mut().unwrap().flush()?;
            upload.file.take();
            upload.complete = true;
            reply(id, b"put: ok 0")?;
            return Ok(None);
        }
        Ok(Some(upload))
    } else {
        if words.next().is_some() {
            return Err(Error("usage", 0));
        }
        file_reply(id, cmd, path, &[]);
        Ok(None)
    }
}
// Persistent client lanes: no per-connection allocation or unreaped threads.
// The accept thread never reads commands or touches a file.
pub extern "C" fn client_worker(arg: *mut core::ffi::c_void) {
    let slot = arg as usize;
    loop {
        if !CLIENT_READY[slot].load(Acquire) {
            sleep(5);
            continue;
        }
        let (fd, id) = SESSIONS[slot].with(|s| (s.fd, s.id));
        client_loop(fd, id);
        CLIENT_READY[slot].store(false, Release);
    }
}
fn client_loop(fd: i32, id: u64) {
    let mut frame = Frame::new();
    let mut upload: Option<Upload> = None;
    let mut last_input = now_ms();
    'client: loop {
        if upload.is_some() && now_ms() - last_input >= 30_000 {
            let _ = reply(id, b"put: error timeout/short read");
            break;
        }
        let mut input = [0; 512];
        // Every body receive is bounded, independently of header framing.
        unsafe {
            ffi::sa_timeout(fd, if upload.is_some() { 30 } else { 2 });
        }
        let n = unsafe { ffi::sa_recv(fd, input.as_mut_ptr(), input.len(), 0) };
        if n == 0 {
            if upload.is_some() {
                let _ = reply(id, b"put: error timeout/short read");
            }
            break;
        }
        if crate::net::pending() {
            break;
        }
        if n < 0 {
            let e = unsafe { ffi::sa_errno() };
            if crate::net::transient(e) {
                sleep(10);
                continue;
            }
            log::add("client: receive failed");
            break;
        }
        let Some(input) = input.get(..n as usize) else {
            break;
        };
        last_input = now_ms();
        for byte in input {
            if let Some(u) = upload.as_mut() {
                match u.push(*byte) {
                    Ok(true) => {
                        let mut msg = Text::<64>::new();
                        let _ = write!(msg, "put: ok {}", u.used);
                        let _ = reply(id, msg.bytes());
                        upload = None;
                    }
                    Ok(false) => {}
                    Err(e) => {
                        file_error(id, "put", e);
                        break 'client;
                    }
                }
                continue;
            }
            if let Some(result) = frame.feed(*byte) {
                match result {
                    Ok(line) if line.len > 0 => {
                        let cmd = line.str().split_ascii_whitespace().next().unwrap_or("");
                        if matches!(cmd, "put" | "cat" | "ls" | "mkdir") {
                            match file_line(id, line.str()) {
                                Ok(u) => upload = u,
                                Err(e) => {
                                    file_error(id, cmd, e);
                                    if cmd == "put" {
                                        break 'client;
                                    }
                                }
                            }
                        } else {
                            receive(id, line);
                        }
                    }
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
    drop(upload); // Close and delete partial uploads before accepting another client.
    disconnect(id);
    log::add("client disconnected");
}
fn admit(fd: i32) -> bool {
    for slot in 0..CLIENTS {
        if CLIENT_READY[slot].load(Acquire) {
            continue;
        }
        let id = NEXT_ID.fetch_add(CLIENTS as u64, AcqRel) + slot as u64;
        SESSIONS[slot].with(|s| {
            s.fd = fd;
            s.id = id;
        });
        CLIENT_READY[slot].store(true, Release);
        log::add("client connected");
        return true;
    }
    false
}
fn listener_step(server: i32) -> bool {
    let fd = unsafe { ffi::sa_accept(server) };
    if fd < 0 {
        let e = unsafe { ffi::sa_errno() };
        if crate::net::transient(e) {
            sleep(20);
            return true;
        }
        let mut line = Text::<96>::new();
        let _ = write!(line, "listener: accept failed errno={e}; recreating");
        log::add(line.str());
        return false;
    }
    unsafe {
        ffi::sa_timeout(fd, 2);
    }
    if !admit(fd) {
        // Capacity is fixed. Do not wait behind an occupied client lane.
        unsafe {
            ffi::sa_send(fd, b"error: busy\r\n".as_ptr(), 13, 0);
            ffi::close(fd);
        }
        log::add("listener: client capacity reached");
    }
    true
}
fn serve_step(server: &mut i32) {
    if *server < 0 {
        *server = unsafe { ffi::sa_listen() };
        if *server < 0 {
            let mut line = Text::<96>::new();
            let _ = write!(line, "listener: listen failed errno={}; retrying", -*server);
            log::add(line.str());
            sleep(250);
            return;
        }
        LISTENER.store(*server, Release);
        log::add("listening :6000");
    }
    if !listener_step(*server) {
        LISTENER.store(-1, Release);
        unsafe {
            ffi::close(*server);
        }
        *server = -1;
        log::add("listener: closed; recreating");
        sleep(100);
    }
}
pub fn serve() -> ! {
    let mut server = -1;
    loop {
        if retiring() {
            forever()
        }
        serve_step(&mut server);
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
    let id = 42;
    session(id).with(|s| {
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

#[cfg(test)]
pub fn test_install_error(e: Error) {
    error(42, "install", e);
}

#[cfg(test)]
mod upload_tests {
    use super::*;
    #[test]
    fn rejects_oversized_put_before_open_or_allocation() {
        assert_eq!(put_size("268435456"), Ok(PUT_LIMIT));
        for n in ["268435457", "2147483648", "18446744073709551615"] {
            assert!(matches!(
                file_line(0, &std::format!("put switch/test {n}")),
                Err(Error("size limit", 268435456))
            ));
        }
    }
    #[test]
    fn partial_upload_is_deleted() {
        let path =
            std::env::temp_dir().join(std::format!("swagent-partial-{}", std::process::id()));
        let mut p = Text::new();
        p.push(path.to_str().unwrap().as_bytes()).unwrap();
        let mut u = Upload {
            file: Some(File::open(p.str(), true).unwrap()),
            path: p,
            bytes: [0; 512],
            buffered: 0,
            used: 0,
            size: 1024,
            complete: false,
        };
        for _ in 0..512 {
            assert!(!u.push(1).unwrap());
        }
        assert!(path.exists());
        drop(u);
        assert!(!path.exists());
    }
}

#[cfg(test)]
pub fn test_file_line(line: &str) {
    if let Err(e) = file_line(42, line) {
        file_error(42, "put", e);
    }
}

#[cfg(test)]
pub fn test_listener_step(fd: i32) -> bool {
    listener_step(fd)
}
#[cfg(test)]
pub fn test_client_once(slot: usize) {
    assert!(CLIENT_READY[slot].load(Acquire));
    let (fd, id) = SESSIONS[slot].with(|s| (s.fd, s.id));
    client_loop(fd, id);
    CLIENT_READY[slot].store(false, Release);
}

#[cfg(test)]
pub fn test_serve_step(server: &mut i32) {
    serve_step(server);
}
