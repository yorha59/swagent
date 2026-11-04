use crate::{
    ffi::{self, Key, Record, Service},
    http,
    io::{self, File},
    util::*,
};
// PFS0 entries are u64 offset, u64 size, u32 name offset, u32 reserved.
const PFS_ENTRY_SIZE: usize = 24;
const NSP: &str = "sdmc:/switch/.install.tmp.nsp";
const CNCA: &str = "sdmc:/switch/.install.tmp.cnmt.nca";
const CHUNK: usize = 4 * 1024 * 1024;
const META_CAP: usize = 512 * 1024;
pub struct Entry<'a> {
    pub name: &'a str,
    pub off: u64,
    pub size: u64,
}
pub struct Pfs {
    header: Buffer,
    pub count: usize,
    pub base: u64,
}
impl Pfs {
    pub fn open(f: &mut File) -> Result<Self> {
        let mut h = [0; 16];
        f.read(0, &mut h)?;
        let count = le32(&h[4..]) as usize;
        let strings = le32(&h[8..]) as usize;
        if &h[..4] != b"PFS0" || count == 0 || count > 2048 {
            return Err(Error("invalid PFS0 header", 0));
        }
        let n = 16 + PFS_ENTRY_SIZE * count + strings;
        let size = f.size()?;
        if n > 256 * 1024 || n as u64 >= size {
            return Err(Error("PFS0 header size", 0));
        }
        let mut header = Buffer::new(n)?;
        f.read(0, &mut header)?;
        let p = Self {
            header,
            count,
            base: n as u64,
        };
        p.validate(size)?;
        Ok(p)
    }
    pub fn entry(&self, i: usize) -> Result<Entry<'_>> {
        entry(&self.header, self.count, self.base, i)
    }
    pub fn validate(&self, size: u64) -> Result {
        for i in 0..self.count {
            let e = self.entry(i)?;
            if e.off.checked_add(e.size).is_none_or(|end| end > size) {
                return Err(Error("PFS0 entry outside file", 0));
            }
            for j in 0..i {
                let prev = self.entry(j)?;
                if prev.name.eq_ignore_ascii_case(e.name)
                    || e.off < prev.off + prev.size && prev.off < e.off + e.size
                {
                    return Err(Error("duplicate or overlapping PFS0 entries", 0));
                }
            }
        }
        Ok(())
    }
    fn find(&self, name: &str) -> Result<Entry<'_>> {
        for i in 0..self.count {
            let e = self.entry(i)?;
            if e.name.eq_ignore_ascii_case(name) {
                return Ok(e);
            }
        }
        Err(Error("missing NCA", 0))
    }
}
pub fn entry(header: &[u8], count: usize, base: u64, i: usize) -> Result<Entry<'_>> {
    if i >= count || 16 + PFS_ENTRY_SIZE * count > header.len() {
        return Err(Error("PFS0 table bounds", 0));
    }
    let p = &header[16 + i * PFS_ENTRY_SIZE..16 + (i + 1) * PFS_ENTRY_SIZE];
    let strings = &header[16 + count * PFS_ENTRY_SIZE..];
    let name = le32(&p[16..]) as usize;
    let rest = strings.get(name..).ok_or(Error("PFS0 name offset", 0))?;
    let end = rest
        .iter()
        .position(|b| *b == 0)
        .ok_or(Error("unterminated PFS0 name", 0))?;
    if end == 0
        || end > 128
        || rest[..end]
            .iter()
            .any(|c| *c < 32 || *c == b'/' || *c == b'\\')
    {
        return Err(Error("invalid PFS0 name", 0));
    }
    Ok(Entry {
        name: core::str::from_utf8(&rest[..end]).map_err(|_| Error("PFS0 filename encoding", 0))?,
        off: base
            .checked_add(le64(p))
            .ok_or(Error("PFS0 offset overflow", 0))?,
        size: le64(&p[8..]),
    })
}
struct Cleanup;
impl Drop for Cleanup {
    fn drop(&mut self) {
        io::remove(NSP);
        io::remove(CNCA)
    }
}
struct Stage(Service);
impl Drop for Stage {
    fn drop(&mut self) {
        unsafe { ffi::sa_stage_close(&mut self.0) }
    }
}
struct Mounted(Service);
impl Drop for Mounted {
    fn drop(&mut self) {
        unsafe { ffi::fsFsClose(&mut self.0) }
    }
}
struct Ncm;
impl Drop for Ncm {
    fn drop(&mut self) {
        unsafe { ffi::ncmExit() }
    }
}
struct Ns;
impl Drop for Ns {
    fn drop(&mut self) {
        unsafe { ffi::nsExit() }
    }
}
struct Storage(Service);
impl Drop for Storage {
    fn drop(&mut self) {
        unsafe { ffi::ncmContentStorageClose(&mut self.0) }
    }
}
struct Database(Service);
impl Drop for Database {
    fn drop(&mut self) {
        unsafe { ffi::ncmContentMetaDatabaseClose(&mut self.0) }
    }
}
struct Placeholder<'a> {
    storage: &'a mut Service,
    id: [u8; 16],
}
impl Drop for Placeholder<'_> {
    fn drop(&mut self) {
        unsafe {
            ffi::ncmContentStorageDeletePlaceHolder(self.storage, self.id.as_ptr());
        }
    }
}
fn ticket(f: &mut File, p: &Pfs) -> Result {
    for i in 0..p.count {
        let t = p.entry(i)?;
        if !t.name.ends_with(".tik") {
            continue;
        }
        if t.size < 4 || t.size > 0x1000 {
            return Err(Error("ticket size", 0));
        }
        let mut tik = Buffer::new(t.size as usize)?;
        f.read(t.off, &mut tik)?;
        let sig = le32(&tik[..4]);
        let body = 4 + match sig {
            0x10000 | 0x10003 => 0x23c,
            0x10001 | 0x10004 => 0x13c,
            0x10002 | 0x10005 => 0x7c,
            0x10006 => 0x3c,
            _ => {
                let mut warning = Text::<192>::new();
                warning.push(b"install: skipping invalid ticket (sig=0x")?;
                warning.push(hex(&sig.to_be_bytes()).bytes())?;
                warning.push(b")")?;
                crate::log::add(warning.str());
                continue;
            }
        };
        if body + 0x180 > tik.len() {
            return Err(Error("truncated ticket", 0));
        }
        let name = t.name.strip_suffix(".tik").unwrap_or("");
        let mut cert = Text::<160>::new();
        cert.push(name.as_bytes())?;
        cert.push(b".cert")?;
        let c = p
            .find(cert.str())
            .map_err(|_| Error("ticket requires matching certificate", 0))?;
        if c.size == 0 || c.size > 0x4000 {
            return Err(Error("certificate size", 0));
        }
        let mut certificate = Buffer::new(c.size as usize)?;
        f.read(c.off, &mut certificate)?;
        tik[body + 0x146] &= !0x10;
        let result = unsafe {
            ffi::sa_ticket(
                tik.as_ptr(),
                tik.len(),
                certificate.as_ptr(),
                certificate.len(),
            )
        };
        if result != 0 {
            let mut warning = Text::<192>::new();
            warning.push(b"install: ticket import failed (rc=0x")?;
            warning.push(hex(&result.to_be_bytes()).bytes())?;
            warning.push(b"); continuing")?;
            crate::log::add(warning.str());
        }
    }
    Ok(())
}
pub struct Meta<'a> {
    pub key: Key,
    pub ext: &'a [u8],
    pub contents: &'a [u8],
    pub metas: &'a [u8],
    pub attributes: u8,
    pub extended: usize,
}
pub fn meta(data: &[u8]) -> Result<Meta<'_>> {
    if data.len() < 32 {
        return Err(Error("short CNMT", 0));
    }
    let kind = data[12];
    let ext = le16(&data[14..]) as usize;
    let count = le16(&data[16..]) as usize;
    let metas = le16(&data[18..]) as usize;
    let expected = match kind {
        0x80 => ext == 16,
        0x81 => ext == 24,
        0x82 => ext == 16 || ext == 24,
        _ => false,
    };
    if !expected || count == 0 || count > 1024 || metas > 256 {
        return Err(Error("unsupported CNMT type or bounds", 0));
    }
    let content_end = 32 + ext + count * 56;
    let meta_end = content_end + metas * 16;
    if meta_end > data.len() {
        return Err(Error("truncated CNMT tables", 0));
    }
    let extended = if kind == 0x81 {
        le32(&data[32 + 12..]) as usize
    } else {
        0
    };
    if extended > META_CAP
        || meta_end
            .checked_add(extended + 32)
            .is_none_or(|n| n > data.len())
    {
        return Err(Error("CNMT extended data bounds", 0));
    }
    Ok(Meta {
        key: Key {
            id: le64(data),
            version: le32(&data[8..]),
            kind,
            install: 0,
            pad: [0; 2],
        },
        ext: &data[32..32 + ext],
        contents: &data[32 + ext..content_end],
        metas: &data[content_end..meta_end],
        attributes: data[20],
        extended,
    })
}
fn content_name(info: &[u8]) -> Result<Text<80>> {
    let mut name = hex(&info[..16]);
    name.push(if info[22] == 0 { b".cnmt.nca" } else { b".nca" })?;
    Ok(name)
}
fn content_size(info: &[u8]) -> u64 {
    le32(&info[16..]) as u64 | ((info[20] as u64) << 32)
}
fn records(key: Key, storage: u32) -> Result {
    rc(unsafe { ffi::nsInitialize() }, "ns initialize")?;
    let _ns = Ns;
    let app = match key.kind {
        0x81 => key.id ^ 0x800,
        0x82 => (key.id ^ 0x1000) & !0xfff,
        _ => key.id,
    };
    let mut count = 0;
    let count_rc = unsafe { ffi::nsCountApplicationContentMeta(app, &mut count) };
    // 0x410 = ns ApplicationIdNotFound. Other IPC failures must not erase records.
    if count_rc != 0 && count_rc != 0x410 {
        return Err(Error("ns CountApplicationContentMeta", count_rc));
    }
    if count < 0 || count > 255 {
        return Err(Error("too many application records", 0));
    }
    let mut records = [Record::default(); 256];
    let mut written = 0;
    if count > 0 {
        let mut input = [0; 16];
        input[8..].copy_from_slice(&app.to_le_bytes());
        rc(
            unsafe {
                ffi::sa_ns(
                    17,
                    input.as_ptr(),
                    16,
                    records.as_mut_ptr().cast(),
                    count as usize * 24,
                    &mut written,
                )
            },
            "ns list records",
        )?;
        if written > count as u32 {
            return Err(Error("invalid ns record count", 0));
        }
    }
    let original = records;
    let mut n = 0;
    for i in 0..written as usize {
        if records[i].key.id != key.id || records[i].key.kind != key.kind {
            records[n] = records[i];
            n += 1;
        }
    }
    records[n] = Record {
        key,
        storage: storage as u64,
    };
    n += 1;
    let mut input = [0; 16];
    input[0] = 3;
    input[8..].copy_from_slice(&app.to_le_bytes());
    let old_count = written;
    // Goldleaf's delete-then-push flow, preserving the old list for recovery.
    unsafe {
        ffi::sa_ns(
            27,
            app.to_le_bytes().as_ptr(),
            8,
            core::ptr::null_mut(),
            0,
            &mut written,
        );
    }
    let pushed = unsafe {
        ffi::sa_ns(
            16,
            input.as_ptr(),
            16,
            records.as_mut_ptr().cast(),
            n * 24,
            &mut written,
        )
    };
    if pushed != 0 && old_count > 0 {
        let restored = unsafe {
            ffi::sa_ns(
                16,
                input.as_ptr(),
                16,
                original.as_ptr() as *mut u8,
                old_count as usize * 24,
                &mut written,
            )
        };
        if restored != 0 {
            return Err(Error("ns push and record restore failed", restored));
        }
    }
    rc(pushed, "ns PushApplicationRecord")
}
pub fn install(url: &str, storage: u32) -> Result<u64> {
    let _cleanup = Cleanup;
    unsafe {
        ffi::mkdir(c"sdmc:/switch".as_ptr(), 0o777);
    }
    let mut chunk = Buffer::new(CHUNK)?;
    crate::log::add("install: downloading");
    http::download(url, NSP, &mut chunk, 32 * 1024 * 1024 * 1024)?;
    install_downloaded(storage, &mut chunk)
}
pub(crate) fn install_downloaded(storage: u32, chunk: &mut [u8]) -> Result<u64> {
    let _cleanup = Cleanup;
    let mut f = File::open(NSP, false)?;
    let p = Pfs::open(&mut f)?;
    for i in 0..p.count {
        use core::fmt::Write;
        let e = p.entry(i)?;
        let mut line = Text::<192>::new();
        write!(&mut line, "install: file {} size={}", e.name, e.size)
            .map_err(|_| Error("file log length", 0))?;
        crate::log::add(line.str());
    }
    ticket(&mut f, &p)?;
    let mut cnca = None;
    for i in 0..p.count {
        let e = p.entry(i)?;
        if e.name.ends_with(".cnmt.nca") {
            if cnca.is_some() {
                return Err(Error("multiple CNMT NCAs unsupported", 0));
            }
            cnca = Some(e)
        }
    }
    let cnca = cnca.ok_or(Error("CNMT NCA missing", 0))?;
    let id = unhex(cnca.name.strip_suffix(".cnmt.nca").unwrap_or("").as_bytes())?;
    {
        let mut out = File::open(CNCA, true)?;
        let mut off = 0;
        while off < cnca.size {
            let n = (cnca.size - off).min(chunk.len() as u64) as usize;
            f.read(cnca.off + off, &mut chunk[..n])?;
            out.write(off, &chunk[..n])?;
            off += n as u64;
            if crate::retiring() {
                return Err(Error("retired", 0));
            }
        }
        out.flush()?;
    }
    // Match Goldleaf: copy the SD-extracted CNMT NCA to SYSTEM Contents,
    // then ask FSP to decrypt it through the SystemContent device.
    let mut name = Text::<160>::new();
    name.push(cnca.name.as_bytes())?;
    let mut stage_fs = Service::default();
    let mut stage_file = Service::default();
    rc(
        unsafe { ffi::sa_stage_open(name.c(), cnca.size as i64, &mut stage_file, &mut stage_fs) },
        "stage CNMT on SYSTEM",
    )?;
    let mut stage = Stage(stage_fs);
    {
        let mut out = File(stage_file);
        let mut input = File::open(CNCA, false)?;
        let mut off = 0;
        while off < cnca.size {
            if crate::retiring() {
                return Err(Error("retired", 0));
            }
            let n = (cnca.size - off).min(chunk.len() as u64) as usize;
            input.read(off, &mut chunk[..n])?;
            out.write(off, &chunk[..n])?;
            off += n as u64;
        }
        out.flush()?;
    }
    rc(
        unsafe { ffi::sa_stage_commit(&mut stage.0) },
        "commit CNMT staging",
    )?;
    let mut content_path = Text::<256>::new();
    content_path.push(b"@SystemContent://swagent-install/")?;
    content_path.push(cnca.name.as_bytes())?;
    let mut mounted = Service::default();
    let mut cnmt_file = Service::default();
    rc(
        unsafe { ffi::sa_cnmt_open(content_path.c(), &mut cnmt_file, &mut mounted) },
        "open CNMT filesystem",
    )?;
    let mount = Mounted(mounted);
    let mut cnmt_file = File(cnmt_file);
    let size = cnmt_file.size()?;
    if size < 32 || size > META_CAP as u64 {
        return Err(Error("CNMT size cap", 0));
    }
    let mut data = Buffer::new(size as usize)?;
    cnmt_file.read(0, &mut data)?;
    drop(cnmt_file);
    drop(mount);
    drop(stage);
    let m = meta(&data)?;
    let mut own = [0u8; 24];
    own[..16].copy_from_slice(&id);
    own[16..20].copy_from_slice(&(cnca.size as u32).to_le_bytes());
    own[20] = (cnca.size >> 32) as u8;
    // Validate all files before modifying NCM.
    for info in core::iter::once(&own[..]).chain(m.contents.chunks_exact(56).map(|c| &c[32..])) {
        if info[22] == 6 {
            continue;
        }
        let name = content_name(info)?;
        let e = p.find(name.str())?;
        if e.size != content_size(info) {
            return Err(Error("NCA size mismatch", 0));
        }
    }
    rc(unsafe { ffi::ncmInitialize() }, "ncm initialize")?;
    let _ncm = Ncm;
    let mut cs = Service::default();
    rc(
        unsafe { ffi::ncmOpenContentStorage(&mut cs, storage) },
        "open content storage",
    )?;
    let mut cs = Storage(cs);
    let mut db = Service::default();
    rc(
        unsafe { ffi::ncmOpenContentMetaDatabase(&mut db, storage) },
        "open meta database",
    )?;
    let mut db = Database(db);
    crate::log::add("install: writing contents");
    for info in core::iter::once(&own[..]).chain(m.contents.chunks_exact(56).map(|c| &c[32..])) {
        if info[22] == 6 {
            continue;
        }
        let name = content_name(info)?;
        let e = p.find(name.str())?;
        let mut present = false;
        rc(
            unsafe { ffi::ncmContentStorageHas(&mut cs.0, &mut present, info.as_ptr()) },
            "NCM Has",
        )?;
        if present {
            continue;
        }
        let mut ph = [0; 16];
        rc(
            unsafe { ffi::ncmContentStorageGeneratePlaceHolderId(&mut cs.0, ph.as_mut_ptr()) },
            "generate placeholder",
        )?;
        rc(
            unsafe {
                ffi::ncmContentStorageCreatePlaceHolder(
                    &mut cs.0,
                    info.as_ptr(),
                    ph.as_ptr(),
                    e.size as i64,
                )
            },
            "create placeholder",
        )?;
        let ph = Placeholder {
            storage: &mut cs.0,
            id: ph,
        };
        let mut off = 0;
        while off < e.size {
            if crate::retiring() {
                return Err(Error("retired", 0));
            }
            let n = (e.size - off).min(chunk.len() as u64) as usize;
            f.read(e.off + off, &mut chunk[..n])?;
            rc(
                unsafe {
                    ffi::ncmContentStorageWritePlaceHolder(
                        ph.storage,
                        ph.id.as_ptr(),
                        off,
                        chunk.as_ptr(),
                        n,
                    )
                },
                "write placeholder",
            )?;
            off += n as u64;
        }
        rc(
            unsafe { ffi::ncmContentStorageRegister(ph.storage, info.as_ptr(), ph.id.as_ptr()) },
            "register content",
        )?;
    }
    let count = 1 + m.contents.chunks_exact(56).filter(|c| c[54] != 6).count();
    let size = 8 + m.ext.len() + count * 24 + m.metas.len() + m.extended;
    if size > META_CAP {
        return Err(Error("rebuilt CNMT size cap", 0));
    }
    let mut out = Buffer::new(size)?;
    out.fill(0);
    out[..2].copy_from_slice(&(m.ext.len() as u16).to_le_bytes());
    out[2..4].copy_from_slice(&(count as u16).to_le_bytes());
    out[4..6].copy_from_slice(&((m.metas.len() / 16) as u16).to_le_bytes());
    out[6] = m.attributes;
    out[7] = storage as u8;
    out[8..8 + m.ext.len()].copy_from_slice(m.ext);
    let mut at = 8 + m.ext.len();
    for info in core::iter::once(&own[..])
        .chain(m.contents.chunks_exact(56).map(|c| &c[32..]))
        .filter(|c| c[22] != 6)
    {
        out[at..at + 24].copy_from_slice(info);
        at += 24;
    }
    out[at..at + m.metas.len()].copy_from_slice(m.metas);
    // Preserve actual extended data rather than fabricate delta records.
    if m.extended > 0 {
        let from = 32 + m.ext.len() + m.contents.len() + m.metas.len();
        let at = at + m.metas.len();
        out[at..at + m.extended].copy_from_slice(&data[from..from + m.extended]);
    }
    rc(
        unsafe { ffi::ncmContentMetaDatabaseSet(&mut db.0, &m.key, out.as_ptr(), size as u64) },
        "set content meta",
    )?;
    rc(
        unsafe { ffi::ncmContentMetaDatabaseCommit(&mut db.0) },
        "commit content meta",
    )?;
    records(m.key, storage)?;
    Ok(m.key.id)
}
