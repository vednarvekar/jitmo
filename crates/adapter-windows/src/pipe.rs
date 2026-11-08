//! Per-user named pipe for the Core service (blocking, byte mode).
//!
//! Not wired into the app yet. To use it, replace the in-process mpsc channel
//! in `crates/core/src/service.rs`:
//! - Core side: `PipeServer::bind(&default_pipe_name())`, then loop on
//!   `accept()`; for each stream, run the existing request handler with a
//!   reader doing `read_message::<_, UiRequest>(&mut BufReader)` and events
//!   sent with `write_message` (use a second handle for the writer half, or a
//!   mutex), instead of `Receiver<UiRequest>` / `Sender<CoreEvent>`.
//! - UI side (`app/src-tauri/src/main.rs`): `connect(&default_pipe_name())`
//!   instead of spawning the service thread with channels; a reader thread
//!   forwards `CoreEvent`s to the Tauri event emitter, and submit/confirm call
//!   `write_message`.
//!
//! The server is created with PIPE_REJECT_REMOTE_CLIENTS and the first
//! instance uses FILE_FLAG_FIRST_PIPE_INSTANCE so another process cannot
//! squat the name. The default DACL applies (current user, SYSTEM, admins).

use std::io::{self, Read, Write};

/// `\\.\pipe\jitmo-core-<user>`; characters outside `[A-Za-z0-9_-]` become `_`.
pub fn pipe_name_for_user(user: &str) -> String {
    let clean: String = user
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!(r"\\.\pipe\jitmo-core-{clean}")
}

/// Pipe name for the current Windows user (from `%USERNAME%`).
pub fn default_pipe_name() -> String {
    pipe_name_for_user(&std::env::var("USERNAME").unwrap_or_else(|_| "user".into()))
}

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const OPEN_EXISTING: u32 = 3;
const PIPE_ACCESS_DUPLEX: u32 = 3;
const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
// Byte type, byte read mode and blocking wait are all zero bits.
const PIPE_REJECT_REMOTE_CLIENTS: u32 = 8;
const PIPE_UNLIMITED_INSTANCES: u32 = 255;
const INVALID_HANDLE: isize = -1;
const ERROR_BROKEN_PIPE: i32 = 109;
const ERROR_PIPE_BUSY: i32 = 231;
const ERROR_PIPE_NOT_CONNECTED: i32 = 233;
const ERROR_PIPE_CONNECTED: i32 = 535;

#[link(name = "kernel32")]
extern "system" {
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        max_instances: u32,
        out_buf: u32,
        in_buf: u32,
        default_timeout: u32,
        security: *const core::ffi::c_void,
    ) -> isize;
    fn ConnectNamedPipe(pipe: isize, overlapped: *mut core::ffi::c_void) -> i32;
    fn DisconnectNamedPipe(pipe: isize) -> i32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *const core::ffi::c_void,
        disposition: u32,
        flags: u32,
        template: isize,
    ) -> isize;
    fn WaitNamedPipeW(name: *const u16, timeout_ms: u32) -> i32;
    fn ReadFile(
        h: isize,
        buf: *mut u8,
        len: u32,
        read: *mut u32,
        overlapped: *mut core::ffi::c_void,
    ) -> i32;
    fn WriteFile(
        h: isize,
        buf: *const u8,
        len: u32,
        written: *mut u32,
        overlapped: *mut core::ffi::c_void,
    ) -> i32;
    fn FlushFileBuffers(h: isize) -> i32;
    fn CloseHandle(h: isize) -> i32;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// One end of a connected pipe. Reads see EOF when the peer closes.
pub struct PipeStream {
    handle: isize,
    server: bool,
}

// SAFETY: a pipe HANDLE can be used from any thread; we only hand out `&mut self` access.
unsafe impl Send for PipeStream {}

impl Read for PipeStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut n = 0u32;
        let len = buf.len().min(u32::MAX as usize) as u32;
        // SAFETY: `buf` is valid for `len` bytes and `n` for one u32; the handle is open.
        let ok = unsafe {
            ReadFile(
                self.handle,
                buf.as_mut_ptr(),
                len,
                &mut n,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            let e = io::Error::last_os_error();
            // The peer closing shows up as "broken pipe", or as "not connected"
            // when the server side called DisconnectNamedPipe.
            if matches!(
                e.raw_os_error(),
                Some(ERROR_BROKEN_PIPE) | Some(ERROR_PIPE_NOT_CONNECTED)
            ) {
                return Ok(0);
            }
            return Err(e);
        }
        Ok(n as usize)
    }
}

impl Write for PipeStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut n = 0u32;
        let len = buf.len().min(u32::MAX as usize) as u32;
        // SAFETY: `buf` is valid for `len` bytes and `n` for one u32; the handle is open.
        let ok = unsafe { WriteFile(self.handle, buf.as_ptr(), len, &mut n, std::ptr::null_mut()) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(n as usize)
    }

    fn flush(&mut self) -> io::Result<()> {
        // WriteFile already hands data to the pipe buffer. Draining it (which
        // would block until the peer reads) is done once, in Drop.
        Ok(())
    }
}

