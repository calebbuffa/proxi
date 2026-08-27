//! Cross-platform process-tree lifecycle management for native builds.

use std::io;
use std::process::{Command, ExitStatus};
use std::sync::Once;
use std::sync::atomic::{AtomicU32, Ordering};

static INSTALL_HANDLER: Once = Once::new();
static ACTIVE_PID: AtomicU32 = AtomicU32::new(0);

/// Return whether a process ID currently identifies a live process.
pub fn is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }

    #[cfg(unix)]
    {
        // kill(pid, 0) performs existence/permission checking without sending
        // a signal. EPERM still means the process exists.
        let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
        result == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        };
        const STILL_ACTIVE: u32 = 259;

        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return false;
        }
        let mut exit_code = 0;
        let queried = unsafe { GetExitCodeProcess(handle, &mut exit_code) } != 0;
        unsafe { CloseHandle(handle) };
        queried && exit_code == STILL_ACTIVE
    }
}

/// Run a native build command and ensure Ctrl+C terminates its descendants.
pub fn run(command: &mut Command, description: &str) -> io::Result<ExitStatus> {
    install_interrupt_handler();

    #[cfg(unix)]
    configure_unix_process_group(command);
    #[cfg(windows)]
    configure_windows_process_group(command);

    eprintln!("PROXI: starting {description}");
    let mut child = command.spawn()?;
    ACTIVE_PID.store(child.id(), Ordering::SeqCst);
    let result = child.wait();
    ACTIVE_PID.store(0, Ordering::SeqCst);
    result
}

fn install_interrupt_handler() {
    INSTALL_HANDLER.call_once(|| {
        ctrlc::set_handler(|| {
            let pid = ACTIVE_PID.load(Ordering::SeqCst);
            if pid == 0 {
                return;
            }

            #[cfg(unix)]
            {
                // The child is the process-group leader. A negative PID sends
                // SIGTERM to CMake and every descendant in that group.
                unsafe {
                    libc::kill(-(pid as libc::pid_t), libc::SIGTERM);
                }
            }

            #[cfg(windows)]
            {
                // taskkill /T reaches CMake's ExternalProject/compiler tree;
                // /F is necessary because build tools may ignore Ctrl+C.
                let _ = std::process::Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/T", "/F"])
                    .status();
            }
        })
        .expect("PROXI: failed to install Ctrl+C handler");
    });
}

#[cfg(unix)]
fn configure_unix_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(windows)]
fn configure_windows_process_group(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP);
}
