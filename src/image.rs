use crate::{ffi,util::*};
struct Decoder(*mut core::ffi::c_void);
impl Drop for Decoder {fn drop(&mut self){unsafe{ffi::sa_jpeg_close(self.0)}}}
pub fn shot(mut send:impl FnMut(&[u8])->Result)->Result {
    let mut jpeg=Buffer::new(0x80000)?;let mut n=0;
    rc(unsafe{ffi::sa_capture(jpeg.as_mut_ptr(),jpeg.len(),&mut n)},"capture screenshot")?;
    if n==0||n>jpeg.len() as u64 {return Err(Error("invalid JPEG size",0))}
    let d=Decoder(unsafe{ffi::sa_jpeg()});if d.0.is_null(){return Err(Error("JPEG decoder allocation",0))}
    let(mut w,mut h)=(0,0);
    if unsafe{ffi::sa_jpeg_header(d.0,jpeg.as_ptr(),n as usize,&mut w,&mut h)}!=0||w<=0||h<=0||w>1280||h>720 {return Err(Error("JPEG header",0))}
    let mut rgb=Buffer::new(w as usize*h as usize*3)?;
    if unsafe{ffi::sa_jpeg_decode(d.0,jpeg.as_ptr(),n as usize,rgb.as_mut_ptr(),w,h)}!=0 {return Err(Error("JPEG decode",0))}
    drop(d);drop(jpeg);
    let mut b=Base64::new(&mut send);
    png(&rgb,w as usize,h as usize,|p|b.write(p))?;b.finish()
}
pub fn crc32(p:&[u8],mut crc:u32)->u32 {
    for b in p {crc^=*b as u32;for _ in 0..8 {crc=(crc>>1)^(0xedb88320u32.wrapping_mul(crc&1));}}crc
}
fn chunk(kind:&[u8;4],data:&[u8],send:&mut impl FnMut(&[u8])->Result)->Result {
    send(&(data.len() as u32).to_be_bytes())?;send(kind)?;send(data)?;
    send(&(!crc32(data,crc32(kind,!0))).to_be_bytes())
}
/* Stored DEFLATE blocks: PNG needs no compression library or full PNG buffer. */
pub fn png(rgb:&[u8],w:usize,h:usize,mut send:impl FnMut(&[u8])->Result)->Result {
    if w==0||h==0||w>1280||h>720||rgb.len()!=w*h*3 {return Err(Error("invalid RGB dimensions",0))}
    send(b"\x89PNG\r\n\x1a\n")?;
    let mut ihdr=[0;13];ihdr[..4].copy_from_slice(&(w as u32).to_be_bytes());ihdr[4..8].copy_from_slice(&(h as u32).to_be_bytes());ihdr[8]=8;ihdr[9]=2;
    chunk(b"IHDR",&ihdr,&mut send)?;
    let (mut a,mut b)=(1u32,0u32);
    for y in 0..h {
        let mut row=[0u8;3852];let mut n=0;
        if y==0 {row[..2].copy_from_slice(&[0x78,0x01]);n=2;}
        let len=(w*3+1) as u16;row[n]=(y+1==h) as u8;
        row[n+1..n+3].copy_from_slice(&len.to_le_bytes());row[n+3..n+5].copy_from_slice(&(!len).to_le_bytes());n+=5;
        row[n]=0;row[n+1..n+len as usize].copy_from_slice(&rgb[y*w*3..(y+1)*w*3]);
        for v in &row[n..n+len as usize] {a=(a+*v as u32)%65521;b=(b+a)%65521;}n+=len as usize;
        if y+1==h {row[n..n+4].copy_from_slice(&((b<<16)|a).to_be_bytes());n+=4;}
        chunk(b"IDAT",&row[..n],&mut send)?;
    }
    chunk(b"IEND",&[],&mut send)
}
pub struct Base64<F> {send:F,line:Text<769>,pending:[u8;3],n:usize}
impl<F:FnMut(&[u8])->Result> Base64<F> {
    pub fn new(send:F)->Self{Self{send,line:Text::new(),pending:[0;3],n:0}}
    fn group(&mut self,n:usize)->Result {
        let abc=b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let p=self.pending;let x=((p[0] as u32)<<16)|((p[1] as u32)<<8)|p[2] as u32;
        self.line.push(&[abc[(x>>18) as usize],abc[((x>>12)&63) as usize],if n>1{abc[((x>>6)&63) as usize]}else{b'='},if n>2{abc[(x&63) as usize]}else{b'='}])?;
        if self.line.len==768 {(self.send)(self.line.bytes())?;self.line.len=0;}self.pending=[0;3];self.n=0;Ok(())
    }
    pub fn write(&mut self,p:&[u8])->Result {
        for b in p {self.pending[self.n]=*b;self.n+=1;if self.n==3 {self.group(3)?;}}Ok(())
    }
    pub fn finish(mut self)->Result {if self.n>0 {self.group(self.n)?;}if self.line.len>0 {(self.send)(self.line.bytes())?;}Ok(())}
}
