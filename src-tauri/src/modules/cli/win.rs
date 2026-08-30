//! Minimal kernel32 surface for the named-pipe server and the console attach.
//!
//! Declared by hand instead of enabling more `windows-sys` features: this
//! keeps the package's Cargo.toml untouched (the crate ships two `[[bin]]`
//! targets and the bundler once picked the wrong one after a manifest edit).
//! Swap for `windows-sys` `Win32_System_Pipes` + `Win32_System_Console` when
//! the manifest is next revised on purpose.

#![cfg(windows)]

use std::ffi::c_void;
use std::fs::File;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;

pub type Handle = *mut c_void;

/// A server-side pipe instance handle. Raw pointers are not `Send`, but a pipe
/// HANDLE is a plain kernel object id that may move to the connection thread.
pub struct PipeInstance(Handle);

unsafe impl Send for PipeInstance {}

const INVALID_HANDLE_VALUE: Handle = -1isize as Handle;

const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
const PIPE_TYPE_BYTE: u32 = 0;
const PIPE_READMODE_BYTE: u32 = 0;
const PIPE_WAIT: u32 = 0;
const PIPE_REJECT_REMOTE_CLIENTS: u32 = 0x0000_0008;
const PIPE_UNLIMITED_INSTANCES: u32 = 255;
const PIPE_BUFFER: u32 = 64 * 1024;

pub const ERROR_PIPE_BUSY: i32 = 231;
const ERROR_PIPE_CONNECTED: i32 = 535;

const STD_INPUT_HANDLE: u32 = 0xFFFF_FFF6;
const STD_OUTPUT_HANDLE: u32 = 0xFFFF_FFF5;
const STD_ERROR_HANDLE: u32 = 0xFFFF_FFF4;
const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;

#[link(name = "kernel32")]
extern "system" {
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        max_instances: u32,
        out_buffer_size: u32,
        in_buffer_size: u32,
        default_timeout: u32,
        security_attributes: *const c_void,
    ) -> Handle;
    fn ConnectNamedPipe(pipe: Handle, overlapped: *mut c_void) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
    fn AttachConsole(process_id: u32) -> i32;
    fn GetStdHandle(std_handle: u32) -> Handle;
    fn SetStdHandle(std_handle: u32, handle: Handle) -> i32;
}

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// One server-side pipe instance. `first` claims the name exclusively so a
/// squatter that pre-created it makes the server fail loudly instead of
/// silently serving nothing.
pub fn create_instance(name: &str, first: bool) -> std::io::Result<PipeInstance> {
    let wname = wide(name);
    let mut open_mode = PIPE_ACCESS_DUPLEX;
    if first {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let h = unsafe {
        CreateNamedPipeW(
            wname.as_ptr(),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUFFER,
            PIPE_BUFFER,
            0,
            std::ptr::null(),
        )
    };
    if h == INVALID_HANDLE_VALUE || h.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    Ok(PipeInstance(h))
}

/// Blocks until a client connects. On success the handle is owned by the
/// returned `File`, so std's Read/Write and Drop (CloseHandle) apply.
pub fn accept(instance: PipeInstance) -> std::io::Result<File> {
    let h = instance.0;
    let ok = unsafe { ConnectNamedPipe(h, std::ptr::null_mut()) } != 0;
    if !ok {
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(ERROR_PIPE_CONNECTED) {
            unsafe { CloseHandle(h) };
            return Err(err);
        }
    }
    Ok(unsafe { File::from_raw_handle(h as _) })
}

fn valid(h: Handle) -> bool {
    !h.is_null() && h != INVALID_HANDLE_VALUE
}

/// The release exe is a GUI-subsystem binary: launched from a console without
/// redirection it has no std handles at all, so help text and errors would
/// vanish. Attach to the parent's console in that case, then put back any
/// handle the parent DID redirect (a pipe from `koden ... | tee`) since
/// AttachConsole overwrites all three.
pub fn ensure_console() {
    unsafe {
        let out = GetStdHandle(STD_OUTPUT_HANDLE);
        let err = GetStdHandle(STD_ERROR_HANDLE);
        let inp = GetStdHandle(STD_INPUT_HANDLE);
        if valid(out) && valid(err) {
            return;
        }
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return;
        }
        if valid(out) {
            SetStdHandle(STD_OUTPUT_HANDLE, out);
        }
        if valid(err) {
            SetStdHandle(STD_ERROR_HANDLE, err);
        }
        if valid(inp) {
            SetStdHandle(STD_INPUT_HANDLE, inp);
        }
    }
}
