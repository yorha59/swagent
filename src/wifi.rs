//! Wire offsets: Switchbrew Network_Interface_services (oldid=15050).
use crate::{ffi, protocol, util::*};
use core::fmt::Write;
pub const PROFILE_SIZE: usize = 0x17c;
pub fn profile(ssid: &[u8], key: &[u8]) -> Result<[u8; PROFILE_SIZE]> {
    if ssid.is_empty() || ssid.len() > 32 || !(8..=63).contains(&key.len())
        || !key.iter().all(|b| (0x20..=0x7e).contains(b)) {
        return Err(Error("invalid WPA2 SSID/passphrase", 0));
    }
    let mut p = [0; PROFILE_SIZE];
    p[0] = 1; // IP IsAuto; all addresses zero.
    p[0xd] = 1; // DNS IsAuto.
    p[0xc0..0xc2].copy_from_slice(&1500u16.to_le_bytes());
    p[0xd2..0xd2 + ssid.len()].copy_from_slice(ssid);
    p[0x112] = 1; // User profile bitmask.
    p[0x113] = 1; // IEEE80211.
    p[0x114] = 1; // Auto-connect.
    p[0x116] = ssid.len() as u8;
    p[0x117..0x117 + ssid.len()].copy_from_slice(ssid);
    p[0x138] = 6; // WPA2 PSK (wire Wireless + 0x22).
    p[0x139] = 4; // AES (wire Wireless + 0x23).
    p[0x13a] = key.len() as u8;
    p[0x13b..0x13b + key.len()].copy_from_slice(key);
    Ok(p)
}
#[no_mangle]
pub extern "C" fn sa_wifi_log(step: u32, rc: u32) {
    let mut s = Text::<96>::new();
    let _ = write!(s, "wifi: step={} rc={}", step, rc);
    crate::log::add(s.str());
}
pub fn command(id: u64, args: &str) -> Result {
    let args = args.trim();
    let join = !args.is_empty();
    let mut ssid = "";
    let mut p = [0; PROFILE_SIZE];
    if join {
        // Quoted tokens support spaces; no escaping or credential logging.
        let mut rest = args;
        if token(&mut rest)? != "join" { return Err(Error("usage: wifi [join SSID PASSPHRASE]", 0)); }
        ssid = token(&mut rest)?;
        let key = token(&mut rest)?;
        if !rest.trim().is_empty() { return Err(Error("too many arguments", 0)); }
        p = profile(ssid.as_bytes(), key.as_bytes())?;
    }
    let mut records = [0u8; 0x75 * 256];
    let mut count = 0i32;
    let code = unsafe { ffi::sa_wifi(join as u32, p.as_ptr(), records.as_mut_ptr(), &mut count) };
    let mut line = Text::<192>::new();
    if code != 0 {
        let _ = write!(line, "wifi: {}error rc={}", if join { "join " } else { "" }, code);
        return protocol::reply(id, line.bytes());
    }
    if join {
        let _ = write!(line, "wifi: join ok {}", ssid);
        protocol::reply(id, line.bytes())
    } else {
        if !(0..=256).contains(&count) { return Err(Error("invalid profile count", 0)); }
        for (i, record) in records.chunks_exact(0x75).take(count as usize).enumerate() {
            if record[0x51] != 1 { continue; }
            let n = record[0x52] as usize;
            if n > 32 { return Err(Error("invalid SSID length", 0)); }
            line = Text::new();
            let _ = write!(line, "wifi: {} ", i);
            for &b in &record[0x53..0x53+n] {
                line.push(&[if b.is_ascii_control() || !b.is_ascii() { b'?' } else { b }])?;
            }
            protocol::reply(id, line.bytes())?;
        }
        protocol::reply(id, b"wifi: end")
    }
}
fn token<'a>(rest: &mut &'a str) -> Result<&'a str> {
    let s = rest.trim_start();
    if let Some(s) = s.strip_prefix('"') {
        let end = s.find('"').ok_or(Error("unterminated quote", 0))?;
        *rest = &s[end+1..];
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) { return Err(Error("invalid quoted argument", 0)); }
        Ok(&s[..end])
    } else {
        let end = s.find(char::is_whitespace).unwrap_or(s.len());
        if end == 0 { return Err(Error("missing argument", 0)); }
        *rest = &s[end..];
        Ok(&s[..end])
    }
}
