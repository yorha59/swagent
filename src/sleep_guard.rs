use crate::{ffi, io::File, util::*};
use core::fmt::Write;

const PATH: &str = "sdmc:/switch/.swagent_sleep_guard.bin";
const MAGIC: &[u8; 8] = b"SWASLP01";

fn settings<T>(f: impl FnOnce(ffi::SleepSettings) -> Result<T>) -> Result<T> {
    rc(
        unsafe {
            let _operation = crate::budget::Operation::new();
            ffi::setsysInitialize()
        },
        "initialize set:sys",
    )?;
    let _session = crate::protocol::Settings;
    let mut current = ffi::SleepSettings::default();
    rc(
        unsafe {
            let _operation = crate::budget::Operation::new();
            ffi::setsysGetSleepSettings(&mut current)
        },
        "read sleep settings",
    )?;
    f(current)
}
fn write(settings: &ffi::SleepSettings) -> Result {
    rc(
        unsafe {
            let _operation = crate::budget::Operation::new();
            ffi::setsysSetSleepSettings(settings)
        },
        "write sleep settings",
    )
}
fn saved() -> Result<ffi::SleepSettings> {
    let mut file = File::open(PATH, false)?;
    let mut bytes = [0; 20];
    if file.size()? != bytes.len() as u64 {
        return Err(Error("invalid sleep guard size", 0));
    }
    file.read(0, &mut bytes)?;
    if &bytes[..8] != MAGIC {
        return Err(Error("invalid sleep guard magic", 0));
    }
    let original = ffi::SleepSettings {
        flags: u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        handheld: i32::from_le_bytes(bytes[12..16].try_into().unwrap()),
        console: i32::from_le_bytes(bytes[16..20].try_into().unwrap()),
    };
    if !(0..=5).contains(&original.handheld) || !(0..=5).contains(&original.console) {
        return Err(Error("invalid sleep guard settings", 0));
    }
    Ok(original)
}
pub(crate) fn cancel() {
    let _ = crate::io::remove(PATH);
}
pub(crate) fn begin() -> Result {
    unsafe {
        let _operation = crate::budget::Operation::new();
        ffi::mkdir(c"sdmc:/switch".as_ptr(), 0o777);
    }
    // Never overwrite a recovery record left by an unsuccessful restore.
    if File::open(PATH, false).is_ok() {
        recover();
        if File::open(PATH, false).is_ok() {
            return Err(Error("pending sleep guard recovery", 0));
        }
    }
    settings(|original| {
        let mut bytes = [0; 20];
        bytes[..8].copy_from_slice(MAGIC);
        bytes[8..12].copy_from_slice(&original.flags.to_le_bytes());
        bytes[12..16].copy_from_slice(&original.handheld.to_le_bytes());
        bytes[16..20].copy_from_slice(&original.console.to_le_bytes());
        let mut file = File::open(PATH, true)?;
        file.write(0, &bytes)?;
        file.flush()?;
        drop(file);
        let mut never = original;
        never.handheld = 5;
        never.console = 5;
        write(&never)?;
        let mut line = Text::<96>::new();
        let _ = write!(
            line,
            "sleep: guard on (was hh={} dk={})",
            original.handheld, original.console
        );
        crate::log::add(line.str());
        Ok(())
    })
}
fn finish(recovery: bool) {
    if File::open(PATH, false).is_err() {
        return;
    }
    let original = match saved() {
        Ok(s) => s,
        Err(_) => {
            cancel();
            return;
        }
    };
    let result = settings(|current| {
        if !recovery || (current.handheld == 5 && current.console == 5) {
            write(&original)?;
            crate::log::add("sleep: guard restore");
        }
        Ok(())
    });
    if result.is_ok() {
        cancel();
    } else {
        crate::log::add("sleep: guard restore failed; retaining recovery file");
    }
}
pub(crate) fn restore() {
    finish(false);
}
pub(crate) fn recover() {
    finish(true);
}
