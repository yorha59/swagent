use crate::{ffi, util::*};
use core::fmt::Write;
pub fn parse(line:&str)->Result<Option<u64>> {
    let mut w=line.split_ascii_whitespace();
    if w.next()!=Some("installed") { return Err(Error("usage: installed [del TITLEID del-TITLEID-confirm]",0)); }
    let Some(action)=w.next() else { return Ok(None); };
    let raw=w.next().ok_or(Error("missing titleID",0))?;
    let id=u64::from_str_radix(raw,16).map_err(|_|Error("invalid titleID",0))?;
    let mut confirm=Text::<64>::new();
    let _=write!(confirm,"del-{}-confirm",raw);
    if action!="del" || raw.len()!=16 || w.next()!=Some(confirm.str()) || w.next().is_some() {
        return Err(Error("require: installed del TITLEID del-TITLEID-confirm",0));
    }
    Ok(Some(id))
}
pub fn run(line:&str, send:&mut dyn FnMut(&[u8])->Result<()>)->Result<()> {
    if let Some(id)=parse(line)? {
        crate::log::add(line);
        rc(unsafe { ffi::sa_installed_delete(id) },"DeleteApplicationCompletely")?;
        return send(b"installed: deleted");
    }
    let mut offset=0;
    loop {
        let mut id=0; let mut name=[0u8;513]; let mut count=0;
        rc(unsafe { ffi::sa_installed(offset,&mut id,name.as_mut_ptr(),name.len(),&mut count) },"ListApplicationRecord")?;
        if count==0 { break; }
        let mut out=Text::<768>::new();
        let _=write!(out,"{:016X} ",id);
        let n=name.iter().position(|b| *b==0).unwrap_or(name.len());
        out.push(&name[..n])?;
        send(out.bytes())?;
        offset+=1;
        if crate::retiring() { return Err(Error("retired",0)); }
    }
    send(b"installed: end")
}
