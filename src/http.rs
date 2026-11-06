use crate::{
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
    download_inner(url, path, chunk, limit, false, 0, &mut None)
}
pub fn download_install(url: &str, path: &str, chunk: &mut [u8], limit: u64) -> Result<u64> {
    static LAST_URL: Lock<(Text<1600>, Option<u64>)> = Lock::new((Text::new(), None));
    LAST_URL.with(|state| -> Result<u64> {
        let (last, expected) = state;
        let same = last.str() == url;
        let mut next = Text::new();
        next.push(url.as_bytes())?;
        if !same {
            File::open(path, true)?;
        }
        if !same {
            *expected = None;
        }
        *last = next;
        for failure in 1..=30 {
            let offset = match File::open(path, false) {
                Ok(mut f) => f.size()?,
                Err(_) => 0,
            };
            match download_inner(url, path, chunk, limit, true, offset, expected) {
                Ok(n) => return Ok(n),
                Err(e) => {
                    if !retryable(e) || failure == 30 || crate::retiring() {
                        return Err(e);
                    }
                    let offset = match File::open(path, false) {
                        Ok(mut f) => f.size()?,
                        Err(_) => 0,
                    };
                    let mut line = Text::<128>::new();
                    let _ = write!(line, "install: retry {} from offset {}", failure, offset);
                    crate::log::add(line.str());
                    sleep(5000);
                }
            }
        }
        unreachable!()
    })
}
fn retryable(e: Error) -> bool {
    matches!(
        e.0,
        "network receive"
            | "network send"
            | "network reset"
            | "network unavailable"
            | "HTTP connect"
            | "short HTTP header"
            | "short HTTP chunk"
            | "short chunk header"
            | "short chunk delimiter"
            | "truncated HTTP trailers"
            | "truncated HTTP body"
    ) || (e.0 == "HTTP status" && e.1 >= 500)
}
fn download_inner(
    url: &str,
    path: &str,
    chunk: &mut [u8],
    limit: u64,
    track: bool,
    offset: u64,
    original_total: &mut Option<u64>,
) -> Result<u64> {
    use core::sync::atomic::Ordering::Relaxed;
    let u = parse(url)?;
    if chunk.is_empty() {
        return Err(Error("empty download buffer", 0));
    }
    if offset > limit {
        return Err(Error("download size limit", 0));
    }
    if track {
        crate::install::INSTALL_RX_BYTES.store(offset, Relaxed);
    }
    let sock = Socket::connect(u.host.c(), u.port.c())?;
    let mut request = Text::<1600>::new();
    write!(
        request,
        "GET {} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\nAccept-Encoding: identity\r\n",
        u.path.str(),
        u.host.str(),
        u.port.str()
    )
    .map_err(|_| Error("HTTP request too long", 0))?;
    if offset > 0 {
        write!(request, "Range: bytes={offset}-\r\n")
            .map_err(|_| Error("HTTP request too long", 0))?;
    }
    request.push(b"\r\n")?;
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
    let code = status
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse::<u32>().ok())
        .ok_or(Error("invalid HTTP status", 0))?;
    if !matches!(code, 200 | 206) {
        return Err(Error("HTTP status", code));
    }
    let mut range = None;
    let mut length = None;
    let mut chunked = false;
    for line in lines.filter(|s| !s.is_empty()) {
        let (key, value) = line
            .split_once(':')
            .ok_or(Error("invalid HTTP header", 0))?;
        let value = value.trim();
        if key.eq_ignore_ascii_case("content-range") {
            range = Some(value);
        }
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
    let start = if code == 206 { offset } else { 0 };
    let mut expected_total = length;
    if code == 206 {
        let r = range
            .and_then(|v| v.strip_prefix("bytes "))
            .ok_or(Error("missing Content-Range", 0))?;
        let (span, total) = r.split_once('/').ok_or(Error("invalid Content-Range", 0))?;
        let total = total
            .parse::<u64>()
            .map_err(|_| Error("invalid Content-Range", 0))?;
        if total == 0 || total > limit {
            return Err(Error("download size limit", 0));
        }
        if original_total.is_none_or(|expected| expected != total) {
            return Err(Error("Content-Range total mismatch", 0));
        }
        let (first, last) = span
            .split_once('-')
            .ok_or(Error("invalid Content-Range", 0))?;
        let first = first
            .parse::<u64>()
            .map_err(|_| Error("invalid Content-Range", 0))?;
        let last = last
            .parse::<u64>()
            .map_err(|_| Error("invalid Content-Range", 0))?;
        if offset == 0
            || first != offset
            || last < first
            || last != total - 1
            || length.is_some_and(|n| n != total - first)
        {
            return Err(Error("invalid Content-Range", 0));
        }
        expected_total = Some(total);
    }
    if length.is_some_and(|n| n == 0 || n > limit - start) {
        return Err(Error("download size limit", 0));
    }
    // A full 200 response starts a new representation and must truncate, even
    // when this request sent Range. Never append a 200 body to the saved prefix.
    *original_total = expected_total;
    if track {
        crate::install::INSTALL_TOTAL_BYTES.store(expected_total.unwrap_or(0), Relaxed);
        crate::install::INSTALL_RX_BYTES.store(start, Relaxed);
    }
    let mut file = File::open_mode(path, if start > 0 { 2 } else { 1 })?;
    let mut total = start;
    if chunked {
        loop {
            let mut line = Text::<128>::new();
            while !line.bytes().ends_with(b"\r\n") {
                if receive(&sock, &mut file, &mut byte)? == 0 {
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
                        if receive(&sock, &mut file, &mut byte)? == 0 {
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
                break;
            }
            if remaining > limit - total {
                return Err(Error("download size limit", 0));
            }
            while remaining > 0 {
                let n = (remaining as usize).min(chunk.len());
                let got = receive(
                    &sock,
                    &mut file,
                    chunk.get_mut(..n).ok_or(Error("HTTP buffer bounds", 0))?,
                )?;
                if got == 0 {
                    return Err(Error("short HTTP chunk", 0));
                }
                if track {
                    crate::install::INSTALL_RX_BYTES.fetch_add(got as u64, Relaxed);
                }
                file.write(total, &chunk[..got])?;
                total += got as u64;
                remaining -= got as u64;
                if crate::retiring() {
                    return Err(Error("retired", 0));
                }
            }
            let mut crlf = [0; 2];
            for b in &mut crlf {
                if receive(&sock, &mut file, core::slice::from_mut(b))? != 1 {
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
                (n - (total - start)).min(chunk.len() as u64) as usize
            });
            if want == 0 {
                break;
            }
            let got = receive(&sock, &mut file, &mut chunk[..want])?;
            if got == 0 {
                break;
            }
            if got as u64 > limit - total {
                return Err(Error("download size limit", 0));
            }
            file.write(
                total,
                chunk.get(..got).ok_or(Error("HTTP buffer bounds", 0))?,
            )?;
            total += got as u64;
            if track {
                crate::install::INSTALL_RX_BYTES.store(total, Relaxed);
            }
            if crate::retiring() {
                return Err(Error("retired", 0));
            }
        }
        if length.is_some_and(|n| n != total - start) {
            return Err(Error("truncated HTTP body", 0));
        }
    }
    if expected_total.is_some_and(|n| n != total) {
        return Err(Error("truncated HTTP body", 0));
    }
    if total == 0 {
        return Err(Error("empty HTTP body", 0));
    }
    file.flush()?;
    Ok(total)
}

// Commit the saved prefix before returning a receive error to the retry loop.
fn receive(sock: &Socket, file: &mut File, bytes: &mut [u8]) -> Result<usize> {
    match sock.read(bytes) {
        Err(e) => {
            file.flush()?;
            Err(e)
        }
        ok => ok,
    }
}
