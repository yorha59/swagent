/* Atomic slots: no mutex held while formatting, copying or sending a log line. */
use core::sync::atomic::{AtomicU64,AtomicU8,AtomicBool,Ordering::*};
use crate::util::Text;
const COUNT:usize=1000;
struct Slot {writing:AtomicBool,seq:AtomicU64,len:AtomicU64,bytes:[AtomicU8;192]}
impl Slot {const fn new()->Self {Self{writing:AtomicBool::new(false),seq:AtomicU64::new(0),len:AtomicU64::new(0),bytes:[const{AtomicU8::new(0)};192]}}}
static NEXT:AtomicU64=AtomicU64::new(1);
static SLOTS:[Slot;COUNT]=[const{Slot::new()};COUNT];
pub fn add(s:&str) {
    let seq=NEXT.fetch_add(1,AcqRel);let slot=&SLOTS[seq as usize%COUNT];
    if slot.writing.compare_exchange(false,true,Acquire,Relaxed).is_err(){return}
    slot.seq.store(0,Release);
    let n=s.len().min(191);
    for (a,b) in slot.bytes.iter().zip(s.as_bytes()).take(n){a.store(*b,Relaxed)}
    slot.len.store(n as u64,Relaxed);slot.seq.store(seq,Release);slot.writing.store(false,Release);
}
pub fn dump(n:usize,mut send:impl FnMut(&[u8])) {
    let end=NEXT.load(Acquire);let start=end.saturating_sub(n.min(COUNT) as u64).max(1);
    for seq in start..end {
        let slot=&SLOTS[seq as usize%COUNT];
        if slot.writing.load(Acquire)||slot.seq.load(Acquire)!=seq {continue}
        let mut line=Text::<192>::new();let n=slot.len.load(Relaxed).min(191) as usize;
        for i in 0..n {line.data[i]=slot.bytes[i].load(Relaxed)} line.len=n;
        if !slot.writing.load(Acquire)&&slot.seq.load(Acquire)==seq {send(line.bytes())}
    }
}
