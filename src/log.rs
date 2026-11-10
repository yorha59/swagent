/* Atomic slots: no mutex held while formatting, copying or sending a log line. */
use crate::util::Text;
use core::sync::atomic::{fence, AtomicBool, AtomicU64, AtomicU8, Ordering::*};
const COUNT: usize = 1000;
struct Slot {
    writing: AtomicBool,
    seq: AtomicU64,
    len: AtomicU64,
    bytes: [AtomicU8; 192],
}
impl Slot {
    const fn new() -> Self {
        Self {
            writing: AtomicBool::new(false),
            seq: AtomicU64::new(0),
            len: AtomicU64::new(0),
            bytes: [const { AtomicU8::new(0) }; 192],
        }
    }
}
static NEXT: AtomicU64 = AtomicU64::new(1);
static SLOTS: [Slot; COUNT] = [const { Slot::new() }; COUNT];
pub fn add(s: &str) {
    unsafe { crate::ffi::sa_persist(s.as_ptr(), s.len()); }
    let seq = NEXT.fetch_add(1, AcqRel);
    let Some(slot) = SLOTS.get(seq as usize % COUNT) else {
        return;
    };
    if slot
        .writing
        .compare_exchange(false, true, Acquire, Relaxed)
        .is_err()
    {
        return;
    }
    slot.seq.store(0, Release);
    let n = s.len().min(191);
    for (a, b) in slot.bytes.iter().zip(s.as_bytes()).take(n) {
        a.store(*b, Relaxed)
    }
    slot.len.store(n as u64, Relaxed);
    slot.seq.store(seq, Release);
    slot.writing.store(false, Release);
}
pub fn dump(n: usize, mut send: impl FnMut(&[u8])) {
    let end = NEXT.load(Acquire);
    let start = end.saturating_sub(n.min(COUNT) as u64).max(1);
    for seq in start..end {
        let Some(slot) = SLOTS.get(seq as usize % COUNT) else {
            return;
        };
        if slot.writing.load(Acquire) || slot.seq.load(Acquire) != seq {
            continue;
        }
        let mut line = Text::<192>::new();
        let n = slot.len.load(Relaxed).min(191) as usize;
        for (out, byte) in line.data.iter_mut().zip(slot.bytes.iter()).take(n) {
            *out = byte.load(Relaxed);
        }
        line.len = n;
        fence(SeqCst);
        if !slot.writing.load(Acquire) && slot.seq.load(Acquire) == seq {
            send(line.bytes())
        }
    }
}

pub fn boot_line(ms: u64) -> Text<96> {
    use core::fmt::Write;
    let mut s=Text::new();
    let _=write!(s,"boot uptime_ms={} version=swagent-1.33-rs",ms);
    s
}
pub fn file_tail(mut send: impl FnMut(&[u8])) {
    let mut bytes=[0u8;16384];
    let n=unsafe { crate::ffi::sa_log_tail(bytes.as_mut_ptr(),bytes.len()) }.min(bytes.len());
    for line in bytes[..n].split(|b| *b==b'\n').filter(|v|!v.is_empty()) { send(line); }
}

