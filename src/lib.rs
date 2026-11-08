#![cfg_attr(not(test), no_std)]
mod installed;
mod budget;
mod ffi;
mod files;
mod http;
mod install;
mod io;
mod log;
mod md5;
mod net;
mod power;
mod protocol;
mod sleep_guard;
mod util;
mod wifi;
pub use protocol::retiring;
#[cfg(not(test))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // No unwinding through libnx, process teardown, fatalThrow or SD logging.
    log::add("panic: thread parked");
    util::forever()
}
#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn agent_main() -> ! {
    log::add(log::boot_line(util::now_ms()).str());
    if unsafe { ffi::sa_is_nro() } == 0 { util::sleep(5000); }
    loop {
        let code = unsafe { ffi::sa_init() };
        if code == 0 {
            break;
        }
        log::add("service initialization failed; retrying");
        util::sleep(5000);
    }
    sleep_guard::recover();
    log::add(version());
    loop {
        let code = unsafe { ffi::sa_spawn(0, protocol::worker, core::ptr::null_mut()) };
        if code == 0 {
            break;
        }
        log::add("worker creation failed; retrying");
        util::sleep(1000);
    }
    for slot in 0..4 {
        loop {
            let code = unsafe {
                ffi::sa_spawn(
                    slot + 1,
                    protocol::client_worker,
                    slot as usize as *mut core::ffi::c_void,
                )
            };
            if code == 0 {
                break;
            }
            log::add("client worker creation failed; retrying");
            util::sleep(250);
        }
    }
    loop {
        let code = unsafe { ffi::sa_spawn(5, protocol::watchdog, core::ptr::null_mut()) };
        if code == 0 {
            break;
        }
        log::add("watchdog creation failed; retrying");
        util::sleep(250);
    }
    protocol::serve()
}
#[cfg(test)]
mod tests;

#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn agent_park() -> ! {
    log::add("C runtime fatal fallback intercepted; thread parked");
    util::forever()
}

pub fn version() -> &'static str {
    unsafe { core::ffi::CStr::from_ptr(ffi::sa_version()).to_str().unwrap_or("unknown") }
}
pub fn is_nro() -> bool { unsafe { ffi::sa_is_nro() != 0 } }
