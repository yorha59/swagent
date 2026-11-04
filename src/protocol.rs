use core::{fmt::Write,sync::atomic::{AtomicBool,AtomicI32,Ordering::*}};
use crate::{ffi,http,image,install,io::{self,File,Socket},log,util::*};
static RETIRING:AtomicBool=AtomicBool::new(false);
static LISTENER:AtomicI32=AtomicI32::new(-1);
struct Session {fd:i32,id:u64}
static SESSION:Lock<Session>=Lock::new(Session{fd:-1,id:0});
#[derive(Clone,Copy)]
struct Job {id:u64,line:Text<512>}
const EMPTY:Job=Job{id:0,line:Text::new()};
struct Queue {jobs:[Job;8],head:usize,len:usize}
static QUEUE:Lock<Queue>=Lock::new(Queue{jobs:[EMPTY;8],head:0,len:0});
pub fn retiring()->bool {RETIRING.load(Acquire)}
pub fn reply(id:u64,line:&[u8])->Result {
    SESSION.with(|s| {
        if s.id!=id||s.fd<0 {return Err(Error("client disconnected",0))}
        let sock=core::mem::ManuallyDrop::new(Socket(s.fd));
        sock.send(line)?;sock.send(b"\r\n")
    })
}
fn error(id:u64,prefix:&str,e:Error) {
    let mut s=Text::<256>::new();let _=write!(s,"{}: error {} rc={}",prefix,e.0,e.1);log::add(s.str());let _=reply(id,s.bytes());
}
fn enqueue(id:u64,line:Text<512>) {
    let ok=QUEUE.with(|q| {if q.len==8{return false}let tail=(q.head+q.len)%8;q.jobs[tail]=Job{id,line};q.len+=1;true});
    if !ok {let _=reply(id,b"error: busy");}
}
fn active(id:u64)->bool {SESSION.with(|s|s.id==id&&s.fd>=0)}
pub extern "C" fn worker(_: *mut core::ffi::c_void) {
    loop {
        if retiring(){forever()}
        let job=QUEUE.with(|q| {if q.len==0 {return None}let job=q.jobs[q.head];q.head=(q.head+1)%8;q.len-=1;Some(job)});
        if let Some(job)=job {if active(job.id){execute(job)}}else{sleep(5)}
    }
}
pub fn button(name:&str)->Result<u64> {
    let bit=match name {"A"=>0,"B"=>1,"X"=>2,"Y"=>3,"LSTICK"|"LS"=>4,"RSTICK"|"RS"=>5,"L"=>6,"R"=>7,"ZL"=>8,"ZR"=>9,"PLUS"|"+"=>10,"MINUS"|"-"=>11,"LEFT"=>12,"UP"=>13,"RIGHT"=>14,"DOWN"=>15,_=>return Err(Error("unknown button",0))};Ok(1u64<<bit)
}
fn duration(s:Option<&str>)->Result<u64> {
    match s {None=>Ok(100),Some(s)=>s.parse::<u64>().ok().filter(|n|*n<=60_000).ok_or(Error("hold_ms must be 0..60000",0))}
}
fn execute(job:Job) {
    let id=job.id;let mut words=job.line.str().split_ascii_whitespace();let cmd=words.next().unwrap_or("");
    let result=(||->Result {
        match cmd {
            "shot"=>{
                if words.next().is_some(){return Err(Error("usage: shot",0))}
                image::shot(|p|reply(id,p))?;reply(id,b"shot: end")?;
            }
            "key"=>{
                let b=button(words.next().ok_or(Error("usage: key BUTTON [hold_ms]",0))?)?;
                let ms=duration(words.next())?;if words.next().is_some(){return Err(Error("too many arguments",0))}
                rc(unsafe{ffi::sa_hid_init()},"HID initialize")?;
                let down=unsafe{ffi::sa_buttons(b)};if down==0 {sleep(ms)}
                let up=unsafe{ffi::sa_buttons(0)};rc(down,"press key")?;rc(up,"release key")?;reply(id,b"key: ok")?;
            }
            "touch"=>{
                let x=words.next().and_then(|s|s.parse::<u32>().ok()).filter(|n|*n<1280).ok_or(Error("x must be 0..1279",0))?;
                let y=words.next().and_then(|s|s.parse::<u32>().ok()).filter(|n|*n<720).ok_or(Error("y must be 0..719",0))?;
                let ms=duration(words.next())?;if words.next().is_some(){return Err(Error("too many arguments",0))}
                rc(unsafe{ffi::sa_hid_init()},"HID initialize")?;
                let down=unsafe{ffi::sa_touch(x,y,ms*1_000_000,1)};if down==0{sleep(ms)}
                let up=unsafe{ffi::sa_touch(0,0,0,0)};let unset=unsafe{ffi::hiddbgUnsetTouchScreenAutoPilotState()};
                rc(down,"touch down")?;rc(up,"touch release")?;rc(unset,"touch unset")?;reply(id,b"touch: ok")?;
            }
            "install"=>{
                let url=words.next().ok_or(Error("usage: install URL [sd|nand]",0))?;
                let storage=match words.next().unwrap_or("sd") {"sd"=>5,"nand"=>4,_=>return Err(Error("destination must be sd or nand",0))};
                if words.next().is_some(){return Err(Error("too many arguments",0))}
                let title=install::install(url,storage)?;let mut s=Text::<64>::new();let _=write!(s,"install: ok {:016x}",title);log::add(s.str());reply(id,s.bytes())?;
            }
            "log"=>{
                let n=match words.next(){Some(s)=>s.parse::<usize>().map_err(|_|Error("invalid log count",0))?,None=>100};
                if words.next().is_some(){return Err(Error("too many arguments",0))}
                log::dump(n,|p|{let _=reply(id,p);});reply(id,b"log: end")?;
            }
            _=>{reply(id,b"error: unknown command")?;}
        }Ok(())
    })();
    if let Err(e)=result {error(id,cmd,e)}
}
const DEST:&str="sdmc:/atmosphere/contents/430000000000000B/exefs.nsp";
const TMP:&str="sdmc:/atmosphere/contents/430000000000000B/exefs.nsp.tmp";
const BACKUP:&str="sdmc:/atmosphere/contents/430000000000000B/exefs.nsp.bak";
fn update(id:u64,url:&str)->Result {
    let mut chunk=Buffer::new(64*1024)?;
    struct Clean;
    impl Drop for Clean {fn drop(&mut self){io::remove(TMP)}}
    let _clean=Clean;
    log::add("update: downloading");let size=http::download(url,TMP,&mut chunk,50*1024*1024)?;
    if size<100*1024{return Err(Error("update image too small",0))}
    {let mut f=File::open(TMP,false)?;let p=install::Pfs::open(&mut f)?;
        let(mut nso,mut npdm)=(false,false);
        for i in 0..p.count {
            let e=p.entry(i)?;
            if e.name=="main" {
                if e.size<0x100{return Err(Error("short update NSO",0))}let mut magic=[0;4];f.read(e.off,&mut magic)?;nso=&magic==b"NSO0";
            }
            if e.name=="main.npdm" {
                if e.size<0x80{return Err(Error("short update NPDM",0))}
                let mut h=[0;0x80];f.read(e.off,&mut h)?;if &h[..4]!=b"META" {return Err(Error("invalid NPDM magic",0))}
                let at=le32(&h[0x70..]) as u64;
                if at+0x20>e.size{return Err(Error("invalid NPDM ACI offset",0))}
                let mut aci=[0;0x20];f.read(e.off+at,&mut aci)?;
                npdm=&aci[..4]==b"ACI0"&&le64(&aci[0x10..])==0x430000000000000b;
            }
        }
        if !nso||!npdm{return Err(Error("update must contain main and matching NPDM",0))}
    }
    // Keep a recoverable previous image until launch succeeds.
    io::remove(BACKUP);io::rename(DEST,BACKUP)?;
    if let Err(e)=io::rename(TMP,DEST){let _=io::rename(BACKUP,DEST);return Err(e)}
    let mut pid=0;let launch=unsafe{ffi::sa_launch(&mut pid)};
    if launch!=0 {
        io::remove(DEST);let _=io::rename(BACKUP,DEST);return Err(Error("launch replacement",launch))
    }
    RETIRING.store(true,Release);
    // Release server first, then client. Never terminate the old process.
    let listener=LISTENER.swap(-1,AcqRel);if listener>=0{unsafe{ffi::shutdown(listener,2);ffi::close(listener);}}
    let _=reply(id,b"update: ok");
    disconnect();log::add("update: retired; sleeping forever");forever()
}
fn disconnect() {SESSION.with(|s| {if s.fd>=0 {unsafe{ffi::shutdown(s.fd,2);ffi::close(s.fd);}s.fd=-1;}})}
fn receive(id:u64,line:Text<512>) {
    let mut words=line.str().split_ascii_whitespace();
    match words.next() {
        Some("ping") if words.next().is_none()=>{let _=reply(id,b"pong");}
        Some("version") if words.next().is_none()=>{let _=reply(id,b"swagent-1.0-rs");}
        Some("update")=>{
            let result=match (words.next(),words.next()) {(Some(url),None)=>update(id,url),_=>Err(Error("usage: update URL",0))};
            if let Err(e)=result{error(id,"update",e)}
        }
        _=>enqueue(id,line)
    }
}
pub fn serve()->! {
    loop {
        if retiring(){forever()}
        // Initial attempt plus five EADDRINUSE retries at one-second intervals.
        let mut server=-1;
        for attempt in 0..=5 {
            server=unsafe{ffi::sa_listen()};if server>=0{break}
            if server != -112||attempt==5{break}sleep(1000);
        }
        if server<0 {log::add("listen failed; retrying");sleep(5000);continue}
        LISTENER.store(server,Release);log::add("listening :6000");
        loop {
            let fd=unsafe{ffi::sa_accept(server)};if fd<0{break}
            unsafe{ffi::sa_timeout(fd,2);}
            let id=SESSION.with(|s|{s.id=s.id.wrapping_add(1);s.fd=fd;s.id});
            log::add("client connected");
            let mut line=Text::<512>::new();let mut overflow=false;let mut previous_cr=false;
            'client: loop {
                let mut input=[0;512];let n=unsafe{ffi::recv(fd,input.as_mut_ptr(),input.len(),0)};
                if n==0{break}if n<0 {
                    let e=unsafe{ffi::sa_errno()};if e==11||e==35||e==116{continue}break
                }
                for byte in &input[..n as usize] {
                    if previous_cr&&*byte==b'\n' {
                        if overflow {let _=reply(id,b"error: line too long");}else if line.len>0 {receive(id,line)}
                        line=Text::new();overflow=false;previous_cr=false;continue;
                    }
                    if previous_cr {overflow=true;previous_cr=false;}
                    if *byte==b'\r' {previous_cr=true;}else if *byte==b'\n'||*byte==0 {overflow=true;}else if line.push(core::slice::from_ref(byte)).is_err(){overflow=true}
                    if retiring(){break 'client}
                }
            }
            disconnect();log::add("client disconnected");
        }
        LISTENER.store(-1,Release);unsafe{ffi::close(server);}sleep(1000);
    }
}
