use crate::{ffi, util::*};
use core::fmt::Write;

// Ground truth: CTCaer/hekate bdk/power/max17050.{c,h}.
// Wire words are little-endian. VCELL discards three unused low bits.
pub(crate) fn voltage_mv(bytes: [u8; 2]) -> u32 {
    (u16::from_le_bytes(bytes) as u32 >> 3) * 625 / 1000
}
pub(crate) fn current_ma(bytes: [u8; 2]) -> f64 {
    // Physical 5mOhm * board CGAIN 2 = effective 10mOhm.
    i16::from_le_bytes(bytes) as f64 * 0.15625
}
pub(crate) fn read() -> Result<Text<256>> {
    let (mut percent, mut charger) = (0, 0);
    rc(unsafe { ffi::sa_power_psm(&mut percent, &mut charger) }, "read psm")?;
    let mut s = Text::new();
    let (mut voltage, mut current) = ([0; 2], [0; 2]);
    let result = unsafe { ffi::sa_power_gauge(voltage.as_mut_ptr(), current.as_mut_ptr()) };
    if result == 0 {
        let mv = voltage_mv(voltage);
        let ma = current_ma(current);
        // mV * mA = microwatts. Keep fractional current for power calculation.
        let _ = write!(s, "power: vbus? no — battery {}mV {:.2}mA {:.2}W soc={}%",
                       mv, ma, mv as f64 * ma / 1_000_000.0, percent);
    } else {
        let _ = write!(s, "power: soc={}%", percent);
    }
    // PSM reports power capability, not adapter manufacturer. 'official' is
    // the protocol label for EnoughPower; other connected categories are usb.
    match charger {
        0 => { let _ = s.push(b" charger=none"); }
        1 => { let _ = s.push(b" charger=official"); }
        2 | 3 => { let _ = s.push(b" charger=usb"); }
        _ => {} // Failed/unknown charger query: never invent a category.
    }
    if result != 0 {
        let _ = write!(s, " (battery telemetry unavailable: i2c rc={})", result);
    }
    Ok(s)
}
