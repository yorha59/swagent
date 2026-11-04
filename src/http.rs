use crate::{
    ffi,
    io::{File, Socket},
    util::*,
};
use core::fmt::Write;
pub struct Url {
    pub host: Text<256>,
    pub port: Text<8>,
    pub path: Text<1024>,
}
pub fn parse(url: &str) -> Result<Url> {
    let rest = url
        .strip_prefix("http://")
        .ok_or(Error("expected http:// URL", 0))?;
    if rest.bytes().any(|b| b <= 32 || b == 127) || rest.contains(['#', '@']) {
        return Err(Error("invalid URL", 0));
    }
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = authority.split_once(':').unwrap_or((authority, "80"));
    if host.is_empty() || port.parse::<u16>().unwrap_or(0) == 0 || host.contains(['[', ']']) {
        return Err(Error("invalid host or port", 0));
    }
    let mut u = Url {
        host: Text::new(),
        port: Text::new(),
        path: Text::new(),
    };
    u.host.push(host.as_bytes())?;
    u.port.push(port.as_bytes())?;
    u.path.push(b"/")?;
    u.path.push(path.as_bytes())?;
    Ok(u)
}
pub fn download(url: &str, path: &str, chunk: &mut [u8], limit: u64) -> Result<u64> {
    let u = parse(url)?;
    let fd = unsafe { ffi::sa_connect(u.host.c(), u.port.c()) };
    if fd < 0 {
        return Err(Error("HTTP connect", 0));
    }
    let sock = Socket(fd);
    let mut request = Text::<1600>::new();
    write!(
        request,
        "GET {} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\nAccept-Encoding: identity\r\n\r\n",
        u.path.str(),
        u.host.str(),
        u.port.str()
    )
    .map_err(|_| Error("HTTP request too long", 0))?;
    sock.send(request.bytes())?;
    let mut header = Text::<8193>::new();
    let mut byte = [0];
    while !header.bytes().ends_with(b"\r\n\r\n") {
        if sock.read(&mut byte)? == 0 {
            return Err(Error("short HTTP header", 0));
        }
        header.push(&byte)?;
    }
    let h = core::str::from_utf8(header.bytes()).map_err(|_| Error("invalid HTTP header", 0))?;
    let mut lines = h.split("\r\n");
    let status = lines.next().unwrap_or("");
    if !(status.starts_with("HTTP/1.0 200 ") || status.starts_with("HTTP/1.1 200 ")) {
        return Err(Error("HTTP status must be 200", 0));
    }
    let mut length = None;
    let mut chunked = false;
    for line in lines.filter(|s| !s.is_empty()) {
        let (key, value) = line
            .split_once(':')
            .ok_or(Error("invalid HTTP header", 0))?;
        let value = value.trim();
        if key.eq_ignore_ascii_case("content-length") {
            let n = value
                .parse::<u64>()
                .map_err(|_| Error("invalid Content-Length", 0))?;
            if length.is_some_and(|old| old != n) {
                return Err(Error("conflicting Content-Length", 0));
            }
            length = Some(n);
        }
        if key.eq_ignore_ascii_case("transfer-encoding") {
            if !value.eq_ignore_ascii_case("chunked") {
                return Err(Error("unsupported Transfer-Encoding", 0));
            }
            chunked = true;
        }
        if key.eq_ignore_ascii_case("content-encoding") && !value.eq_ignore_ascii_case("identity") {
            return Err(Error("unsupported Content-Encoding", 0));
        }
    }
    if chunked && length.is_some() {
        return Err(Error("ambiguous HTTP framing", 0));
    }
    if length.is_some_and(|n| n == 0 || n > limit) {
        return Err(Error("download size limit", 0));
    }
    let mut file = File::open(path, true)?;
    let mut total = 0;
    if chunked {
        let mut buffered = 0;
        loop {
            let mut line = Text::<128>::new();
            while !line.bytes().ends_with(b"\r\n") {
                if sock.read(&mut byte)? == 0 {
                    return Err(Error("short chunk header", 0));
                }
                line.push(&byte)?;
            }
            let size = line.str().trim().split(';').next().unwrap_or("");
            let mut remaining =
                u64::from_str_radix(size, 16).map_err(|_| Error("invalid HTTP chunk", 0))?;
            if remaining == 0 {
                let mut trailer_bytes = 0;
                loop {
                    let mut trailer = Text::<1024>::new();
                    while !trailer.bytes().ends_with(b"\r\n") {
                        if sock.read(&mut byte)? == 0 {
                            return Err(Error("truncated HTTP trailers", 0));
                        }
                        trailer.push(&byte)?;
                        trailer_bytes += 1;
                        if trailer_bytes > 8192 {
                            return Err(Error("HTTP trailers too long", 0));
                        }
                    }
                    if trailer.len == 2 {
                        break;
                    }
                }
                if buffered > 0 {
                    file.write(total - buffered as u64, &chunk[..buffered])?;
                }
                break;
            }
            if remaining > limit - total {
                return Err(Error("download size limit", 0));
            }
            while remaining > 0 {
                let n = (remaining as usize).min(chunk.len() - buffered);
                let got = sock.read(&mut chunk[buffered..buffered + n])?;
                if got == 0 {
                    return Err(Error("short HTTP chunk", 0));
                }
                total += got as u64;
                buffered += got;
                if buffered == chunk.len() {
                    file.write(total - buffered as u64, &chunk[..buffered])?;
                    buffered = 0;
                }
                remaining -= got as u64;
                if crate::retiring() {
                    return Err(Error("retired", 0));
                }
            }
            let mut crlf = [0; 2];
            for b in &mut crlf {
                if sock.read(core::slice::from_mut(b))? != 1 {
                    return Err(Error("short chunk delimiter", 0));
                }
            }
            if crlf != *b"\r\n" {
                return Err(Error("invalid chunk delimiter", 0));
            }
        }
    } else {
        loop {
            let want = length.map_or(chunk.len(), |n| {
                (n - total).min(chunk.len() as u64) as usize
            });
            if want == 0 {
                break;
            }
            let mut got = 0;
            while got < want {
                let n = sock.read(&mut chunk[got..want])?;
                if n == 0 {
                    break;
                }
                got += n;
                if crate::retiring() {
                    return Err(Error("retired", 0));
                }
            }
            if got == 0 {
                break;
            }
            if got as u64 > limit - total {
                return Err(Error("download size limit", 0));
            }
            file.write(total, &chunk[..got])?;
            total += got as u64;
            if crate::retiring() {
                return Err(Error("retired", 0));
            }
        }
        if length.is_some_and(|n| n != total) {
            return Err(Error("truncated HTTP body", 0));
        }
    }
    if total == 0 {
        return Err(Error("empty HTTP body", 0));
    }
    file.flush()?;
    Ok(total)
}
