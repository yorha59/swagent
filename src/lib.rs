#![cfg_attr(not(test), no_std)]
mod ffi;
mod http;
mod install;
mod io;
mod log;
mod protocol;
mod util;
pub use protocol::retiring;
#[cfg(not(test))]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // No unwinding through libnx, process teardown, fatalThrow or SD logging.
    log::add("internal invariant failed; thread parked");
    util::forever()
}
#[cfg(not(test))]
#[no_mangle]
pub extern "C" fn agent_main() -> ! {
    util::sleep(5000);
    loop {
        let code = unsafe { ffi::sa_init() };
        if code == 0 {
            break;
        }
        log::add("service initialization failed; retrying");
        util::sleep(5000);
    }
    log::add("swagent-1.3-rs");
    loop {
        let code = unsafe { ffi::sa_spawn(0, protocol::worker, core::ptr::null_mut()) };
        if code == 0 {
            break;
        }
        log::add("worker creation failed; retrying");
        util::sleep(1000);
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
