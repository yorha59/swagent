//! Bounded-memory PFS0 export. Backend owns IPC handles; no index lock spans IO.
use crate::{ffi, io::File, util::*};
use core::{fmt::Write, sync::atomic::Ordering::*};
const CHUNK: usize = 4 * 1024 * 1024;
const MAX: usize = 1026;
#[derive(Clone, Copy)]
pub struct Item {
    pub name: Text<129>,
    pub size: u64,
    pub id: [u8; 16],
}
const EMPTY: Item = Item {
    name: Text::new(),
    size: 0,
    id: [0; 16],
};
pub struct Catalog {
    pub items: Buffer,
    pub count: usize,
}
impl Catalog {
    pub fn new() -> Result<Self> {
        Ok(Self {
            items: Buffer::new(MAX * core::mem::size_of::<Item>())?,
            count: 0,
        })
    }
    pub fn push(&mut self, item: Item) -> Result {
        if self.count >= MAX || item.size == 0 {
            return Err(Error("dump catalog bounds", 0));
        }
        if self
            .entries()
            .iter()
            .any(|e| e.name.str() == item.name.str())
        {
            return Err(Error("duplicate dump name", 0));
        }
        // Buffer is malloc-aligned; Item is Copy and each slot is initialized before use.
        unsafe {
            self.items
                .as_mut_ptr()
                .cast::<Item>()
                .add(self.count)
                .write(item);
        }
        self.count += 1;
        Ok(())
    }
    pub fn entries(&self) -> &[Item] {
        unsafe { core::slice::from_raw_parts(self.items.as_ptr().cast(), self.count) }
    }
}
/// NCM lookup, ContentStorage reads and ES ticket access are mockable together.
pub trait Backend {
    fn catalog(&mut self, title: u64, out: &mut Catalog) -> Result<bool>;
    fn read(&mut self, item: &Item, offset: u64, bytes: &mut [u8]) -> Result;
}
pub fn path(title: u64, output: Option<&str>) -> Result<Text<768>> {
    let mut default = Text::<128>::new();
    write!(default, "sdmc:/switch/dump/{title:016x}.nsp").map_err(|_| Error("dump path", 0))?;
    let p = crate::files::path(output.unwrap_or(default.str()), true)?;
    if !p.str().starts_with("sdmc:/switch/") || p.str().ends_with('/') {
        return Err(Error("path not allowed", 0));
    }
    Ok(p)
}
/// Preserve caller-selected string padding and entry order, including fixture XML.
pub fn header(items: &[Item], padding: usize) -> Result<Buffer> {
    if items.is_empty() || items.len() > MAX || padding > 4096 {
        return Err(Error("dump header bounds", 0));
    }
    let strings = items.iter().try_fold(padding, |n, e| {
        n.checked_add(e.name.len + 1)
            .ok_or(Error("dump strings overflow", 0))
    })?;
    let mut h = Buffer::new(16 + items.len() * 24 + strings)?;
    h.fill(0);
    h[..4].copy_from_slice(b"PFS0");
    h[4..8].copy_from_slice(&(items.len() as u32).to_le_bytes());
    h[8..12].copy_from_slice(&(strings as u32).to_le_bytes());
    let (mut off, mut name) = (0u64, 0usize);
    for (i, e) in items.iter().enumerate() {
        if e.name.len == 0
            || e.name
                .bytes()
                .iter()
                .any(|b| *b < 32 || *b == b'/' || *b == b'\\')
        {
            return Err(Error("invalid dump name", 0));
        }
        let at = 16 + i * 24;
        h[at..at + 8].copy_from_slice(&off.to_le_bytes());
        h[at + 8..at + 16].copy_from_slice(&e.size.to_le_bytes());
        h[at + 16..at + 20].copy_from_slice(&(name as u32).to_le_bytes());
        let pos = 16 + items.len() * 24 + name;
        h[pos..pos + e.name.len].copy_from_slice(e.name.bytes());
        name += e.name.len + 1;
        off = off
            .checked_add(e.size)
            .filter(|n| *n <= i64::MAX as u64)
            .ok_or(Error("dump size overflow", 0))?;
    }
    Ok(h)
}
pub fn pack(
    backend: &mut dyn Backend,
    items: &[Item],
    padding: usize,
    out: &mut File,
    emit: &mut dyn FnMut(&str),
) -> Result<u64> {
    let h = header(items, padding)?;
    let total = items.iter().try_fold(h.len() as u64, |n, e| {
        n.checked_add(e.size).ok_or(Error("dump size overflow", 0))
    })?;
    if total > i64::MAX as u64 {
        return Err(Error("dump size overflow", 0));
    }
    crate::install::INSTALL_TOTAL_BYTES.store(total, Relaxed);
    out.write(0, &h)?;
    let mut offset = h.len() as u64;
    let mut chunk = Buffer::new(CHUNK)?;
    for item in items {
        let mut pos = 0;
        while pos < item.size {
            crate::budget::check()?;
            let n = (item.size - pos).min(CHUNK as u64) as usize;
            backend.read(item, pos, &mut chunk[..n])?;
            out.write(offset, &chunk[..n])?;
            pos += n as u64;
            offset += n as u64;
            crate::budget::progress()?;
            crate::install::INSTALL_RX_BYTES.store(offset, Relaxed);
            emit(crate::protocol::format_status(2, offset, total).str());
        }
    }
    out.flush()?;
    Ok(total)
}
pub fn run(
    backend: &mut dyn Backend,
    title: u64,
    output: Option<&str>,
    emit: &mut dyn FnMut(&str),
) -> Result {
    let p = path(title, output)?;
    crate::install::INSTALL_RX_BYTES.store(0, Relaxed);
    crate::install::INSTALL_TOTAL_BYTES.store(0, Relaxed);
    crate::install::INSTALL_PHASE.store(1, Release);
    let result: Result = (|| {
        let mut catalog = Catalog::new()?;
        let notik = backend.catalog(title, &mut catalog)?;
        if notik {
            crate::log::add("dump: warning no ticket (notik)");
            emit("dump: warning notik");
        }
        // Create only the requested SD parents. No filesystem operation holds a lock.
        for (i, b) in p.bytes().iter().enumerate() {
            if *b == b'/' && i > 6 {
                let mut parent = Text::<768>::new();
                parent.push(&p.bytes()[..i])?;
                rc(unsafe { ffi::sa_dump_mkdir(parent.c()) }, "dump mkdir")?;
            }
        }
        struct Partial<'a>(&'a str, bool);
        impl Drop for Partial<'_> {
            fn drop(&mut self) {
                if !self.1 {
                    let _ = crate::io::remove(self.0);
                }
            }
        }
        let mut cleanup = Partial(p.str(), false);
        let mut out = File::open(p.str(), true)?;
        crate::install::INSTALL_PHASE.store(2, Release);
        let total = pack(backend, catalog.entries(), 0, &mut out, emit)?;
        drop(out);
        cleanup.1 = true;
        let mut line = Text::<1024>::new();
        write!(
            line,
            "dump: ok {title:016x} {} bytes={}{}",
            p.str(),
            total,
            if notik { " notik" } else { "" }
        )
        .map_err(|_| Error("dump result", 0))?;
        crate::log::add(line.str());
        emit(line.str());
        Ok(())
    })();
    crate::install::INSTALL_PHASE.store(if result.is_ok() { 3 } else { 4 }, Release);
    if let Err(e) = result {
        let mut line = Text::<192>::new();
        let _ = write!(line, "dump: error {} rc={}", e.0, e.1);
        crate::log::add(line.str());
    }
    result
}
struct Native {
    storage: u32,
    ticket: Buffer,
    cert: Buffer,
    rights: [u8; 16],
}
impl Native {
    fn new() -> Result<Self> {
        Ok(Self {
            storage: 0,
            ticket: Buffer::new(0x4000)?,
            cert: Buffer::new(0x4000)?,
            rights: [0; 16],
        })
    }
}
impl Backend for Native {
    fn catalog(&mut self, title: u64, out: &mut Catalog) -> Result<bool> {
        let mut infos = Buffer::new(MAX * 24)?;
        let mut count = 0u32;
        rc(
            unsafe {
                ffi::sa_dump_meta(
                    title,
                    &mut self.storage,
                    infos.as_mut_ptr(),
                    MAX as u32,
                    &mut count,
                )
            },
            "dump NCM metadata",
        )?;
        if count == 0 || count as usize >= MAX {
            return Err(Error("dump content count", 0));
        }
        // ContentMeta must lead the exported archive, regardless of NCM ordering.
        if let Some(index) = infos[..count as usize * 24]
            .chunks_exact(24)
            .position(|info| info[22] == 0)
        {
            for j in 0..24 {
                infos.swap(j, index * 24 + j);
            }
        }
        let mut meta = 0;
        for info in infos[..count as usize * 24].chunks_exact(24) {
            let mut item = EMPTY;
            item.id.copy_from_slice(&info[..16]);
            item.size = le32(&info[16..])? as u64 | ((info[20] as u64) << 32);
            item.name.push(hex(&item.id).bytes())?;
            item.name.push(if info[22] == 0 {
                meta += 1;
                b".cnmt.nca"
            } else {
                b".nca"
            })?;
            out.push(item)?;
        }
        if meta != 1 {
            return Err(Error("dump requires one CNMT NCA", 0));
        }
        let cnca = out
            .entries()
            .iter()
            .find(|e| e.name.str().ends_with(".cnmt.nca"))
            .ok_or(Error("missing CNMT", 0))?;
        // Reuse install's SYSTEM staging adapters: @Sdcard is not a content mount.
        let (mut staged_file, mut staged_fs) = (ffi::Service::default(), ffi::Service::default());
        rc(
            unsafe {
                ffi::sa_stage_open(
                    cnca.name.c(),
                    cnca.size as i64,
                    &mut staged_file,
                    &mut staged_fs,
                )
            },
            "dump stage CNMT",
        )?;
        struct Stage(ffi::Service);
        impl Drop for Stage {
            fn drop(&mut self) {
                unsafe {
                    ffi::sa_stage_close(&mut self.0);
                }
            }
        }
        let mut stage = Stage(staged_fs);
        {
            let mut staged = File(staged_file);
            let mut buf = Buffer::new(CHUNK)?;
            let mut pos = 0;
            while pos < cnca.size {
                let n = (cnca.size - pos).min(CHUNK as u64) as usize;
                self.read(cnca, pos, &mut buf[..n])?;
                staged.write(pos, &buf[..n])?;
                pos += n as u64;
            }
            staged.flush()?;
        }
        rc(
            unsafe { ffi::sa_stage_commit(&mut stage.0) },
            "dump commit CNMT",
        )?;
        let mut content_path = Text::<256>::new();
        content_path.push(b"@SystemContent://swagent-install/")?;
        content_path.push(cnca.name.bytes())?;
        let (mut file, mut fs) = (ffi::Service::default(), ffi::Service::default());
        rc(
            unsafe { ffi::sa_cnmt_open(content_path.c(), &mut file, &mut fs) },
            "dump open CNMT",
        )?;
        struct Mount(ffi::Service);
        impl Drop for Mount {
            fn drop(&mut self) {
                unsafe {
                    ffi::fsFsClose(&mut self.0);
                }
            }
        }
        let mount = Mount(fs);
        let mut file = File(file);
        let size = file.size()?;
        if !(32..=512 * 1024).contains(&size) {
            return Err(Error("dump CNMT size", 0));
        }
        let mut data = Buffer::new(size as usize)?;
        file.read(0, &mut data)?;
        let m = crate::install::meta(&data)?;
        if m.key.id != title || !matches!(m.key.kind, 0x80 | 0x81) {
            return Err(Error("dump CNMT title mismatch", 0));
        }
        for record in m.contents.chunks_exact(56) {
            let info = &record[32..];
            if info[22] == 6 {
                continue;
            } // Installed updates need not retain delta fragments.
            let size = le32(&info[16..])? as u64 | ((info[20] as u64) << 32);
            if !out
                .entries()
                .iter()
                .any(|e| e.id == info[..16] && e.size == size)
            {
                return Err(Error("dump CNMT content mismatch", 0));
            }
        }
        drop(file);
        drop(mount);
        drop(stage);
        let (mut tn, mut cn) = (0usize, 0usize);
        rc(
            unsafe {
                ffi::sa_dump_ticket(
                    self.storage,
                    infos.as_ptr(),
                    count,
                    self.rights.as_mut_ptr(),
                    self.ticket.as_mut_ptr(),
                    self.ticket.len(),
                    &mut tn,
                    self.cert.as_mut_ptr(),
                    self.cert.len(),
                    &mut cn,
                )
            },
            "dump ES ticket/cert",
        )?;
        if tn == 0 {
            return Ok(true);
        }
        if tn > self.ticket.len() || cn == 0 || cn > self.cert.len() {
            return Err(Error("dump ticket bounds", 0));
        }
        for (suffix, size) in [(".tik", tn), (".cert", cn)] {
            let mut item = EMPTY;
            item.id = self.rights;
            item.name.push(hex(&self.rights).bytes())?;
            item.name.push(suffix.as_bytes())?;
            item.size = size as u64;
            out.push(item)?;
        }
        Ok(false)
    }
    fn read(&mut self, item: &Item, offset: u64, bytes: &mut [u8]) -> Result {
        let blob = if item.name.str().ends_with(".tik") {
            Some(&self.ticket)
        } else if item.name.str().ends_with(".cert") {
            Some(&self.cert)
        } else {
            None
        };
        if let Some(blob) = blob {
            let start = usize::try_from(offset).map_err(|_| Error("dump blob offset", 0))?;
            let end = start
                .checked_add(bytes.len())
                .ok_or(Error("dump blob bounds", 0))?;
            bytes.copy_from_slice(blob.get(start..end).ok_or(Error("dump blob bounds", 0))?);
            return Ok(());
        }
        rc(
            unsafe {
                ffi::sa_dump_read(
                    self.storage,
                    item.id.as_ptr(),
                    offset,
                    bytes.as_mut_ptr(),
                    bytes.len(),
                )
            },
            "dump ContentStorage read",
        )
    }
}
pub fn command(title: u64, output: Option<&str>, emit: &mut dyn FnMut(&str)) -> Result {
    #[cfg(test)]
    if let Some(mut backend) = TEST_BACKEND.with(|b| b.borrow_mut().take()) {
        return run(backend.as_mut(), title, output, emit);
    }
    let result = (|| {
        let mut backend = Native::new()?;
        crate::sleep_guard::begin()?;
        run(&mut backend, title, output, emit)
    })();
    crate::sleep_guard::restore();
    if result.is_err() {
        crate::install::INSTALL_PHASE.store(4, Release);
    }
    result
}

#[cfg(test)]
std::thread_local! { static TEST_BACKEND: std::cell::RefCell<Option<std::boxed::Box<dyn Backend>>> = std::cell::RefCell::new(None); }
#[cfg(test)]
pub fn test_backend(backend: std::boxed::Box<dyn Backend>) {
    TEST_BACKEND.with(|b| *b.borrow_mut() = Some(backend));
}

// Native enumeration also completes bounded units of work between IPC calls.
#[no_mangle]
pub extern "C" fn agent_dump_progress() -> u32 {
    match crate::budget::progress() {
        Ok(()) => 0,
        Err(_) => 110,
    }
}
