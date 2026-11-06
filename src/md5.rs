// Original dependency-free implementation of RFC 1321 MD5. Public domain (CC0).
use crate::{io::File, util::*};
pub struct Md5 {
    state: [u32; 4],
    block: [u8; 64],
    used: usize,
    len: u64,
}
impl Md5 {
    pub fn new() -> Self {
        Self {
            state: [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476],
            block: [0; 64],
            used: 0,
            len: 0,
        }
    }
    pub fn update(&mut self, mut bytes: &[u8]) {
        self.len = self.len.wrapping_add(bytes.len() as u64);
        while !bytes.is_empty() {
            let n = (64 - self.used).min(bytes.len());
            self.block[self.used..self.used + n].copy_from_slice(&bytes[..n]);
            self.used += n;
            bytes = &bytes[n..];
            if self.used == 64 {
                self.compress();
                self.used = 0;
            }
        }
    }
    fn compress(&mut self) {
        const K: [u32; 64] = [
            0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
            0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
            0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
            0x2441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
            0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
            0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
            0xd4ef3085, 0x4881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
            0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
            0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
            0xeb86d391,
        ];
        const S: [u32; 16] = [7, 12, 17, 22, 5, 9, 14, 20, 4, 11, 16, 23, 6, 10, 15, 21];
        let [mut a, mut b, mut c, mut d] = self.state;
        for i in 0..64 {
            let (f, g) = match i {
                0..=15 => ((b & c) | (!b & d), i),
                16..=31 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                32..=47 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let at = g * 4;
            let word = u32::from_le_bytes(self.block[at..at + 4].try_into().unwrap());
            let next = b.wrapping_add(
                a.wrapping_add(f)
                    .wrapping_add(K[i])
                    .wrapping_add(word)
                    .rotate_left(S[(i / 16) * 4 + i % 4]),
            );
            a = d;
            d = c;
            c = b;
            b = next;
        }
        for (s, v) in self.state.iter_mut().zip([a, b, c, d]) {
            *s = s.wrapping_add(v);
        }
    }
    pub fn finish(mut self) -> Text<33> {
        let bits = self.len.wrapping_mul(8).to_le_bytes();
        self.update(&[0x80]);
        while self.used != 56 {
            self.update(&[0]);
        }
        self.update(&bits);
        let mut out = Text::new();
        for word in self.state {
            let _ = out.push(hex(&word.to_le_bytes()).bytes());
        }
        out
    }
}
pub fn expected(arg: &str) -> Result<Text<33>> {
    let value = arg
        .strip_prefix("md5=")
        .ok_or(Error("expected md5=HEX (32 digits)", 0))?;
    if value.len() != 32 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error("expected md5=HEX (32 digits)", 0));
    }
    let mut out = Text::new();
    for b in value.bytes() {
        out.push(&[b.to_ascii_lowercase()])?;
    }
    Ok(out)
}
pub fn file(path: &str, chunk: &mut [u8], limit: u64) -> Result<Text<33>> {
    if chunk.is_empty() {
        return Err(Error("empty hash buffer", 0));
    }
    let mut f = File::open(path, false)?;
    let size = f.size()?;
    if size > limit {
        return Err(Error("too large", 0));
    }
    let mut hash = Md5::new();
    let mut at = 0;
    while at < size {
        let n = (size - at).min(chunk.len() as u64) as usize;
        f.read(at, &mut chunk[..n])?;
        hash.update(&chunk[..n]);
        at += n as u64;
        if crate::retiring() {
            return Err(Error("retired", 0));
        }
    }
    Ok(hash.finish())
}
