use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::Mutex,
};
use util::Error;
static SERIAL: Mutex<()> = Mutex::new(());
extern "C" {
    fn host_reset(fail: u32);
    fn host_stat(n: u32) -> u32;
    fn host_launch_result(rc: u32);
}
fn server(body: Vec<u8>, chunked: bool, truncate: bool) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut b = [0];
            socket.read_exact(&mut b).unwrap();
            request.push(b[0]);
        }
        if chunked {
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n"
            )
            .unwrap();
            for part in body.chunks(117) {
                write!(socket, "{:x};foo=bar\r\n", part.len()).unwrap();
                socket.write_all(part).unwrap();
                socket.write_all(b"\r\n").unwrap();
            }
            socket
                .write_all(if truncate {
                    b"0\r\n"
                } else {
                    b"0\r\nX-Test: yes\r\n\r\n"
                })
                .unwrap();
        } else {
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                body.len() + truncate as usize
            )
            .unwrap();
            socket.write_all(&body).unwrap();
        }
    });
    (format!("http://127.0.0.1:{port}/test.nsp"), handle)
}
fn setup() {
    std::fs::create_dir_all("build/host-sd/switch").unwrap();
}
#[test]
fn url_validation() {
    let u = http::parse("http://example.com:8000/a?b=c").unwrap();
    assert_eq!(u.host.str(), "example.com");
    assert_eq!(u.port.str(), "8000");
    assert_eq!(u.path.str(), "/a?b=c");
    for u in [
        "https://host/a",
        "http://",
        "http://h:0/a",
        "http://h:99999/a",
        "http://user@host/a",
        "http://host/\r\nInjected: x",
    ] {
        assert!(http::parse(u).is_err(), "{u}");
    }
}
#[test]
fn framing_fragmentation_overflow_recovery() {
    let mut f = protocol::Frame::new();
    let mut lines = Vec::new();
    for b in b"ping\r\nversion\r\n" {
        if let Some(l) = f.feed(*b) {
            lines.push(l.unwrap().str().to_string())
        }
    }
    assert_eq!(lines, ["ping", "version"]);
    for _ in 0..600 {
        assert!(f.feed(b'x').is_none())
    }
    assert!(f.feed(b'\r').is_none());
    assert!(f.feed(b'\n').unwrap().is_err());
    for b in b"ping\r" {
        assert!(f.feed(*b).is_none())
    }
    assert_eq!(f.feed(b'\n').unwrap().unwrap().str(), "ping");
    for b in b"p\nng\r" {
        f.feed(*b);
    }
    assert!(f.feed(b'\n').unwrap().is_err());
}
#[test]
fn bounded_queue_fifo() {
    protocol::test_queue();
}
#[test]
fn streaming_http_and_truncation() {
    let _guard = SERIAL.lock().unwrap();
    setup();
    let path = "sdmc:/switch/download";
    for chunked in [false, true] {
        for truncated in [false, true] {
            let body: Vec<u8> = (0..9999).map(|i| (i % 251) as u8).collect();
            let (url, server) = server(body.clone(), chunked, truncated);
            let mut scratch = [0; 313];
            let result = http::download(&url, path, &mut scratch, 10000);
            server.join().unwrap();
            assert_eq!(result.is_err(), truncated);
            if !truncated {
                assert_eq!(result.unwrap(), 9999);
                assert_eq!(
                    std::fs::read("build/host-sd/switch/download").unwrap(),
                    body
                );
            }
        }
    }
}
fn cnmt() -> Vec<u8> {
    let mut c = vec![0; 32 + 16 + 56 + 32];
    c[..8].copy_from_slice(&0x0100123400000000u64.to_le_bytes());
    c[12] = 0x80;
    c[14] = 16;
    c[16] = 1;
    c[48 + 32..48 + 48].fill(0x22);
    let n = 4 * 1024 * 1024 + 123;
    c[48 + 48..48 + 52].copy_from_slice(&(n as u32).to_le_bytes());
    c[48 + 54] = 1;
    c
}
fn package() -> Vec<u8> {
    let mut tik = vec![0; 0x350];
    tik[..4].copy_from_slice(&0x10004u32.to_be_bytes());
    tik[0x286] = 0x10;
    let files = vec![
        ("11111111111111111111111111111111.cnmt.nca", cnmt()),
        (
            "22222222222222222222222222222222.nca",
            vec![7; 4 * 1024 * 1024 + 123],
        ),
        ("license.tik", tik),
        ("license.cert", vec![1; 512]),
    ];
    let mut strings = Vec::new();
    let mut table = Vec::new();
    let mut data = Vec::new();
    for (name, body) in files {
        table.extend_from_slice(&(data.len() as u64).to_le_bytes());
        table.extend_from_slice(&(body.len() as u64).to_le_bytes());
        table.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        table.extend_from_slice(&[0; 4]);
        strings.extend_from_slice(name.as_bytes());
        strings.push(0);
        data.extend_from_slice(&body);
    }
    let mut out = b"PFS0".to_vec();
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(&(strings.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend(table);
    out.extend(strings);
    out.extend(data);
    out
}
#[test]
fn installer_streams_single_chunk_and_cleans_failure() {
    let _guard = SERIAL.lock().unwrap();
    setup();
    for fail in [0, 0x1234] {
        unsafe { host_reset(fail) };
        let (url, server) = server(package(), false, false);
        let result = install::install(&url, 5);
        server.join().unwrap();
        if fail == 0 {
            assert_eq!(result.unwrap(), 0x0100123400000000);
            assert_eq!(unsafe { host_stat(0) }, 4 * 1024 * 1024);
            assert_eq!(unsafe { host_stat(1) }, 2);
            assert_eq!(unsafe { host_stat(2) }, 2);
            assert_eq!(unsafe { host_stat(3) }, 1);
            assert_ne!(unsafe { host_stat(4) } & 1, 0);
        } else {
            assert_eq!(result.unwrap_err(), Error("write placeholder", fail));
            assert_eq!(unsafe { host_stat(2) }, 1);
        }
        assert!(!std::path::Path::new("build/host-sd/switch/.install.tmp.nsp").exists());
        assert!(!std::path::Path::new("build/host-sd/switch/.install.tmp.cnmt.nca").exists());
    }
}
#[test]
fn malformed_cnmt_never_panics() {
    let c = cnmt();
    assert!(install::meta(&c).is_ok());
    for n in 0..c.len() {
        assert!(install::meta(&c[..n]).is_err(), "length {n}");
    }
    let mut bad = c.clone();
    bad[12] = 1;
    assert!(install::meta(&bad).is_err());
    bad = c.clone();
    bad[16..18].copy_from_slice(&1025u16.to_le_bytes());
    assert!(install::meta(&bad).is_err());
}
#[test]
fn malformed_pfs0_bounds() {
    let good = package();
    let count = 4;
    let base = 16 + 24 * count + util::le32(&good[8..]) as usize;
    let header = &good[..base];
    assert!(install::entry(header, count, base as u64, 0).is_ok());
    for n in 0..16 + 24 * count {
        assert!(install::entry(&header[..n], count, base as u64, 0).is_err());
    }
    let mut bad = header.to_vec();
    bad[32..36].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(install::entry(&bad, count, base as u64, 0).is_err());
    bad = header.to_vec();
    bad[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(install::entry(&bad, count, base as u64, 0).is_err());
}
#[test]
fn ring_wrap_and_concurrent_reads() {
    let _guard = SERIAL.lock().unwrap();
    for _ in 0..1100 {
        log::add("ring test")
    }
    let mut n = 0;
    log::dump(1000, |_| n += 1);
    assert_eq!(n, 1000);
    let writer = std::thread::spawn(|| {
        for _ in 0..3000 {
            log::add("concurrent");
        }
    });
    for _ in 0..20 {
        log::dump(1000, |s| assert!(s == b"ring test" || s == b"concurrent"));
    }
    writer.join().unwrap();
}

#[test]
fn update_validates_image_and_restores_failed_launch() {
    let _guard = SERIAL.lock().unwrap();
    setup();
    let root = "build/host-sd/atmosphere/contents/430000000000000B";
    std::fs::create_dir_all(root).unwrap();
    let dest = format!("{root}/exefs.nsp");
    let artifact = std::fs::read("build/swagent.nsp").unwrap();
    for launch in [0x5678, 0] {
        std::fs::write(&dest, b"previous image").unwrap();
        unsafe { host_launch_result(launch) };
        let (url, server) = server(artifact.clone(), false, false);
        let result = protocol::apply_update(&url);
        server.join().unwrap();
        if launch != 0 {
            assert_eq!(result.unwrap_err(), Error("launch replacement", launch));
            assert_eq!(std::fs::read(&dest).unwrap(), b"previous image");
        } else {
            assert_eq!(result.unwrap(), 123);
            assert_eq!(std::fs::read(&dest).unwrap(), artifact);
            assert_eq!(
                std::fs::read(format!("{root}/exefs.nsp.bak")).unwrap(),
                b"previous image"
            );
        }
        assert!(!std::path::Path::new(&format!("{root}/exefs.nsp.tmp")).exists());
    }
    let mut wrong = artifact.clone();
    let count = util::le32(&wrong[4..]) as usize;
    let base = 16 + 24 * count + util::le32(&wrong[8..]) as usize;
    for i in 0..count {
        let e = install::entry(&wrong[..base], count, base as u64, i).unwrap();
        if e.name == "main.npdm" {
            let npdm = e.off as usize;
            let aci = util::le32(&wrong[npdm + 0x70..]) as usize;
            wrong[npdm + aci + 0x10] = 0xff;
            break;
        }
    }
    std::fs::write(&dest, b"untouched").unwrap();
    let (url, server) = server(wrong, false, false);
    let result = protocol::apply_update(&url);
    server.join().unwrap();
    assert!(result.is_err());
    assert_eq!(std::fs::read(dest).unwrap(), b"untouched");
}

#[test]
fn ping_version_and_busy_while_install_worker_is_blocked() {
    use std::{
        io::{BufRead, BufReader},
        os::unix::{io::AsRawFd, net::UnixStream},
        sync::mpsc,
    };
    let _guard = SERIAL.lock().unwrap();
    setup();
    unsafe { host_reset(0) };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let http = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut req = Vec::new();
        while !req.ends_with(b"\r\n\r\n") {
            let mut b = [0];
            socket.read_exact(&mut b).unwrap();
            req.push(b[0]);
        }
        started_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        let body = package();
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
            body.len()
        )
        .unwrap();
        socket.write_all(&body).unwrap();
    });
    let (server, client) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .unwrap();
    protocol::test_session(server.as_raw_fd());
    protocol::test_receive(&format!("install http://127.0.0.1:{port}/file"));
    let worker = std::thread::spawn(protocol::test_run_one);
    started_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    let mut client = BufReader::new(client);
    let mut line = String::new();
    for (command, expected) in [("ping", "pong\r\n"), ("version", "swagent-1.1-rs\r\n")] {
        protocol::test_receive(command);
        client.read_line(&mut line).unwrap();
        assert_eq!(line, expected);
        line.clear();
    }
    for _ in 0..9 {
        protocol::test_receive("log 1")
    }
    client.read_line(&mut line).unwrap();
    assert_eq!(line, "error: busy\r\n");
    line.clear();
    protocol::test_receive("ping");
    client.read_line(&mut line).unwrap();
    assert_eq!(line, "pong\r\n");
    line.clear();
    release_tx.send(()).unwrap();
    http.join().unwrap();
    worker.join().unwrap();
    client.read_line(&mut line).unwrap();
    assert!(line.starts_with("install: ok "), "{line}");
    protocol::test_session(-1);
}

#[test]
fn removed_commands_are_unknown() {
    use std::{
        io::{BufRead, BufReader},
        os::unix::{io::AsRawFd, net::UnixStream},
    };
    let _guard = SERIAL.lock().unwrap();
    let (server, client) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .unwrap();
    protocol::test_session(server.as_raw_fd());
    let mut client = BufReader::new(client);
    for command in [
        "shot",
        "key A",
        "key ZR 100",
        "touch 100 200",
        "touch 0 0 100",
        "unknown",
    ] {
        protocol::test_receive(command);
        protocol::test_run_one();
        let mut line = String::new();
        client.read_line(&mut line).unwrap();
        assert_eq!(line, "error: unknown command\r\n", "{command}");
    }
    protocol::test_session(-1);
}