/// Fixed-size, best-effort stage timer. A dead/stalled writer disables all work.
pub struct Span {
    op: &'static str, title: u64, phase: &'static str,
    start: u64, last: u64, bytes: u64, enabled: bool, slot: Option<usize>,
}
impl Span {
    pub fn new(op: &'static str, title: u64, phase: &'static str) -> Self {
        let enabled=unsafe { crate::ffi::sa_log_alive()!=0 };
        let start=if enabled { crate::util::now_ms() } else {0};
        let slot=if enabled { STAGES.iter().position(|s| s.phase.compare_exchange(0,255,Acquire,Relaxed).is_ok()) } else {None};
        if let Some(i)=slot {
            let s=&STAGES[i];
            s.title.store(title,Relaxed); s.op.store((op=="install") as u8,Relaxed);
            s.done.store(0,Relaxed); s.total.store(0,Relaxed); s.last.store(start,Relaxed);
            s.phase.store(PHASES.iter().position(|p| *p==phase).map(|i|i as u8+1).unwrap_or(254),Release);
        }
        Self {op,title,phase,start,last:start,bytes:0,enabled,slot}
    }
    pub fn progress(&mut self, bytes: u64, total: u64) {
        if !self.enabled || unsafe { crate::ffi::sa_log_alive()==0 } { return; }
        self.bytes=bytes;
        if let Some(i)=self.slot {
            STAGES[i].done.store(bytes,Relaxed); STAGES[i].total.store(total,Relaxed);
            if STAGES[i].phase.load(Acquire)<254 { tick_stage(&STAGES[i],crate::util::now_ms()); return; }
        }
        let now=crate::util::now_ms();
        if now.saturating_sub(self.last)<30000 { return; }
        self.last=now;
        use core::fmt::Write;
        let pct=if total==0 {0} else {(bytes as u128*1000/total as u128).min(1000) as u64};
        let mut line=Text::<192>::new();
        let _=write!(line,"span op={} title={:016X} phase={} progress={}.{}%",self.op,self.title,self.phase,pct/10,pct%10);
        add(line.str());
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        if let Some(i)=self.slot { STAGES[i].phase.store(0,Release); }
        if !self.enabled || unsafe { crate::ffi::sa_log_alive()==0 } { return; }
        use core::fmt::Write;
        let mut line=Text::<192>::new();
        let _=write!(line,"span op={} title={:016X} phase={} t={}ms bytes={}",self.op,self.title,self.phase,crate::util::now_ms().saturating_sub(self.start),if self.phase=="download" {crate::install::INSTALL_RX_BYTES.load(Relaxed)} else {self.bytes});
        add(line.str());
    }
}

const PHASES: &[&str]=&["download","lookup","ncm-lookup","ticket","cnmt","pack","content-read","finalize"];
struct Stage {
    phase: AtomicU8, op: AtomicU8, title: AtomicU64,
    last: AtomicU64, done: AtomicU64, total: AtomicU64,
}
impl Stage {
    const fn new()->Self { Self {phase:AtomicU8::new(0),op:AtomicU8::new(0),title:AtomicU64::new(0),last:AtomicU64::new(0),done:AtomicU64::new(0),total:AtomicU64::new(0)} }
}
static STAGES: [Stage;8]=[const {Stage::new()};8];
fn tick_stage(s:&Stage,now:u64) {
    let phase=s.phase.load(Acquire);
    if phase==0 || phase as usize>PHASES.len() {return;}
    let last=s.last.load(Relaxed);
    if now.saturating_sub(last)<30000 || s.last.compare_exchange(last,now,AcqRel,Relaxed).is_err() {return;}
    let (done,total)=if phase==1 {
        (crate::install::INSTALL_RX_BYTES.load(Relaxed),crate::install::INSTALL_TOTAL_BYTES.load(Relaxed))
    } else { (s.done.load(Relaxed),s.total.load(Relaxed)) };
    let pct=if total==0 {0} else {(done as u128*1000/total as u128).min(1000) as u64};
    use core::fmt::Write;
    let mut line=Text::<192>::new();
    let _=write!(line,"span op={} title={:016X} phase={} progress={}.{}%",if s.op.load(Relaxed)==1 {"install"} else {"dump"},s.title.load(Relaxed),PHASES[phase as usize-1],pct/10,pct%10);
    if s.phase.load(Acquire)==phase {add(line.str());}
}
/// Watchdog emits even if the operation is currently stuck inside an IPC.
pub fn span_tick() {
    if unsafe {crate::ffi::sa_log_alive()==0} {return;}
    let now=crate::util::now_ms();
    for stage in &STAGES {tick_stage(stage,now);}
}
