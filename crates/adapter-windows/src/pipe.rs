//! Per-user named pipe for the Core service (blocking, byte mode).
//!
//! Wired into the app: Core side is `PipeServer::bind` + `accept` feeding
//! `jitmo_core::service::serve_connection` (in `app/src-tauri/src/main.rs`);
//! UI side is `app/src-tauri/src/core_client.rs` (`connect`, reader thread,
//! writes from the Tauri commands). Handles are opened overlapped so
//! [`PipeStream::split`] halves can read and write at the same time.
//!
//! The server is created with PIPE_REJECT_REMOTE_CLIENTS and the first
//! instance uses FILE_FLAG_FIRST_PIPE_INSTANCE so another process cannot
//! squat the name. The default DACL applies (current user, SYSTEM, admins).

use std::io::{self, Read, Write};
use std::sync::Arc;

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
// Overlapped handles let one thread read while another writes. A synchronous
// handle serialises all calls, so a blocked read would stall every write.
const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
const ERROR_IO_PENDING: i32 = 997;
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
    fn CreateEventW(
        security: *const core::ffi::c_void,
        manual_reset: i32,
        initial: i32,
        name: *const u16,
    ) -> isize;
    fn GetOverlappedResult(h: isize, overlapped: *mut Overlapped, read: *mut u32, wait: i32)
        -> i32;
    fn FlushFileBuffers(h: isize) -> i32;
    fn CloseHandle(h: isize) -> i32;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[repr(C)]
struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u64,
    event: isize,
}

/// Start an overlapped call with `start` and wait for it to finish.
fn overlapped_call(handle: isize, start: impl FnOnce(*mut Overlapped) -> i32) -> io::Result<u32> {
    // SAFETY: plain FFI; the event is closed on every path below.
    let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) };
    if event == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut ov = Overlapped {
        internal: 0,
        internal_high: 0,
        offset: 0,
        event,
    };
    let result = if start(&mut ov) == 0
        && io::Error::last_os_error().raw_os_error() != Some(ERROR_IO_PENDING)
    {
        Err(io::Error::last_os_error())
    } else {
        let mut n = 0u32;
        // SAFETY: `ov` is the structure the call above was started with and is still alive.
        if unsafe { GetOverlappedResult(handle, &mut ov, &mut n, 1) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(n)
        }
    };
    // SAFETY: the event is owned here.
    unsafe { CloseHandle(event) };
    result
}

/// An open pipe handle shared by the reader and writer halves.
struct Inner {
    handle: isize,
    server: bool,
}

impl Inner {
    fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        let len = buf.len().min(u32::MAX as usize) as u32;
        let ptr = buf.as_mut_ptr();
        // SAFETY: `buf` is valid for `len` bytes until the call completes (we wait for it).
        let r = overlapped_call(self.handle, |ov| unsafe {
            ReadFile(self.handle, ptr, len, std::ptr::null_mut(), ov.cast())
        });
        match r {
            Ok(n) => Ok(n as usize),
            // The peer closing shows up as "broken pipe", or as "not connected"
            // when the server side called DisconnectNamedPipe.
            Err(e)
                if matches!(
                    e.raw_os_error(),
                    Some(ERROR_BROKEN_PIPE) | Some(ERROR_PIPE_NOT_CONNECTED)
                ) =>
            {
                Ok(0)
            }
            Err(e) => Err(e),
        }
    }

    fn write(&self, buf: &[u8]) -> io::Result<usize> {
        let len = buf.len().min(u32::MAX as usize) as u32;
        let ptr = buf.as_ptr();
        // SAFETY: `buf` is valid for `len` bytes until the call completes (we wait for it).
        overlapped_call(self.handle, |ov| unsafe {
            WriteFile(self.handle, ptr, len, std::ptr::null_mut(), ov.cast())
        })
        .map(|n| n as usize)
    }
}

impl Drop for Inner {
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

/// One end of a connected pipe. Reads see EOF when the peer closes.
pub struct PipeStream {
    inner: Arc<Inner>,
}

/// Read half of a split [`PipeStream`].
pub struct PipeReader {
    inner: Arc<Inner>,
}

/// Write half of a split [`PipeStream`].
pub struct PipeWriter {
    inner: Arc<Inner>,
}

impl PipeStream {
    /// Split into halves that can be used from different threads at once
    /// (one blocked in a read while the other writes). The connection closes
    /// when both halves are dropped.
    pub fn split(self) -> (PipeReader, PipeWriter) {
        (
            PipeReader {
                inner: self.inner.clone(),
            },
            PipeWriter { inner: self.inner },
        )
    }
}

impl Read for PipeStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Write for PipeStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        // WriteFile already hands data to the pipe buffer. Draining it (which
        // would block until the peer reads) is done once, in Drop.
        Ok(())
    }
}

impl Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Write for PipeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn create_instance(name: &[u16], first: bool) -> io::Result<isize> {
    let open_mode = PIPE_ACCESS_DUPLEX
        | FILE_FLAG_OVERLAPPED
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
        let r = overlapped_call(current, |ov| unsafe {
            ConnectNamedPipe(current, ov.cast())
        });
        match r {
            Ok(_) => {}
            // A client that connected before this call is reported as an error code but is fine.
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_CONNECTED) => {}
            Err(e) => return Err(e),
        }
        self.pending = create_instance(&self.name, false)?;
        Ok(PipeStream {
            inner: Arc::new(Inner {
                handle: current,
                server: true,
            }),
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
                FILE_FLAG_OVERLAPPED,
                0,
            )
        };
        if h != INVALID_HANDLE {
            return Ok(PipeStream {
                inner: Arc::new(Inner {
                    handle: h,
                    server: false,
                }),
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