impl Drop for PipeStream {
    fn drop(&mut self) {
        // SAFETY: the handle is owned by this value and closed exactly once.
        unsafe {
            if self.server {
                // Let the client finish reading before the connection is cut.
                FlushFileBuffers(self.handle);
                DisconnectNamedPipe(self.handle);
            }
            CloseHandle(self.handle);
        }
    }
}

fn create_instance(name: &[u16], first: bool) -> io::Result<isize> {
    let open_mode = PIPE_ACCESS_DUPLEX
        | if first {
            FILE_FLAG_FIRST_PIPE_INSTANCE
        } else {
            0
        };
    // SAFETY: `name` is NUL-terminated; a null security pointer means the default DACL.
    let h = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            open_mode,
            PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            64 * 1024,
            64 * 1024,
            0,
            std::ptr::null(),
        )
    };
    if h == INVALID_HANDLE {
        return Err(io::Error::last_os_error());
    }
    Ok(h)
}

/// Listening side. Always keeps one unconnected instance ready, so a client
/// that connects between two `accept` calls does not see "file not found".
pub struct PipeServer {
    name: Vec<u16>,
    pending: isize,
}

// SAFETY: see PipeStream.
unsafe impl Send for PipeServer {}

impl PipeServer {
    /// Create the pipe. Fails if another process already owns the name.
    pub fn bind(name: &str) -> io::Result<Self> {
        let name = wide(name);
        let pending = create_instance(&name, true)?;
        Ok(Self { name, pending })
    }

    /// Block until a client connects.
    pub fn accept(&mut self) -> io::Result<PipeStream> {
        let current = self.pending;
        // SAFETY: `current` is a valid, unconnected pipe instance we own.
        let ok = unsafe { ConnectNamedPipe(current, std::ptr::null_mut()) };
        if ok == 0 {
            let e = io::Error::last_os_error();
            // A client that connected before this call is reported as an error code but is fine.
            if e.raw_os_error() != Some(ERROR_PIPE_CONNECTED) {
                return Err(e);
            }
        }
        self.pending = create_instance(&self.name, false)?;
        Ok(PipeStream {
            handle: current,
            server: true,
        })
    }
}

impl Drop for PipeServer {
    fn drop(&mut self) {
        // SAFETY: `pending` is owned by this value and closed exactly once.
        unsafe { CloseHandle(self.pending) };
    }
}

/// Connect to a pipe created by `PipeServer`, waiting up to 5 seconds if all instances are busy.
pub fn connect(name: &str) -> io::Result<PipeStream> {
    let w = wide(name);
    for _ in 0..50 {
        // SAFETY: `w` is NUL-terminated and outlives the call.
        let h = unsafe {
            CreateFileW(
                w.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                0,
            )
        };
        if h != INVALID_HANDLE {
            return Ok(PipeStream {
                handle: h,
                server: false,
            });
        }
        let e = io::Error::last_os_error();
        if e.raw_os_error() != Some(ERROR_PIPE_BUSY) {
            return Err(e);
        }
        // SAFETY: as above.
        unsafe { WaitNamedPipeW(w.as_ptr(), 100) };
    }
    Err(io::Error::new(io::ErrorKind::TimedOut, "pipe stayed busy"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jitmo_ipc::{read_message, write_message, CoreEvent, UiRequest};
    use std::io::BufReader;

    #[test]
    fn name_is_sanitised() {
        assert_eq!(pipe_name_for_user("Ved N"), r"\\.\pipe\jitmo-core-Ved_N");
        assert_eq!(pipe_name_for_user("a\\b"), r"\\.\pipe\jitmo-core-a_b");
    }

    #[test]
    fn loopback_round_trip() {
        let name = format!(r"\\.\pipe\jitmo-test-{}", std::process::id());
        let mut server = PipeServer::bind(&name).unwrap();
        let t = std::thread::spawn(move || {
            let stream = server.accept().unwrap();
            let mut r = BufReader::new(stream);
            let req: UiRequest = read_message(&mut r).unwrap().unwrap();
            let UiRequest::Submit { id, .. } = req else {
                panic!("expected Submit");
            };
            let done = CoreEvent::Done {
                id,
                message: "ok".into(),
                undoable: false,
            };
            write_message(r.get_mut(), &done).unwrap();
            req
        });

        let mut r = BufReader::new(connect(&name).unwrap());
        let sent = UiRequest::Submit {
            id: 7,
            text: "what time is it".into(),
        };
        write_message(r.get_mut(), &sent).unwrap();
        let ev: CoreEvent = read_message(&mut r).unwrap().unwrap();
        assert_eq!(
            ev,
            CoreEvent::Done {
                id: 7,
                message: "ok".into(),
                undoable: false
            }
        );
        assert_eq!(t.join().unwrap(), sent);
        // Server dropped its end: the client sees a clean EOF.
        assert!(read_message::<_, CoreEvent>(&mut r).unwrap().is_none());
    }

    #[test]
    fn second_bind_with_same_name_fails() {
        let name = format!(r"\\.\pipe\jitmo-test-dup-{}", std::process::id());
        let _a = PipeServer::bind(&name).unwrap();
        assert!(PipeServer::bind(&name).is_err());
    }
}
