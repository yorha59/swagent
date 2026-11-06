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
    fn host_process_reset(fail: u32);
    fn host_process_stat(n: u32) -> u32;
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
fn process_commands_queue_self_guard_and_errors() {
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
    for (fail, command, expected, terminated, exits) in [
        (
            0,
            "ps",
            "ps: 123 0100123400000000\r\nps: 456 430000000000000b\r\nps: end\r\n",
            0,
            1,
        ),
        (0, "kill 123", "kill: ok 123\r\n", 123, 0),
        (0, "kill 456", "kill: refusing self\r\n", 0, 0),
        (1, "kill 123", "kill: error get own pid rc=77\r\n", 0, 0),
        (
            2,
            "kill 123",
            "kill: error terminate process rc=88\r\n",
            123,
            0,
        ),
        (3, "ps", "ps: error list processes rc=99\r\n", 0, 0),
        (4, "ps", "ps: error initialize pm:info rc=100\r\n", 0, 0),
        (5, "ps", "ps: 123 ?\r\nps: 456 ?\r\nps: end\r\n", 0, 1),
        (
            7,
            "ps",
            "ps: 123 ?\r\nps: 456 430000000000000b\r\nps: end\r\n",
            0,
            1,
        ),
        (
            0,
            "df",
            "df: sd total=64000000000 free=512500000 total=64.00 GB free=512.50 MB\r\ndf: end\r\n",
            0,
            0,
        ),
        (8, "df", "df: error get SD space rc=102\r\n", 0, 0),
        (0, "df extra", "df: error usage: df rc=0\r\n", 0, 0),
        (
            6,
            "ps",
            "ps: error process list exceeds capacity rc=0\r\n",
            0,
            0,
        ),
        (0, "kill", "kill: error usage: kill PID rc=0\r\n", 0, 0),
        (0, "kill nope", "kill: error invalid pid rc=0\r\n", 0, 0),
        (0, "kill -1", "kill: error invalid pid rc=0\r\n", 0, 0),
        (
            0,
            "kill 18446744073709551616",
            "kill: error invalid pid rc=0\r\n",
            0,
            0,
        ),
        (
            0,
            "kill 123 extra",
            "kill: error usage: kill PID rc=0\r\n",
            0,
            0,
        ),
        (0, "ps extra", "ps: error usage: ps rc=0\r\n", 0, 0),
    ] {
        unsafe { host_process_reset(fail) };
        protocol::test_receive(command);
        // Immediate ping must precede the queued command's output.
        protocol::test_receive("ping");
        let mut line = String::new();
        client.read_line(&mut line).unwrap();
        assert_eq!(line, "pong\r\n", "{command}");
        assert_eq!(unsafe { host_process_stat(0) }, 0);
        protocol::test_run_one();
        let mut actual = String::new();
        for _ in expected.lines() {
            client.read_line(&mut actual).unwrap();
        }
        assert_eq!(actual, expected, "{command}");
        assert_eq!(unsafe { host_process_stat(0) }, terminated);
        assert_eq!(unsafe { host_process_stat(1) }, exits);
    }
    unsafe { host_process_reset(0) };
    protocol::test_session(-1);
}
#[test]
fn launch_and_quit_commands() {
    use std::{
        io::{BufRead, BufReader},
        os::unix::{io::AsRawFd, net::UnixStream},
    };
    unsafe extern "C" {
        fn host_launched_title() -> u64;
        fn host_launch_calls() -> u32;
    }
    let _guard = SERIAL.lock().unwrap();
    let (server, client) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .unwrap();
    protocol::test_session(server.as_raw_fd());
    let mut client = BufReader::new(client);
    for (fail, command, expected, terminated, exits) in [
        (
            0,
            "launch 0100123400000000",
            "launch: ok 0100123400000000",
            0,
            0,
        ),
        (
            0,
            "launch 0100ABCDEF000000",
            "launch: ok 0100abcdef000000",
            0,
            0,
        ),
        (0, "quit 0100123400000000", "quit: ok 123", 123, 1),
        (0, "quit 0000000000000000", "quit: not running", 0, 1),
        (0, "quit 430000000000000B", "quit: refusing self", 0, 1),
        (1, "quit 0100123400000000", "quit: error rc=77", 0, 1),
        (2, "quit 0100123400000000", "quit: error rc=88", 123, 1),
        (3, "quit 0100123400000000", "quit: error rc=99", 0, 0),
        (4, "quit 0100123400000000", "quit: error rc=100", 0, 0),
        (5, "quit 0100123400000000", "quit: not running", 0, 1),
        (6, "quit 0100123400000000", "quit: error rc=0", 0, 0),
        (7, "quit 0100123400000000", "quit: not running", 0, 1),
    ] {
        unsafe {
            host_process_reset(fail);
            host_launch_result(0);
        }
        protocol::test_receive(command);
        assert_eq!(unsafe { host_launch_calls() }, 0);
        protocol::test_run_one();
        let mut actual = String::new();
        client.read_line(&mut actual).unwrap();
        assert_eq!(actual, format!("{expected}\r\n"), "{command}");
        assert_eq!(unsafe { host_process_stat(0) }, terminated);
        assert_eq!(unsafe { host_process_stat(1) }, exits);
        if command.starts_with("launch") {
            assert_eq!(unsafe { host_launch_calls() }, 1);
            assert_eq!(
                unsafe { host_launched_title() },
                u64::from_str_radix(command.split_whitespace().nth(1).unwrap(), 16).unwrap()
            );
        }
    }
    // Failed AM requests must return the original result without retrying PM.
    for result in [1234, 0xAD800A08, 0xAF400A08, 0xD601] {
        unsafe { host_launch_result(result) };
        protocol::test_receive("launch 0100123400000000");
        protocol::test_run_one();
        let mut actual = String::new();
        client.read_line(&mut actual).unwrap();
        assert_eq!(actual, format!("launch: error rc={result}\r\n"));
        assert_eq!(unsafe { host_launch_calls() }, 1);
    }
    let mut attempt = false;
    log::dump(1000, |s| {
        attempt |= s == b"launch: attempt 0100123400000000"
    });
    assert!(attempt);
    for cmd in ["launch", "quit"] {
        for arg in [
            "",
            "1",
            "01001234000000000",
            "0x00123400000000",
            "g100123400000000",
            "+100123400000000",
            "0100123400000000 extra",
        ] {
            unsafe {
                host_launch_result(0);
                host_process_reset(0);
            }
            protocol::test_receive(&format!("{cmd} {arg}"));
            protocol::test_run_one();
            let mut actual = String::new();
            client.read_line(&mut actual).unwrap();
            assert_eq!(
                actual,
                format!("{cmd}: error expected 16-hex-digit title ID rc=0\r\n")
            );
            assert_eq!(unsafe { host_launch_calls() }, 0);
            assert_eq!(unsafe { host_process_stat(0) }, 0);
            assert_eq!(unsafe { host_process_stat(1) }, 0);
        }
    }
    unsafe {
        host_launch_result(0);
        host_process_reset(0);
    }
    protocol::test_session(-1);
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
    tik[..4].copy_from_slice(&0x10004u32.to_le_bytes());
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
        let size = package().len() as u64;
        let (url, server) = server(package(), false, false);
        let result = install::install(&url, 5);
        assert_eq!(
            install::progress(),
            (if fail == 0 { 3 } else { 4 }, size, size)
        );
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
        assert!(!std::path::Path::new("build/host-sd/switch/.host-stage.nca").exists());
        assert!(!std::path::Path::new("build/host-sd/switch/.install.tmp.nsp").exists());
        assert!(!std::path::Path::new("build/host-sd/switch/.install.tmp.cnmt.nca").exists());
    }
}
#[test]
fn installer_continues_without_importing_invalid_or_failed_tickets() {
    let _guard = SERIAL.lock().unwrap();
    setup();
    for (sig, warning) in [
        (
            0xb28fab5au32,
            "install: skipping invalid ticket (sig=0xb28fab5a)",
        ),
        (
            0x10006,
            "install: ticket import failed (rc=0x000003e7); continuing",
        ),
    ] {
        unsafe { host_reset(0) };
        let mut image = package();
        let count = util::le32(&image[4..]).unwrap() as usize;
        let base = 16 + 24 * count + util::le32(&image[8..]).unwrap() as usize;
        let ticket = install::entry(&image[..base], count, base as u64, 2).unwrap();
        let off = ticket.off as usize;
        image[off..off + 4].copy_from_slice(&sig.to_le_bytes());
        if sig == 0xb28fab5a {
            // A placeholder must not require a matching certificate.
            let pos = image
                .windows(12)
                .position(|s| s == b"license.cert")
                .unwrap();
            image[pos] = b'x';
        }
        let (url, server) = server(image, false, false);
        let result = install::install(&url, 5);
        server.join().unwrap();
        assert_eq!(result.unwrap(), 0x0100123400000000);
        assert_eq!(unsafe { host_stat(1) }, 2);
        assert_eq!(unsafe { host_stat(3) }, 0);
        let mut found = false;
        log::dump(1000, |line| found |= line == warning.as_bytes());
        assert!(found, "missing warning: {warning}");
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
    let base = 16 + 24 * count + util::le32(&good[8..]).unwrap() as usize;
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
    let count = util::le32(&wrong[4..]).unwrap() as usize;
    let base = 16 + 24 * count + util::le32(&wrong[8..]).unwrap() as usize;
    for i in 0..count {
        let e = install::entry(&wrong[..base], count, base as u64, i).unwrap();
        if e.name == "main.npdm" {
            let npdm = e.off as usize;
            let aci = util::le32(&wrong[npdm + 0x70..]).unwrap() as usize;
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
fn ping_version_status_and_busy_while_install_worker_is_blocked() {
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
    for (command, expected) in [
        ("ping", "pong\r\n"),
        ("version", "swagent-1.12-rs\r\n"),
        ("status", "status: downloading rx=0 total=0\r\n"),
    ] {
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
    protocol::test_receive("status");
    client.read_line(&mut line).unwrap();
    assert_eq!(line, "status: downloading rx=0 total=0\r\n");
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

// Optional local fixture: read only the header and file prefixes, never load 5 GB.
#[test]
fn known_good_gta_update_pfs0_from_disk() {
    use std::io::{Seek, SeekFrom};
    let path = "/tmp/nsp_share/GTA_VC_Update.nsp";
    let mut file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("fixture unavailable: {path}");
            return;
        }
        Err(e) => panic!("{path}: {e}"),
    };
    let size = file.metadata().unwrap().len();
    let mut source = io::File::open(path, false).unwrap();
    let parsed = install::Pfs::open(&mut source).unwrap();
    assert_eq!(parsed.count, 8);
    parsed.validate(size).unwrap();
    let mut fixed = [0; 16];
    file.read_exact(&mut fixed).unwrap();
    assert_eq!(&fixed[..4], b"PFS0");
    let count = util::le32(&fixed[4..]).unwrap() as usize;
    assert_eq!(count, 8);
    let base = 16 + count * 24 + util::le32(&fixed[8..]).unwrap() as usize;
    let mut header = vec![0; base];
    file.seek(SeekFrom::Start(0)).unwrap();
    file.read_exact(&mut header).unwrap();
    let mut ticket_found = false;
    let mut meta_found = false;
    for i in 0..count {
        let e = install::entry(&header, count, base as u64, i).unwrap();
        let actual = parsed.entry(i).unwrap();
        assert_eq!(
            (actual.name, actual.off, actual.size),
            (e.name, e.off, e.size)
        );
        assert!(e.off >= base as u64 && e.off + e.size <= size);
        println!("{} size={} absolute_offset={}", e.name, e.size, e.off);
        if e.name.ends_with(".tik") {
            ticket_found = true;
            let mut sig = [0; 4];
            file.seek(SeekFrom::Start(e.off)).unwrap();
            file.read_exact(&mut sig).unwrap();
            assert_eq!(e.size, 704);
            assert_eq!(util::le32(&sig).unwrap(), 0x10004);
            println!("ticket prefix: {sig:02x?}");
        }
        meta_found |= e.name.ends_with(".cnmt.nca");
    }
    assert!(ticket_found && meta_found);
}

#[test]
fn installer_from_disk_stages_and_cleans_success_and_failure() {
    let _guard = SERIAL.lock().unwrap();
    setup();
    for fail in [0, 0x1234] {
        unsafe { host_reset(fail) };
        std::fs::write("build/host-sd/switch/.install.tmp.nsp", package()).unwrap();
        let mut chunk = vec![0; 4 * 1024 * 1024];
        let result = install::install_downloaded(5, &mut chunk);
        if fail == 0 {
            assert_eq!(result.unwrap(), 0x0100123400000000);
            assert_eq!(unsafe { host_stat(1) }, 2);
            assert_eq!(unsafe { host_stat(3) }, 1);
        } else {
            assert_eq!(result.unwrap_err(), Error("write placeholder", fail));
        }
        for name in [
            ".install.tmp.nsp",
            ".install.tmp.cnmt.nca",
            ".host-stage.nca",
        ] {
            assert!(!std::path::Path::new(&format!("build/host-sd/switch/{name}")).exists());
        }
    }
}

#[test]
fn sleep_network_logic_and_checked_bounds() {
    let _guard = SERIAL.lock().unwrap();
    for rc in [4, 11, 35, 116] {
        assert!(crate::net::transient(rc));
        crate::net::failed(rc);
        assert!(!crate::net::pending());
    }
    crate::net::failed(9);
    assert!(crate::net::pending());
    let old = crate::net::epoch();
    assert_eq!(crate::net::reinit(), 0);
    assert!(!crate::net::pending());
    assert_ne!(old, crate::net::epoch());
    let mut logged = false;
    log::dump(1000, |s| logged |= s == b"net: reinit rc=0");
    assert!(logged);
    for n in 0..8 {
        let p = vec![0; n];
        assert_eq!(util::le16(&p).is_ok(), n >= 2);
        assert_eq!(util::le32(&p).is_ok(), n >= 4);
        assert!(util::le64(&p).is_err());
    }
    assert!(install::entry(&[], usize::MAX, 0, 0).is_err());
    let mut text = util::Text::<4>::new();
    text.len = usize::MAX;
    assert!(text.push(b"a").is_err());
    assert_eq!(text.bytes(), b"");
}

#[test]
fn install_eagain_retries_and_preserves_download() {
    let _guard = SERIAL.lock().unwrap();
    setup();
    unsafe extern "C" {
        fn host_receive_fixture(p: *const core::ffi::c_char, error: i32);
        fn host_fixture_connects() -> i32;
    }
    for response in [
        c"HTTP/1.1",
        c"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial",
        c"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n64\r\npartial",
    ] {
        let _ = std::fs::remove_file("build/host-sd/switch/.install.tmp.nsp");
        unsafe {
            host_receive_fixture(response.as_ptr(), 11);
        }
        let result = install::install("http://fixture/test", 5);
        assert_eq!(unsafe { host_fixture_connects() }, 30);
        unsafe {
            host_receive_fixture(core::ptr::null(), 0);
        }
        assert_eq!(result, Err(Error("network receive", 11)));
        assert!(!std::path::Path::new("build/host-sd/switch/.swagent_sleep_guard.bin").exists());
        assert!(!crate::net::pending());
        protocol::test_session(-1);
        protocol::test_install_error(result.unwrap_err());
        let mut logged = false;
        log::dump(1000, |s| {
            logged |= s == b"install: error network receive rc=11"
        });
        assert!(logged);
        assert!(std::path::Path::new("build/host-sd/switch/.install.tmp.nsp").exists());
        assert!(!std::path::Path::new("build/host-sd/switch/.install.tmp.cnmt.nca").exists());
    }
}

#[test]
fn bsd_reset_waits_for_calls_and_rejects_stale_descriptors() {
    use std::{
        os::unix::{io::AsRawFd, net::UnixStream},
        sync::mpsc,
        time::Duration,
    };
    let _guard = SERIAL.lock().unwrap();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let call = std::thread::spawn(move || {
        crate::net::GATE.with(|_| {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        })
    });
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    // A blocked download must not serialize a ping/send behind it.
    crate::net::GATE.with(|_| ());
    let (mut server, _client) = UnixStream::pair().unwrap();
    let stale = io::Socket::new(server.as_raw_fd());
    let (done_tx, done_rx) = mpsc::channel();
    let reset = std::thread::spawn(move || {
        done_tx.send(crate::net::reinit()).unwrap();
    });
    assert!(done_rx.recv_timeout(Duration::from_millis(30)).is_err());
    release_tx.send(()).unwrap();
    call.join().unwrap();
    assert_eq!(done_rx.recv_timeout(Duration::from_secs(2)).unwrap(), 0);
    reset.join().unwrap();
    assert_eq!(stale.read(&mut [0]), Err(Error("network reset", 0)));
    drop(stale);
    server.write_all(b"still open").unwrap();
}

#[test]
fn connect_failure_requests_recovery_and_init_failure_retries() {
    let _guard = SERIAL.lock().unwrap();
    unsafe extern "C" {
        fn host_connect_failure(error: i32);
        fn host_reinit_failures(n: u32);
    }
    for code in [11, 9] {
        unsafe {
            host_connect_failure(code);
        }
        let result = io::Socket::connect(c"fixture".as_ptr(), c"80".as_ptr());
        unsafe {
            host_connect_failure(0);
        }
        assert!(matches!(result, Err(Error("HTTP connect", e)) if e == code as u32));
        assert_eq!(crate::net::pending(), code == 9);
    }
    unsafe {
        host_reinit_failures(1);
    }
    crate::net::recover();
    assert!(crate::net::ready());
    assert!(!crate::net::pending());
    let mut failed = false;
    let mut recovered = false;
    log::dump(1000, |s| {
        failed |= s == b"net: reinit rc=123";
        recovered |= s == b"net: reinit rc=0";
    });
    assert!(failed && recovered);
}

#[test]
fn status_formatting() {
    for (phase, name) in [
        (0, "idle"),
        (1, "downloading"),
        (2, "writing"),
        (3, "done"),
        (4, "error"),
    ] {
        assert_eq!(
            protocol::format_status(phase, 0, 0).str(),
            format!("status: {name} rx=0 total=0")
        );
    }
    assert_eq!(
        protocol::format_status(1, 1, 3).str(),
        "status: downloading rx=1 total=3 pct=33.3"
    );
    assert_eq!(
        protocol::format_status(3, 42, 42).str(),
        "status: done rx=42 total=42 pct=100.0"
    );
    assert!(protocol::format_status(2, u64::MAX, u64::MAX)
        .str()
        .ends_with("pct=100.0"));
    assert!(protocol::format_status(4, u64::MAX, 1)
        .str()
        .ends_with("pct=1844674407370955161500.0"));
}

#[test]
fn install_http_range_resume_restart_and_complete() {
    let _guard = SERIAL.lock().unwrap();
    setup();
    let path = "sdmc:/switch/range";
    for code in [206, 200, 416] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://127.0.0.1:{}/range",
            listener.local_addr().unwrap().port()
        );
        let server = std::thread::spawn(move || {
            for attempt in 0..2 {
                let (mut socket, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut b = [0];
                    socket.read_exact(&mut b).unwrap();
                    request.push(b[0]);
                }
                let request = String::from_utf8(request).unwrap();
                if attempt == 0 {
                    assert!(!request.contains("Range:"));
                    socket
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nabc")
                        .unwrap();
                } else {
                    assert!(request.contains("Range: bytes=3-\r\n"));
                    let response = match code {
                        206 => "HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 3-5/6\r\n\r\ndef",
                        200 => "HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nuvwxyz",
                        _ => "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nContent-Range: bytes */3\r\n\r\n",
                    };
                    socket.write_all(response.as_bytes()).unwrap();
                }
            }
        });
        let mut scratch = [0; 16];
        let size = http::download_install(&url, path, &mut scratch, 100).unwrap();
        server.join().unwrap();
        let expected: &[u8] = match code {
            206 => b"abcdef",
            200 => b"uvwxyz",
            _ => b"abc",
        };
        assert_eq!(
            std::fs::read("build/host-sd/switch/range").unwrap(),
            expected
        );
        assert_eq!(size, expected.len() as u64);
        assert_eq!(install::progress().1, size);
        assert_eq!(install::progress().2, size);
    }
}

#[test]
fn install_range_fixtures_across_commands() {
    let _guard = SERIAL.lock().unwrap();
    setup();
    unsafe extern "C" {
        fn host_receive_fixture(p: *const core::ffi::c_char, error: i32);
        fn host_fixture_request() -> *const core::ffi::c_char;
    }
    let path = "sdmc:/switch/range-fixture";
    let url = "http://fixture/range-fixture";
    let mut scratch = [0; 16];
    for code in [206, 200, 416, 404] {
        // A different URL must discard old bytes, even before the GET succeeds.
        std::fs::write("build/host-sd/switch/range-fixture", b"stale").unwrap();
        unsafe {
            host_receive_fixture(
                c"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nabc".as_ptr(),
                11,
            );
        }
        assert!(http::download_install(&format!("{url}/{code}"), path, &mut scratch, 100).is_err());
        assert_eq!(
            std::fs::read("build/host-sd/switch/range-fixture").unwrap(),
            b"abc"
        );
        let response = match code {
            206 => c"HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nContent-Range: bytes 3-5/6\r\n\r\ndef",
            200 => c"HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nuvwxyz",
            416 => c"HTTP/1.1 416 Range Not Satisfiable\r\nContent-Length: 0\r\nContent-Range: bytes */3\r\n\r\n",
            _ => c"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n",
        };
        unsafe {
            host_receive_fixture(response.as_ptr(), 11);
        }
        let result = http::download_install(&format!("{url}/{code}"), path, &mut scratch, 100);
        let request = unsafe { std::ffi::CStr::from_ptr(host_fixture_request()) }
            .to_str()
            .unwrap();
        assert!(request.contains("Range: bytes=3-\r\n"));
        unsafe {
            host_receive_fixture(core::ptr::null(), 0);
        }
        if code == 404 {
            assert_eq!(result, Err(Error("HTTP status", 404)));
            continue;
        }
        let expected: &[u8] = match code {
            206 => b"abcdef",
            200 => b"uvwxyz",
            _ => b"abc",
        };
        assert_eq!(result.unwrap(), expected.len() as u64);
        assert_eq!(
            std::fs::read("build/host-sd/switch/range-fixture").unwrap(),
            expected
        );
        assert_eq!(install::progress().1, expected.len() as u64);
        assert_eq!(install::progress().2, expected.len() as u64);
    }
}

#[test]
fn invalid_download_is_removed_before_resume() {
    let _guard = SERIAL.lock().unwrap();
    setup();
    std::fs::write("build/host-sd/switch/.install.tmp.nsp", b"invalid package").unwrap();
    let mut chunk = [0; 32];
    assert!(install::install_downloaded(5, &mut chunk).is_err());
    assert!(!std::path::Path::new("build/host-sd/switch/.install.tmp.nsp").exists());
}

#[test]
fn system_settings_commands_and_readback() {
    use std::{
        io::{BufRead, BufReader},
        os::unix::{io::AsRawFd, net::UnixStream},
    };
    unsafe extern "C" {
        fn host_settings_fail(fail: u32);
    }
    let _guard = SERIAL.lock().unwrap();
    let (server, client) = UnixStream::pair().unwrap();
    client
        .set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .unwrap();
    protocol::test_session(server.as_raw_fd());
    let mut client = BufReader::new(client);
    for (fail, command, expected) in [
        (0, "sleep off", "sleep: ok off"),
        (0, "set sleep", "set: sleep off handheld=5 console=5"),
        (0, "sleep off", "sleep: ok off"),
        (0, "sleep on", "sleep: ok on"),
        (0, "set sleep", "set: sleep on handheld=2 console=3"),
        (0, "sleep", "sleep: error usage: sleep <on|off> rc=0"),
        (0, "sleep ON", "sleep: error usage: sleep <on|off> rc=0"),
        (
            0,
            "sleep off extra",
            "sleep: error usage: sleep <on|off> rc=0",
        ),
        (0, "set", "set: error usage: set KEY rc=0"),
        (0, "set battery 10", "set: error usage: set KEY rc=0"),
        (0, "set arbitrary", "set: error unknown setting key rc=0"),
        (0, "set firmware", "set: firmware 23.0.0"),
        (0, "set hostname", "set: hostname Switch  name"),
        (0, "set serial", "set: serial XAW000000"),
        (0, "set battery", "set: battery 87%"),
        (0, "set temperature", "set: temperature 42 C"),
        (1, "sleep off", "sleep: error initialize set:sys rc=91"),
        (2, "sleep off", "sleep: error read sleep settings rc=92"),
        (3, "sleep off", "sleep: error write sleep settings rc=93"),
        (4, "sleep on", "sleep: ok mixed"),
        (0, "sleep on", "sleep: ok on"),
    ] {
        unsafe {
            host_settings_fail(fail);
        }
        protocol::test_receive(command);
        protocol::test_run_one();
        let mut line = String::new();
        client.read_line(&mut line).unwrap();
        assert_eq!(line, format!("{expected}\r\n"), "{command}");
    }
    unsafe {
        host_settings_fail(0);
    }
    protocol::test_session(-1);
    for (handheld, console, expected) in [
        (5, 5, "off"),
        (0, 4, "on"),
        (5, 0, "mixed"),
        (3, 5, "mixed"),
    ] {
        assert_eq!(
            protocol::format_sleep(&ffi::SleepSettings {
                flags: 99,
                handheld,
                console
            }),
            expected
        );
    }
}

#[test]
fn sleep_guard_save_restore_and_crash_recovery() {
    unsafe extern "C" {
        fn host_settings_fail(fail: u32);
    }
    let _serial = SERIAL.lock().unwrap();
    setup();
    unsafe {
        host_settings_fail(0);
    }
    let path = "build/host-sd/switch/.swagent_sleep_guard.bin";
    sleep_guard::cancel();
    let original = ffi::SleepSettings {
        flags: 0xa5,
        handheld: 2,
        console: 3,
    };
    let read = || {
        let mut s = ffi::SleepSettings::default();
        assert_eq!(unsafe { ffi::setsysGetSleepSettings(&mut s) }, 0);
        (s.flags, s.handheld, s.console)
    };
    assert_eq!(unsafe { ffi::setsysSetSleepSettings(&original) }, 0);
    sleep_guard::begin().unwrap();
    assert_eq!(read(), (0xa5, 5, 5));
    let record = std::fs::read(path).unwrap();
    assert_eq!(&record[..8], b"SWASLP01");
    sleep_guard::restore();
    assert_eq!(read(), (0xa5, 2, 3));
    assert!(!std::path::Path::new(path).exists());
    sleep_guard::begin().unwrap();
    sleep_guard::recover(); // Simulates a fresh module: all state comes from SD.
    assert_eq!(read(), (0xa5, 2, 3));
    assert!(!std::path::Path::new(path).exists());
    sleep_guard::begin().unwrap();
    assert_eq!(unsafe { ffi::setsysSetSleepSettings(&original) }, 0);
    sleep_guard::recover(); // User changed settings after the crash.
    assert_eq!(read(), (0xa5, 2, 3));
    assert!(!std::path::Path::new(path).exists());
    sleep_guard::begin().unwrap();
    sleep_guard::cancel(); // Successful manual sleep off cancels restoration.
    sleep_guard::restore();
    assert_eq!(read(), (0xa5, 5, 5));
    assert_eq!(unsafe { ffi::setsysSetSleepSettings(&original) }, 0);
    for bad in [b"bad".as_slice(), &[0; 20]] {
        std::fs::write(path, bad).unwrap();
        sleep_guard::recover();
        assert_eq!(read(), (0xa5, 2, 3));
        assert!(!std::path::Path::new(path).exists());
    }
    sleep_guard::begin().unwrap();
    unsafe {
        host_settings_fail(3);
    }
    sleep_guard::restore();
    assert!(std::path::Path::new(path).exists());
    unsafe {
        host_settings_fail(0);
    }
    sleep_guard::recover();
    assert_eq!(read(), (0xa5, 2, 3));
    assert!(!std::path::Path::new(path).exists());
    unsafe {
        host_settings_fail(2);
    }
    assert!(sleep_guard::begin().is_err());
    assert!(!std::path::Path::new(path).exists());
    unsafe {
        host_settings_fail(0);
    }
}

#[test]
fn max17050_decode_fixtures() {
    // Derived vectors from hekate bdk/power/max17050.c constants:
    // (raw >> 3)*625/1000 mV; signed raw * 156.25 uA.
    for (raw, expected) in [(0u16, 0), (7, 0), (8, 0), (16, 1),
                            (0xb900, 3700), (0xc800, 4000), (0xffff, 5119)] {
        assert_eq!(crate::power::voltage_mv(raw.to_le_bytes()), expected);
    }
    for (raw, expected) in [(0u16, 0.0), (1, 0.15625), (0xffff, -0.15625),
                            (0x1900, 1000.0), (0xe700, -1000.0),
                            (0x8000, -5120.0), (0x7fff, 5119.84375)] {
        assert_eq!(crate::power::current_ma(raw.to_le_bytes()), expected);
    }
}

#[test]
fn power_queued_command_and_fallback() {
    use std::{io::{BufRead, BufReader}, os::unix::{io::AsRawFd, net::UnixStream}};
    unsafe extern "C" { fn host_power_fixture(fail: u32, charger: u32); }
    let _guard = SERIAL.lock().unwrap();
    let (server, client) = UnixStream::pair().unwrap();
    client.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
    protocol::test_session(server.as_raw_fd());
    let mut client = BufReader::new(client);
    for (fail, charger, command, expected) in [
        (0, 0, "power", "power: vbus? no — battery 3700mV -1000.00mA -3.70W soc=87% charger=none"),
        (0, 1, "power", "power: vbus? no — battery 3700mV -1000.00mA -3.70W soc=87% charger=official"),
        (0, 2, "power", "power: vbus? no — battery 3700mV -1000.00mA -3.70W soc=87% charger=usb"),
        (1, 3, "power", "power: soc=87% charger=usb (battery telemetry unavailable: i2c rc=91)"),
        (2, 0, "power", "power: error read psm rc=92"),
        (0, 0, "power extra", "power: error usage: power rc=0"),
    ] {
        unsafe { host_power_fixture(fail, charger); }
        protocol::test_receive(command);
        // ping passes the queued power command before the worker runs.
        protocol::test_receive("ping");
        let mut line = String::new();
        client.read_line(&mut line).unwrap();
        assert_eq!(line, "pong\r\n");
        protocol::test_run_one();
        line.clear();
        client.read_line(&mut line).unwrap();
        assert_eq!(line, format!("{expected}\r\n"));
    }
    protocol::test_session(-1);
}
